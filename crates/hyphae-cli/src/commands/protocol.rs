use anyhow::Result;

pub(crate) fn cmd_protocol(project: Option<&str>) -> Result<()> {
    println!("{}", hyphae_mcp::memory_protocol::protocol_surface_json(project)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn test_protocol_surface_json_includes_schema_version() {
        let payload = hyphae_mcp::memory_protocol::protocol_surface_json(Some("demo"))
            .expect("serialize protocol");
        let parsed: Value = serde_json::from_str(&payload).expect("parse json");
        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        assert_eq!(parsed["project"].as_str(), Some("demo"));
    }
}
