use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};

use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{Router, routing::get};
use futures::stream::StreamExt;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoirEventKind {
    ConceptAdded,
    ConceptRefined,
    LinkAdded,
    LinkRemoved,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoirEvent {
    pub kind: MemoirEventKind,
    pub memoir_id: String,
}

pub struct MemoirEventBus {
    sender: broadcast::Sender<MemoirEvent>,
}

impl MemoirEventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(64);
        Self { sender }
    }

    pub fn emit(&self, event: MemoirEvent) {
        let _ = self.sender.send(event); // ignore if no subscribers
    }

    pub fn subscribe(&self) -> broadcast::Receiver<MemoirEvent> {
        self.sender.subscribe()
    }
}

impl Default for MemoirEventBus {
    fn default() -> Self {
        Self::new()
    }
}

// Module-level global so memoir tool handlers can emit without signature changes.
static BUS: OnceLock<Arc<MemoirEventBus>> = OnceLock::new();

/// Initialize the global bus. Call once from run_socket_server before spawning the SSE server.
pub fn init_bus() -> Arc<MemoirEventBus> {
    let bus = Arc::new(MemoirEventBus::new());
    // OnceLock::set fails silently if already set — that's fine.
    let _ = BUS.set(bus.clone());
    bus
}

/// Emit an event if the bus has been initialized.
pub fn emit(event: MemoirEvent) {
    if let Some(bus) = BUS.get() {
        bus.emit(event);
    }
}

/// Start the SSE HTTP server in a background tokio runtime thread.
/// Returns the bound SocketAddr so callers can write it to the endpoint descriptor.
pub fn start_events_server(port: u16) -> anyhow::Result<SocketAddr> {
    let bus = BUS
        .get()
        .ok_or_else(|| anyhow::anyhow!("MemoirEventBus not initialized; call init_bus() first"))?
        .clone();

    // We run a dedicated tokio runtime on a background thread so the existing
    // synchronous socket server is unaffected.
    let (addr_tx, addr_rx) = std::sync::mpsc::channel::<anyhow::Result<SocketAddr>>();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!("memoir_events: tokio runtime build failed: {e}");
                let _ = addr_tx.send(Err(anyhow::anyhow!("failed to build tokio runtime: {e}")));
                return;
            }
        };

        rt.block_on(async move {
            let result = async {
                let listener =
                    tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
                let addr = listener.local_addr()?;

                let router = Router::new().route(
                    "/memoir-events",
                    get(move || {
                        let rx = bus.subscribe();
                        let stream = BroadcastStream::new(rx).filter_map(|result| async move {
                            match result {
                                Ok(event) => {
                                    let data = serde_json::to_string(&event).ok()?;
                                    Some(Ok::<Event, Infallible>(Event::default().data(data)))
                                }
                                Err(BroadcastStreamRecvError::Lagged(n)) => {
                                    tracing::warn!("SSE subscriber lagged, dropped {n} events");
                                    None
                                }
                            }
                        });
                        async move { Sse::new(stream).keep_alive(KeepAlive::default()) }
                    }),
                );

                addr_tx.send(Ok(addr)).ok();
                axum::serve(listener, router).await?;
                Ok::<(), anyhow::Error>(())
            }
            .await;

            if let Err(e) = result {
                addr_tx.send(Err(e)).ok();
            }
        });
    });

    addr_rx
        .recv()
        .map_err(|_| anyhow::anyhow!("SSE server thread died"))?
}
