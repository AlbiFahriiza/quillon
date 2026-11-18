//! Library surface of the CLI crate: the preview server lives here so
//! integration tests can exercise it; the `quillon` binary in `main.rs` is
//! a thin shell over it.

pub mod preview;
