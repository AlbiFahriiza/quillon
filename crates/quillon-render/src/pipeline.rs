//! Chunked, ordered, parallel frame rendering (plan §4 F7, §6.3; research/R).
//!
//! Frames render on a dedicated rayon pool in chunks of at least 20 frames;
//! finished chunks stream through a bounded channel (~2× workers) to the
//! caller — usually the encoder, the only consumer. Peak memory is one
//! chunk plus the channel buffer, never the whole video (a 1080p minute is
//! 12.4 GB if buffered naively, research/R), and a slow consumer applies
//! backpressure up the chain. Dropping the receiver aborts rendering, which
//! is how an encoder crash stops the workers.
//!
//! The cache hooks keep this crate independent of quillon-cache (plan §6.2:
//! render imports core only): callers supply lookup/store closures keyed by
//! (scene index, scene-local tick), and `render_tick` stays a pure function.

use crate::{RenderError, Renderer};
use quillon_core::document::Document;
use quillon_core::frame_source::{Frame, FrameSource};
use quillon_core::time::frame_tick;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, sync_channel};

use rayon::prelude::*;

/// One rendered frame at its document frame index.
pub struct FrameJob {
    pub index: u64,
    pub frame: Frame,
}

/// Cache lookup hook: (scene index, scene-local tick) -> cached frame.
/// Arc so it can cross into the render thread ('static).
pub type CacheGet = Arc<dyn Fn(usize, i64) -> Option<Frame> + Send + Sync>;
/// Cache store hook: (scene index, scene-local tick, rendered frame).
pub type CachePut = Arc<dyn Fn(usize, i64, &Frame) + Send + Sync>;

/// Render every frame of `doc` in parallel and stream them, in frame order,
/// over a bounded channel. The returned receiver is the only consumer;
/// dropping it aborts rendering. Errors surface as `Err` jobs in the stream.
///
/// `src` supplies decoded video frames (plan §6.3); documents without video
/// assets pass a null source and video elements error against it.
pub fn render_document_parallel(
    doc: &Document,
    base_dir: &Path,
    workers: usize,
    src: Arc<dyn FrameSource>,
    cache_get: Option<CacheGet>,
    cache_put: Option<CachePut>,
) -> Result<Receiver<Result<FrameJob, RenderError>>, RenderError> {
    let workers = workers.max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .map_err(|err| RenderError::Pool(err.to_string()))?;
    // Fail fast on unreadable assets before any pixel work: one Renderer on
    // the feeding thread mirrors what each worker thread builds for itself.
    drop(Renderer::new(doc, base_dir)?);

    // r3 F3: fps 0 makes frame_tick divide by zero — check BEFORE the
    // call, not after it (a library-built Document skips the validator,
    // and a public API must not panic on caller data).
    if doc.fps == 0 {
        return Err(RenderError::Pool(format!(
            "fps {} does not produce a usable frame tick",
            doc.fps
        )));
    }
    let frame = frame_tick(doc.fps);
    if frame <= 0 {
        return Err(RenderError::Pool(format!(
            "fps {} does not produce a usable frame tick",
            doc.fps
        )));
    }
    let count = doc.frame_count();
    let (tx, rx) = sync_channel(2 * workers);
    let doc = Arc::new(doc.clone());
    let base_dir: Arc<PathBuf> = Arc::new(base_dir.to_path_buf());
    let cache_get = cache_get.map(Arc::new);
    let cache_put = cache_put.map(Arc::new);
    let workers_copy = workers;

    std::thread::spawn(move || {
        let chunk = chunk_size(workers_copy);
        let mut start = 0u64;
        while start < count {
            let end = (start + chunk).min(count);
            let results: Vec<Result<Frame, RenderError>> = pool.install(|| {
                (start..end)
                    .into_par_iter()
                    .map(|i| {
                        render_one(
                            &doc,
                            &base_dir,
                            src.as_ref(),
                            i,
                            frame,
                            cache_get.as_deref(),
                            cache_put.as_deref(),
                        )
                    })
                    .collect()
            });
            // A chunk is complete: emit it in order. The first error (in
            // frame order) aborts the stream.
            for (offset, result) in results.into_iter().enumerate() {
                let index = start + offset as u64;
                match result {
                    Ok(frame) => {
                        if tx.send(Ok(FrameJob { index, frame })).is_err() {
                            return; // consumer went away: stop rendering
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(Err(err));
                        return;
                    }
                }
            }
            start = end;
        }
    });
    Ok(rx)
}

/// Frames per rayon task batch: plan §6.3 says at least 20 frames per
/// worker chunk, and the reorder-in-a-chunk buffer adds another 2×.
fn chunk_size(workers: usize) -> u64 {
    (2 * workers).max(20) as u64
}

/// Render (or fetch from cache) one frame. Worker threads each build their
/// own `Renderer` once (font caches are single-threaded `RefCell`s) in
/// thread-local storage; the pool is created per call, so the slots start
/// empty and die with the pool.
fn render_one(
    doc: &Document,
    base_dir: &Path,
    src: &dyn FrameSource,
    index: u64,
    frame: i64,
    cache_get: Option<&CacheGet>,
    cache_put: Option<&CachePut>,
) -> Result<Frame, RenderError> {
    let tick = index as i64 * frame;
    let (scene_idx, local) = doc.scene_at(tick).ok_or(RenderError::TickOutOfRange {
        tick,
        end: doc.total_duration(),
    })?;
    if let Some(get) = cache_get {
        if let Some(frame) = get(scene_idx, local) {
            return Ok(frame);
        }
    }
    let renderer_frame = WORKER_RENDERER.with_borrow_mut(|slot| {
        let key = doc.scenes.len();
        if slot.as_ref().is_none_or(|(k, _)| *k != key) {
            let renderer = Renderer::new(doc, base_dir)?;
            *slot = Some((key, renderer));
        }
        slot.as_ref()
            .expect("initialized above")
            .1
            .render_tick(tick, src)
    })?;
    if let Some(put) = cache_put {
        put(scene_idx, local, &renderer_frame);
    }
    Ok(renderer_frame)
}

thread_local! {
    static WORKER_RENDERER: RefCell<Option<(usize, Renderer)>> = const { RefCell::new(None) };
}
