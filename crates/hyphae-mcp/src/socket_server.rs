//! Unix-socket endpoint for direct JSON-RPC 2.0 tool calls.
//!
//! Cortina and other hook-time callers use this endpoint to invoke
//! `hyphae_memory_store` without spawning a new process. This avoids the
//! circular-dependency risk of calling back through Claude Code's MCP channel
//! at hook time.
//!
//! # Protocol
//!
//! Each connection sends exactly one newline-delimited JSON-RPC 2.0 request and
//! reads one response. The method name is the tool name directly (e.g.
//! `hyphae_memory_store`), and params are the tool arguments (the same shape as
//! the `arguments` field in MCP `tools/call` payloads). The connection is kept
//! open until EOF to allow multiple requests per connection.
//!
//! # Endpoint registration
//!
//! On startup, `run_socket_server` writes an endpoint descriptor to
//! `~/.config/hyphae/hyphae.endpoint.json` so clients can discover the socket
//! path via `spore::paths::config_dir("hyphae")`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use hyphae_core::ConsolidationConfig;
use hyphae_store::SqliteStore;
use serde_json::{Value, json};
use tracing::{debug, error};

#[cfg(unix)]
use crate::cap_methods;
use crate::protocol::{JsonRpcMessage, JsonRpcResponse};
use crate::tools;

const CAPABILITY_ID: &str = "memory.store.v1";
const PING_METHOD: &str = "PING";

/// Write the `local-service-endpoint-v1` descriptor to the hyphae config dir.
fn write_endpoint_descriptor(socket_path: &Path, events_url: Option<&str>) -> anyhow::Result<()> {
    let config_dir = spore::paths::config_dir("hyphae");
    std::fs::create_dir_all(&config_dir)?;
    let descriptor_path = config_dir.join("hyphae.endpoint.json");
    let mut descriptor = json!({
        "schema_version": "1.0",
        "transport": "unix-socket",
        "endpoint": socket_path.to_string_lossy(),
        "capability_id": CAPABILITY_ID,
        "version": env!("CARGO_PKG_VERSION"),
        "health_probe": {
            "method": PING_METHOD,
            "timeout_ms": 1000
        }
    });
    if let Some(url) = events_url {
        descriptor["events_url"] = serde_json::Value::String(url.to_string());
    }
    std::fs::write(&descriptor_path, serde_json::to_string_pretty(&descriptor)?)?;
    Ok(())
}

fn remove_stale_socket(socket_path: &Path) {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
}

/// Guard that cleans up the PID file on drop.
struct PidFileGuard {
    pid_path: PathBuf,
}

impl PidFileGuard {
    fn new(pid_path: PathBuf) -> Self {
        PidFileGuard { pid_path }
    }
}

impl Drop for PidFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.pid_path);
    }
}

/// Check if a PID corresponds to a running process.
/// On Unix, kill(pid, 0) returns 0 if the process exists.
#[cfg(unix)]
#[allow(unsafe_code)]
fn is_process_alive(pid: i32) -> bool {
    // SAFETY: kill(pid, 0) is a non-destructive signal check
    // that only tests process existence, no side effects.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_alive(_pid: i32) -> bool {
    // Non-Unix platforms: assume process may be alive
    true
}

fn write_response(
    writer: &mut (impl Write + ?Sized),
    resp: &JsonRpcResponse,
) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *writer, resp)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Handle one accepted unix-socket client connection.
///
/// Reads newline-delimited JSON-RPC 2.0 requests until EOF. Each request is
/// dispatched through the shared tool handler and a response written back. One
/// connection can carry multiple request/response pairs.
fn handle_connection(
    stream: std::os::unix::net::UnixStream,
    store: Arc<Mutex<SqliteStore>>,
    consolidation: Arc<ConsolidationConfig>,
    compact: bool,
    reject_secrets: bool,
) {
    let writer_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            error!("failed to clone unix stream: {e}");
            return;
        }
    };
    let mut reader = BufReader::new(stream);
    let mut writer = writer_stream;

    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return, // EOF — client closed
            Ok(_) => {}
            Err(e) => {
                error!("socket read error: {e}");
                return;
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let msg: JsonRpcMessage = match serde_json::from_str(trimmed) {
            Ok(m) => m,
            Err(e) => {
                let resp = JsonRpcResponse::err(Value::Null, -32700, format!("parse error: {e}"));
                let _ = write_response(&mut writer, &resp);
                return;
            }
        };

        // Notifications (no id) — no response, keep reading
        let id = match msg.id {
            Some(id) => id,
            None => continue,
        };

        let method = msg.method.as_deref().unwrap_or("");
        debug!("socket request: {method}");

        let response = if method == PING_METHOD || method == "ping" {
            JsonRpcResponse::ok(id, json!({}))
        } else if method.starts_with("cap_") {
            let args = msg.params.unwrap_or_else(|| json!({}));
            let store_guard = match store.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    tracing::error!("store mutex was poisoned — recovering");
                    poisoned.into_inner()
                }
            };
            let result = cap_methods::dispatch_cap_method(&store_guard, method, &args);
            drop(store_guard);
            JsonRpcResponse::ok(id, result)
        } else {
            let args = msg.params.unwrap_or_else(|| json!({}));
            let store_guard = match store.lock() {
                Ok(g) => g,
                Err(poisoned) => {
                    tracing::error!("store mutex was poisoned — recovering");
                    poisoned.into_inner()
                }
            };
            let result = tools::call_tool_with_consolidation(
                &store_guard,
                None, // no embedder in socket mode; store works without it
                &consolidation,
                method,
                &args,
                compact,
                None, // no project context in fire-and-forget hook calls
                reject_secrets,
                &tools::ToolTraceContext::default(),
            );
            drop(store_guard);
            let result_val = match serde_json::to_value(result) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(error = %e, "failed to serialize tool result");
                    // Return a JSON-RPC error response using write_response to ensure
                    // the client receives a newline and flush (required for BufReader::read_line)
                    let err_response =
                        JsonRpcResponse::err(id, -32603, format!("Internal error: {e}"));
                    let _ = write_response(&mut writer, &err_response);
                    return;
                }
            };
            if result_val
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let msg = result_val
                    .get("content")
                    .and_then(|c| c.get(0))
                    .and_then(|c| c.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("tool error");
                JsonRpcResponse::err(id, -32000, msg.to_string())
            } else {
                JsonRpcResponse::ok(id, result_val)
            }
        };

        if let Err(e) = write_response(&mut writer, &response) {
            error!("socket write error: {e}");
            return;
        }
    }
}

/// Start the hyphae unix-socket service endpoint.
///
/// Binds to `~/.local/share/basidiocarp/hyphae/hyphae.sock`, writes the
/// endpoint descriptor to `~/.config/hyphae/hyphae.endpoint.json`, then
/// accepts connections indefinitely. Each connection is handled in a
/// background thread.
///
/// # Errors
///
/// Returns an error if the socket path cannot be created or bound.
pub fn run_socket_server(
    store: SqliteStore,
    consolidation: ConsolidationConfig,
    compact: bool,
    reject_secrets: bool,
) -> anyhow::Result<()> {
    let socket_path: PathBuf = spore::paths::data_dir("basidiocarp")
        .join("hyphae")
        .join("hyphae.sock");

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Check for existing singleton process before removing socket
    let pid_path = socket_path.with_extension("pid");
    if pid_path.exists() {
        if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                let pid_i32 = pid as i32;
                if is_process_alive(pid_i32) {
                    tracing::error!(
                        pid = pid_i32,
                        "hyphae socket server is already running (PID {}) — exiting", pid_i32
                    );
                    return Err(anyhow::anyhow!(
                        "hyphae socket server already running as PID {}",
                        pid_i32
                    ));
                }
            }
        }
        // Stale PID file — clean it up
        let _ = std::fs::remove_file(&pid_path);
    }

    // Now safe to remove stale socket and bind
    remove_stale_socket(&socket_path);

    let listener = std::os::unix::net::UnixListener::bind(&socket_path).map_err(|e| {
        anyhow::anyhow!(
            "failed to bind hyphae socket {}: {e}",
            socket_path.display()
        )
    })?;

    // Write PID file and create guard to clean it up on exit
    let current_pid = std::process::id();
    std::fs::write(&pid_path, format!("{}\n", current_pid))?;
    let _pid_guard = PidFileGuard::new(pid_path);

    // Initialize event bus and start SSE server.
    crate::memoir_events::init_bus();
    let events_url = match crate::memoir_events::start_events_server(0) {
        Ok(addr) => {
            tracing::info!(addr = %addr, "memoir SSE server ready");
            Some(format!("http://{addr}/memoir-events"))
        }
        Err(e) => {
            tracing::warn!("memoir SSE server failed to start: {e}");
            None
        }
    };

    write_endpoint_descriptor(&socket_path, events_url.as_deref())?;

    tracing::info!(
        socket = %socket_path.display(),
        pid = current_pid,
        "hyphae socket endpoint ready"
    );

    let store = Arc::new(Mutex::new(store));
    let consolidation = Arc::new(consolidation);

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let store = Arc::clone(&store);
                let consolidation = Arc::clone(&consolidation);
                std::thread::spawn(move || {
                    handle_connection(stream, store, consolidation, compact, reject_secrets);
                });
            }
            Err(e) => {
                error!("hyphae socket accept error: {e}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_store::SqliteStore;
    use std::io::{BufRead, BufReader, Write};
    use tempfile::TempDir;

    fn temp_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn temp_socket_path(dir: &TempDir) -> PathBuf {
        dir.path().join("test.sock")
    }

    #[test]
    fn write_endpoint_descriptor_creates_json_file() {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");
        let old_config = std::env::var("XDG_CONFIG_HOME").ok();
        // Point XDG_CONFIG_HOME to tmp so we don't pollute the real config dir.
        // SAFETY: test-only environment variable modification.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        }

        write_endpoint_descriptor(&socket_path, None).expect("descriptor should write");

        let descriptor_path = spore::paths::config_dir("hyphae").join("hyphae.endpoint.json");
        assert!(descriptor_path.exists(), "descriptor file should exist");

        let content = std::fs::read_to_string(&descriptor_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["schema_version"], "1.0");
        assert_eq!(v["transport"], "unix-socket");
        assert_eq!(v["capability_id"], CAPABILITY_ID);
        assert!(v["endpoint"].as_str().unwrap().contains("test.sock"));

        // SAFETY: test-only cleanup.
        #[allow(unsafe_code)]
        unsafe {
            if let Some(old) = old_config {
                std::env::set_var("XDG_CONFIG_HOME", old);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
        }
    }

    #[test]
    fn socket_server_ping_responds_ok() {
        let tmp = TempDir::new().unwrap();
        let socket_path = temp_socket_path(&tmp);
        let socket_path_clone = socket_path.clone();

        // Bind the server socket
        remove_stale_socket(&socket_path);
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();

        // Spawn a server thread that handles one connection
        let handle = std::thread::spawn(move || {
            let store = Arc::new(Mutex::new(temp_store()));
            let consolidation = Arc::new(ConsolidationConfig::default());
            if let Ok(stream) = listener.accept().map(|(s, _)| s) {
                handle_connection(stream, store, consolidation, false, false);
            }
        });

        // Connect as client and send a PING
        let mut client = std::os::unix::net::UnixStream::connect(&socket_path_clone).unwrap();
        let request = r#"{"jsonrpc":"2.0","id":1,"method":"PING","params":null}"#;
        client.write_all(request.as_bytes()).unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        // Shut down the write half so the server sees EOF
        client.shutdown(std::net::Shutdown::Write).unwrap();

        let reader = BufReader::new(&client);
        let mut lines = reader.lines();
        let line = lines.next().expect("should receive a response").unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], 1);
        assert!(v["result"].is_object());
        assert!(v.get("error").is_none());

        handle.join().unwrap();
    }

    #[test]
    fn socket_server_unknown_method_dispatches_to_tool_handler() {
        let tmp = TempDir::new().unwrap();
        let socket_path = temp_socket_path(&tmp);

        remove_stale_socket(&socket_path);
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let socket_path_clone = socket_path.clone();

        let handle = std::thread::spawn(move || {
            let store = Arc::new(Mutex::new(temp_store()));
            let consolidation = Arc::new(ConsolidationConfig::default());
            if let Ok(stream) = listener.accept().map(|(s, _)| s) {
                handle_connection(stream, store, consolidation, false, false);
            }
        });

        let mut client = std::os::unix::net::UnixStream::connect(&socket_path_clone).unwrap();
        // hyphae_memory_stats is a valid tool with no required params
        let request = r#"{"jsonrpc":"2.0","id":2,"method":"hyphae_memory_stats","params":{}}"#;
        client.write_all(request.as_bytes()).unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();

        let reader = BufReader::new(&client);
        let line = reader.lines().next().expect("response").unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], 2);
        // stats should succeed — result present, no error
        assert!(v.get("result").is_some(), "expected result, got: {v}");

        handle.join().unwrap();
    }

    #[test]
    fn socket_server_cap_stats_returns_versioned_json() {
        let tmp = TempDir::new().unwrap();
        let socket_path = temp_socket_path(&tmp);

        remove_stale_socket(&socket_path);
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let socket_path_clone = socket_path.clone();

        let handle = std::thread::spawn(move || {
            let store = Arc::new(Mutex::new(temp_store()));
            let consolidation = Arc::new(ConsolidationConfig::default());
            if let Ok(stream) = listener.accept().map(|(s, _)| s) {
                handle_connection(stream, store, consolidation, false, false);
            }
        });

        let mut client = std::os::unix::net::UnixStream::connect(&socket_path_clone).unwrap();
        let request = r#"{"jsonrpc":"2.0","id":3,"method":"cap_stats","params":{}}"#;
        client.write_all(request.as_bytes()).unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();

        let reader = BufReader::new(&client);
        let line = reader.lines().next().expect("response").unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], 3);
        let result = &v["result"];
        assert!(result.is_object());
        assert_eq!(result["schema_version"], "1.0");
        assert!(result["total_memories"].is_number());
        assert!(result["total_topics"].is_number());
        assert!(result.get("error").is_none());

        handle.join().unwrap();
    }
}
