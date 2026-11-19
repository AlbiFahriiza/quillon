//! Library surface of the CLI crate: the preview server, the shared render
//! machinery and the MCP server live here so integration tests can exercise
//! them; the `quillon` binary in `main.rs` is a thin shell over it.

pub mod mcp;
pub mod preview;
pub mod render_doc;
