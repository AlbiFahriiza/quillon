//! Library surface of the CLI crate: the preview server, the shared render
//! machinery, the MCP server and the caption/audio command implementations
//! live here so integration tests can exercise them; the `quillon` binary
//! in `main.rs` is a thin shell over it.

pub mod audio_cmd;
pub mod caption_cmd;
pub mod mcp;
pub mod preview;
pub mod render_doc;
