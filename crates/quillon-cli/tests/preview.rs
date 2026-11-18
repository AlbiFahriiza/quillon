//! Preview acceptance tests (plan §12 v0.2): a single-scene draft builds in
//! under a second, the server serves the page, state and frames, and a
//! document save rebuilds the strip.

use quillon_cache::FrameCache;
use quillon_cli::preview::{build_state, spawn_preview};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn unique_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "quillon-preview-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fresh_cache(tag: &str) -> FrameCache {
    FrameCache::open(&unique_dir(tag).join("cache"), 64 << 20).unwrap()
}

#[test]
fn single_scene_draft_builds_under_one_second() {
    // The v0.2 acceptance target, measured cold: fresh cache, so every
    // frame renders for real.
    let doc = PathBuf::from(format!("{FIXTURES}/valid.json"));
    let started = Instant::now();
    let state = build_state(&doc, &fresh_cache("cold"));
    let elapsed = started.elapsed();
    assert!(state.valid, "fixture must be valid: {:?}", state.errors);
    assert!(!state.frames.is_empty(), "the strip has frames");
    assert!(
        elapsed < Duration::from_secs(1),
        "draft preview must build in <1s, took {elapsed:?}"
    );
}

#[test]
fn second_build_is_cache_assisted() {
    let cache = fresh_cache("warm");
    let doc = PathBuf::from(format!("{FIXTURES}/valid.json"));
    let first = build_state(&doc, &cache);
    let (hits_before, misses_before) = cache.stats();
    assert_eq!(hits_before, 0, "cold build has no hits");
    assert_eq!(
        misses_before,
        first.frames.len() as u64,
        "cold build misses every frame"
    );
    let _second = build_state(&doc, &cache);
    let (hits, misses) = cache.stats();
    assert_eq!(misses, misses_before, "the warm build renders nothing new");
    assert_eq!(hits, first.frames.len() as u64, "every frame is a hit");
}

struct Server {
    addr: std::net::SocketAddr,
}

fn start_server(doc_path: PathBuf) -> Server {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let cache = FrameCache::open(&unique_dir("server").join("cache"), 64 << 20).unwrap();
        let addr = runtime
            .block_on(spawn_preview(&doc_path, 0, cache))
            .expect("server binds");
        tx.send(addr).expect("send addr");
        runtime.block_on(std::future::pending::<()>());
    });
    Server {
        addr: rx.recv().expect("server started"),
    }
}

fn http_get(addr: std::net::SocketAddr, path: &str) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("write request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read response");
    let status: u16 = std::str::from_utf8(&response[..12])
        .expect("ascii status")
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("status code");
    let body = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| response[at + 4..].to_vec())
        .unwrap_or_default();
    (status, body)
}

#[test]
fn server_serves_page_state_and_frames() {
    let server = start_server(PathBuf::from(format!("{FIXTURES}/valid.json")));
    let (status, page) = http_get(server.addr, "/");
    assert_eq!(status, 200);
    assert!(page.windows(12).any(|w| w == b"Quillon prev"), "page title");
    let (status, body) = http_get(server.addr, "/state");
    assert_eq!(status, 200);
    let state: serde_json::Value = serde_json::from_slice(&body).expect("state json");
    assert_eq!(state["valid"], serde_json::json!(true));
    assert_eq!(state["frame_count"], serde_json::json!(10));
    let frames = state["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty());
    let (status, png) = http_get(server.addr, "/frame/0.png");
    assert_eq!(status, 200);
    assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "PNG magic");
    let (status, _) = http_get(server.addr, "/frame/999.png");
    assert_eq!(status, 404);
    let (status, _) = http_get(server.addr, "/nope");
    assert_eq!(status, 404);
}

#[test]
fn document_save_rebuilds_the_strip() {
    // Watch chain end to end: notify (watching the document's directory,
    // because editors save via rename) -> debounced rebuild -> new state.
    let dir = unique_dir("watch");
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::copy(
        format!("{FIXTURES}/assets/font.ttf"),
        dir.join("assets/font.ttf"),
    )
    .unwrap();
    std::fs::copy(
        format!("{FIXTURES}/assets/dot.png"),
        dir.join("assets/dot.png"),
    )
    .unwrap();
    let doc = dir.join("doc.json");
    let text = std::fs::read_to_string(format!("{FIXTURES}/valid.json")).unwrap();
    std::fs::write(&doc, &text).unwrap();

    let server = start_server(doc.clone());
    let (_, body) = http_get(server.addr, "/state");
    let state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(state["frame_count"], serde_json::json!(10));

    // A longer scene: the frame count must change after the rebuild.
    let edited = text.replace("\"duration\": 705600000", "\"duration\": 1411200000");
    assert_ne!(edited, text, "fixture must contain the duration");
    std::fs::write(&doc, &edited).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, body) = http_get(server.addr, "/state");
        let state: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if state["frame_count"] == serde_json::json!(20) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the preview must rebuild within 10s of a save"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
