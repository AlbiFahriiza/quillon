//! `quillon preview` (plan §12 v0.2, research/G §5, D §4): a localhost
//! filmstrip server. The draft is a half-resolution copy of the document
//! rendered through the exact same `render_tick` path (same-engine
//! guarantee), sampled at a handful of ticks and cached content-addressed,
//! so a single-scene draft renders in well under a second and a save
//! re-renders only the frames whose keys changed. `notify` watches the
//! document; changes rebuild the strip and push an SSE reload to the page.
//!
//! The server binds 127.0.0.1 only, serves one page, PNG frames, a JSON
//! state document and an SSE stream — a hand-rolled HTTP/1.1 subset on
//! tokio, because the stack table approves tokio for preview and nothing
//! heavier (plan §7).

use quillon_cache::{FrameCache, SessionKeys};
use quillon_core::document::Document;
use quillon_core::draft::scale_document;
use quillon_core::frame_source::{Frame, NullFrameSource};
use quillon_core::validate_str;
use quillon_render::{RENDERER_VERSION, Renderer};
use serde_json::json;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

/// Draft frames sample the timeline at (at most) this many points.
const FILMSTRIP_SAMPLES: usize = 12;

/// How long the watcher waits for the editor's write to settle.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(150);

/// In-memory layer of the preview cache; drafts are quarter-size frames.
pub const PREVIEW_CACHE_MEMORY: usize = 64 << 20;

/// Where the preview cache lives: the system temp dir, so repeated previews
/// hit their frames without polluting the project tree.
pub fn preview_cache_dir() -> PathBuf {
    std::env::temp_dir().join("quillon-preview-cache")
}

/// One filmstrip frame: PNG bytes plus its position on the timeline.
pub struct PreviewFrame {
    pub tick_ms: u64,
    pub png: Vec<u8>,
}

/// Everything the page shows. Rebuilt from scratch on every document save.
pub struct PreviewState {
    pub doc_name: String,
    pub valid: bool,
    pub errors: Vec<quillon_core::ValidationError>,
    pub lint: Vec<quillon_core::LintFinding>,
    pub frames: Vec<PreviewFrame>,
    pub frame_count: u64,
    pub width: u32,
    pub height: u32,
}

/// Read + validate + render the draft filmstrip for one document version.
/// This is the whole preview build; the <1s acceptance target is measured
/// across this call.
pub fn build_state(doc_path: &Path, cache: &FrameCache) -> PreviewState {
    let doc_name = doc_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    let base_dir = doc_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let text = match std::fs::read_to_string(doc_path) {
        Ok(text) => text,
        Err(err) => {
            return PreviewState {
                doc_name,
                valid: false,
                errors: vec![quillon_core::ValidationError {
                    path: String::new(),
                    code: "read".into(),
                    message: format!("cannot read the document: {err}"),
                }],
                lint: Vec::new(),
                frames: Vec::new(),
                frame_count: 0,
                width: 0,
                height: 0,
            };
        }
    };
    let doc = match validate_str(&text, &base_dir) {
        Ok(doc) => doc,
        Err(errors) => {
            return PreviewState {
                doc_name,
                valid: false,
                errors,
                lint: Vec::new(),
                frames: Vec::new(),
                frame_count: 0,
                width: 0,
                height: 0,
            };
        }
    };
    let lint = quillon_core::lint_document(&doc);
    let frame_count = doc.frame_count();
    let (width, height) = (doc.size.w, doc.size.h);
    let draft = scale_document(&doc, 2);
    let frames = render_filmstrip(&draft, &base_dir, cache);
    PreviewState {
        doc_name,
        valid: true,
        errors: Vec::new(),
        lint,
        frames,
        frame_count,
        width: width / 2,
        height: height / 2,
    }
}

/// Sample the timeline at up to [`FILMSTRIP_SAMPLES`] ticks and render each
/// through the standard Renderer, consulting the frame cache first
/// (research/G §5: only frames whose keys changed re-render).
pub fn render_filmstrip(doc: &Document, base_dir: &Path, cache: &FrameCache) -> Vec<PreviewFrame> {
    let total = doc.total_duration();
    if total <= 0 {
        return Vec::new();
    }
    let count = FILMSTRIP_SAMPLES.min(doc.frame_count() as usize).max(1);
    let keys = SessionKeys::new(doc, base_dir, RENDERER_VERSION, 2).ok();
    let renderer = match Renderer::new(doc, base_dir) {
        Ok(renderer) => renderer,
        Err(_) => return Vec::new(),
    };
    let mut frames = Vec::with_capacity(count);
    for i in 0..count {
        let tick = i as i64 * total / count as i64;
        let (scene_idx, local) = match doc.scene_at(tick) {
            Some(pos) => pos,
            None => continue,
        };
        let frame = keys
            .as_ref()
            .and_then(|k| cache.get(&k.frame_key(doc, scene_idx, local)))
            .or_else(|| {
                let frame = renderer.render_tick(tick, &NullFrameSource).ok()?;
                if let Some(k) = &keys {
                    let _ = cache.put(k.frame_key(doc, scene_idx, local), frame.clone());
                }
                Some(frame)
            });
        if let Some(frame) = frame {
            frames.push(PreviewFrame {
                tick_ms: (tick / (quillon_core::time::FLICKS_PER_SECOND / 1000)).max(0) as u64,
                png: encode_png(&frame),
            });
        }
    }
    frames
}

fn encode_png(frame: &Frame) -> Vec<u8> {
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    use image::ImageEncoder as _;
    encoder
        .write_image(
            &frame.rgba,
            frame.width,
            frame.height,
            image::ExtendedColorType::Rgba8,
        )
        .ok();
    out
}

/// Start the preview server: bind, build the first state, watch the
/// document, and serve until the process exits. Returns the bound address.
pub async fn spawn_preview(
    doc_path: &Path,
    port: u16,
    cache: FrameCache,
) -> io::Result<std::net::SocketAddr> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr = listener.local_addr()?;
    let state = Arc::new(RwLock::new(build_state(doc_path, &cache)));
    let (events, _) = broadcast::channel::<()>(16);

    spawn_watcher(
        doc_path.to_path_buf(),
        cache,
        Arc::clone(&state),
        events.clone(),
    );

    let state_for_conn = Arc::clone(&state);
    let events_for_conn = events;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let state = Arc::clone(&state_for_conn);
            let events = events_for_conn.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_connection(stream, state, events).await {
                    let _ = err;
                }
            });
        }
    });
    Ok(addr)
}

/// Watch the document's directory (editors save via rename, so the file
/// itself can vanish mid-write) and rebuild the state after the write
/// settles. Rebuilds run on the blocking pool: they render.
fn spawn_watcher(
    doc_path: PathBuf,
    cache: FrameCache,
    state: Arc<RwLock<PreviewState>>,
    events: broadcast::Sender<()>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(watcher) => watcher,
        Err(_) => return,
    };
    let watch_dir = doc_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let file_name = doc_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    use notify::Watcher as _;
    if watcher
        .watch(&watch_dir, notify::RecursiveMode::NonRecursive)
        .is_err()
    {
        return;
    }
    // Keep the watcher alive for the process lifetime.
    std::mem::forget(watcher);
    std::thread::spawn(move || {
        let relevant = |paths: &[PathBuf]| {
            paths
                .iter()
                .any(|p| p.file_name().map(|n| n == file_name).unwrap_or(false))
        };
        loop {
            // Wait for the first relevant event, then drain until quiet.
            let first = loop {
                match rx.recv() {
                    Ok(Ok(event)) if relevant(&event.paths) => break event,
                    Ok(_) => continue,
                    Err(_) => return,
                }
            };
            let _ = first;
            let deadline = Instant::now() + WATCH_DEBOUNCE;
            while let Ok(event) =
                rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                if let Ok(event) = event {
                    if relevant(&event.paths) {
                        // Extend the quiet window on every write.
                        continue;
                    }
                }
            }
            // This watcher thread is dedicated, so the rebuild runs right
            // here; the broadcast then wakes every SSE subscriber.
            let next = build_state(&doc_path, &cache);
            if let Ok(mut slot) = state.write() {
                *slot = next;
            }
            let _ = events.send(());
        }
    });
}

async fn handle_connection(
    mut stream: TcpStream,
    state: Arc<RwLock<PreviewState>>,
    events: broadcast::Sender<()>,
) -> io::Result<()> {
    let mut buffer = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    // Read the request head only; every route is a GET without a body.
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..n]);
        if buffer.windows(4).any(|w| w == b"\r\n\r\n") || buffer.len() > 16 * 1024 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buffer);
    let mut request_line = head.lines().next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or("");
    let path = request_line.next().unwrap_or("");
    if method != "GET" {
        return respond(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed\n",
        )
        .await;
    }
    match path {
        "/" => respond(&mut stream, 200, "text/html; charset=utf-8", PAGE).await,
        "/state" => {
            let body = state_json(&state);
            respond(&mut stream, 200, "application/json", &body).await
        }
        p if p.starts_with("/frame/") => {
            let index: Option<usize> = p
                .strip_prefix("/frame/")
                .and_then(|rest| rest.strip_suffix(".png"))
                .and_then(|n| n.parse().ok());
            match index.and_then(|i| state.read().ok()?.frames.get(i).map(|f| f.png.clone())) {
                Some(png) => respond(&mut stream, 200, "image/png", &png).await,
                None => {
                    respond(
                        &mut stream,
                        404,
                        "text/plain; charset=utf-8",
                        b"no such frame\n",
                    )
                    .await
                }
            }
        }
        "/events" => serve_events(stream, events).await,
        _ => {
            respond(
                &mut stream,
                404,
                "text/plain; charset=utf-8",
                b"not found\n",
            )
            .await
        }
    }
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

/// Server-sent events: one `reload` per rebuild, a comment heartbeat every
/// 15s so proxies do not close the stream.
async fn serve_events(mut stream: TcpStream, events: broadcast::Sender<()>) -> io::Result<()> {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(head.as_bytes()).await?;
    let mut receiver = events.subscribe();
    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(()) => {
                        stream
                            .write_all(b"event: reload\ndata: {}\n\n")
                            .await?;
                        stream.flush().await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(15)) => {
                stream.write_all(b": keep-alive\n\n").await?;
                stream.flush().await?;
            }
        }
    }
}

fn state_json(state: &Arc<RwLock<PreviewState>>) -> Vec<u8> {
    let state = state.read().unwrap_or_else(|e| e.into_inner());
    let errors: Vec<_> = state
        .errors
        .iter()
        .map(|e| json!({ "path": e.path, "code": e.code, "message": e.message }))
        .collect();
    let lint: Vec<_> = state
        .lint
        .iter()
        .map(|f| json!({ "path": f.path, "code": f.code, "message": f.message }))
        .collect();
    let frames: Vec<_> = state
        .frames
        .iter()
        .enumerate()
        .map(|(i, f)| json!({ "index": i, "tick_ms": f.tick_ms }))
        .collect();
    let body = json!({
        "document": state.doc_name,
        "valid": state.valid,
        "errors": errors,
        "lint": lint,
        "frames": frames,
        "frame_count": state.frame_count,
        "width": state.width,
        "height": state.height,
    });
    serde_json::to_vec(&body).unwrap_or_default()
}

/// The filmstrip page: self-contained, no external assets. It fetches the
/// state and the frames, and reloads on the SSE `reload` event. What it
/// shows is the product surface (validity, lint codes, frames) — never
/// renderer internals (disclosure rules).
const PAGE: &[u8] = br#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Quillon preview</title>
<style>
  :root { color-scheme: dark; }
  body { margin: 0; font: 14px/1.5 system-ui, sans-serif; background: #101418; color: #e8e8e8; }
  header { padding: 12px 16px; border-bottom: 1px solid #2a3138; position: sticky; top: 0; background: #101418; }
  h1 { font-size: 16px; margin: 0 0 4px; }
  #status { font-size: 12px; color: #9aa4ad; }
  #status.invalid { color: #ff8f6b; }
  #strip { display: flex; flex-wrap: wrap; gap: 8px; padding: 16px; }
  #strip img { max-width: 100%; height: auto; border: 1px solid #2a3138; border-radius: 4px; }
  figure { margin: 0; }
  figcaption { font-size: 11px; color: #9aa4ad; text-align: center; padding-top: 4px; }
  ul { margin: 4px 0 0; padding-left: 18px; font-size: 12px; color: #ff8f6b; }
</style>
</head>
<body>
<header>
  <h1 id="doc">loading...</h1>
  <div id="status"></div>
  <ul id="problems"></ul>
</header>
<main id="strip"></main>
<script>
async function refresh() {
  const s = await (await fetch('/state')).then(r => r.json());
  document.getElementById('doc').textContent = s.document;
  const status = document.getElementById('status');
  status.textContent = s.valid
    ? s.frames.length + ' draft frames of ' + s.frame_count + ' @ ' + s.width + 'x' + s.height
    : 'invalid document';
  status.classList.toggle('invalid', !s.valid);
  const problems = document.getElementById('problems');
  problems.innerHTML = '';
  for (const e of s.errors) {
    const li = document.createElement('li');
    li.textContent = e.path + ': ' + e.message;
    problems.appendChild(li);
  }
  for (const f of s.lint) {
    const li = document.createElement('li');
    li.textContent = 'lint ' + f.code + ' at ' + f.path + ': ' + f.message;
    problems.appendChild(li);
  }
  const strip = document.getElementById('strip');
  strip.innerHTML = '';
  for (const f of s.frames) {
    const figure = document.createElement('figure');
    const img = document.createElement('img');
    img.src = '/frame/' + f.index + '.png';
    const caption = document.createElement('figcaption');
    caption.textContent = (f.tick_ms / 1000).toFixed(2) + 's';
    figure.appendChild(img);
    figure.appendChild(caption);
    strip.appendChild(figure);
  }
}
const source = new EventSource('/events');
source.addEventListener('reload', refresh);
refresh();
</script>
</body>
</html>
"#;
