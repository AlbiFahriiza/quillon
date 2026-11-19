//! Document rendering machinery shared by the CLI `render` subcommand and
//! the MCP `render_document` tool (plan §6.3): parallel frame render,
//! PNG/MP4 sinks, frame manifests, audio mix/mux. Pure engine code — no
//! exit codes, no printing; callers decide how to report.

use quillon_core::frame_source::{FrameSource, NullFrameSource};
use quillon_core::{Document, Frame};
use quillon_media::{DocumentFrameSource, Mp4Sink};
use quillon_render::{RENDERER_VERSION, render_document_parallel};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Png,
    Mp4,
}

/// Everything a finished render can report (CLI prints it, MCP returns it
/// from `render_status`).
#[derive(Clone, Debug)]
pub struct RenderOutcome {
    pub frames: u64,
    pub output: PathBuf,
    pub manifest_path: PathBuf,
    pub frame_hashes: Vec<String>,
    /// (hits, misses) when a cache directory was given; batch renders
    /// accumulate these to prove shared-cache reuse.
    pub cache_stats: Option<(u64, u64)>,
}

/// In-memory cache layer budget; the disk layer is bounded by the caller's
/// directory. 128 MiB is roughly sixteen 1080p frames.
const CACHE_MEMORY_BYTES: usize = 128 << 20;

/// Render a validated document to MP4 (system ffmpeg) or PNG frames and
/// write its manifest. `base` resolves relative asset paths; `workers`,
/// `cache_dir` and `lufs` mirror the CLI flags (None = defaults).
/// Everything a render may tune, bundled for the parameter budget.
pub struct RenderOptions<'a> {
    pub workers: Option<usize>,
    pub cache_dir: Option<&'a Path>,
    pub lufs: Option<f64>,
    /// The ffmpeg `-c:v` codec ("libx264" for software; a hardware
    /// encoder for `--encode auto|name`). Encoded bytes are not part of
    /// the determinism contract — frames are.
    pub video_codec: &'a str,
}

pub fn render_document(
    doc: &Document,
    base: &Path,
    out: &Path,
    format: Format,
    opts: &RenderOptions,
) -> Result<RenderOutcome, String> {
    // Hardening D6: a zero worker count is a typo, not a request for a
    // silent fallback — name it instead of quietly rendering with one.
    if opts.workers == Some(0) {
        return Err("--workers must be at least 1".to_string());
    }
    // A directory as the output path would otherwise surface as an
    // opaque ffmpeg failure deep in the encode.
    if out.is_dir() {
        return Err(format!(
            "the output path {} is a directory; it must be a file path",
            out.display()
        ));
    }
    let workers = opts.workers.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    // The document's video assets, decoded on demand (plan §6.3); documents
    // without video render against the null source.
    let src: Arc<dyn FrameSource> = match DocumentFrameSource::open(doc, base) {
        Ok(source) if !source.is_empty() => Arc::new(source),
        _ => Arc::new(NullFrameSource),
    };

    // Opt-in cache: content-addressed frames keyed by document subtree,
    // renderer version, asset hashes and tick (research/G).
    let cache = match opts.cache_dir {
        Some(dir) => Some(
            quillon_cache::FrameCache::open(dir, CACHE_MEMORY_BYTES).map_err(|err| {
                format!("cannot open the cache directory {}: {err}", dir.display())
            })?,
        ),
        None => None,
    };
    let keys = match &cache {
        Some(_) => Some(Arc::new(
            quillon_cache::SessionKeys::new(doc, base, RENDERER_VERSION, 0)
                .map_err(|err| format!("cannot hash the document assets: {err}"))?,
        )),
        None => None,
    };
    let (cache_get, cache_put) = match (&cache, &keys) {
        (Some(cache), Some(keys)) => {
            // Each hook owns its Arc clones; `doc` travels with them so the
            // closures can outlive this stack frame ('static).
            let shared_doc = Arc::new(doc.clone());
            let get: quillon_render::CacheGet = {
                let cache = cache.clone();
                let keys = Arc::clone(keys);
                let doc = Arc::clone(&shared_doc);
                Arc::new(move |scene_idx, local| cache.get(&keys.frame_key(&doc, scene_idx, local)))
            };
            let put: quillon_render::CachePut = {
                let cache = cache.clone();
                let keys = Arc::clone(keys);
                let doc = Arc::clone(&shared_doc);
                Arc::new(move |scene_idx, local, frame| {
                    // A failed cache write costs a re-render, nothing more.
                    let _ = cache.put(keys.frame_key(&doc, scene_idx, local), frame.clone());
                })
            };
            (Some(get), Some(put))
        }
        _ => (None, None),
    };

    let rx = render_document_parallel(doc, base, workers, Arc::clone(&src), cache_get, cache_put)
        .map_err(|err| format!("cannot prepare the render: {err}"))?;

    // Stream frames to the output as they arrive: peak memory is the
    // pipeline chunk plus the channel, never the whole video (research/R).
    let mut sink = match format {
        Format::Mp4 => Some(
            Mp4Sink::new(out, doc.size.w, doc.size.h, doc.fps, opts.video_codec)
                .map_err(|err| format!("cannot start the encoder: {err}"))?,
        ),
        Format::Png => {
            std::fs::create_dir_all(out)
                .map_err(|err| format!("cannot create {}: {err}", out.display()))?;
            None
        }
    };
    let mut hashes = Vec::new();
    let mut frames = 0u64;
    for job in rx {
        let job = job.map_err(|err| format!("render failed at frame {frames}: {err}"))?;
        let write = match &mut sink {
            Some(sink) => sink.write_frame(&job.frame).map_err(|e| e.to_string()),
            None => save_png(&job.frame, &out.join(format!("frame_{frames:06}.png")))
                .map_err(|e| e.to_string()),
        };
        write.map_err(|err| format!("writing the output stopped: {err}"))?;
        hashes.push(frame_hash(&job.frame));
        frames += 1;
    }
    if let Some(sink) = sink {
        sink.finish()
            .map_err(|err| format!("encoding failed: {err}"))?;
    }
    let manifest_path = match format {
        Format::Mp4 => manifest_path_for(out),
        Format::Png => out.join("manifest.json"),
    };
    write_manifest(doc.size.w, doc.size.h, &manifest_path, &hashes)
        .map_err(|err| format!("cannot write {}: {err}", manifest_path.display()))?;

    if let Some(cache) = &cache {
        let (hits, misses) = cache.stats();
        eprintln!("cache: {hits} hit(s), {misses} miss(es)");
    }

    // Audio: mix the document's tracks, optionally normalize loudness, and
    // mux them into the MP4 (video stream copied, audio AAC-encoded once).
    if !doc.audio.is_empty() && format == Format::Mp4 {
        mux_document_audio(doc, base, out, opts.lufs)?;
    }

    let cache_stats = cache.as_ref().map(|c| c.stats());
    Ok(RenderOutcome {
        frames,
        output: out.to_path_buf(),
        manifest_path,
        frame_hashes: hashes,
        cache_stats,
    })
}

/// sha256 of a frame's RGBA buffer — the reproducible-pixels contract
/// (plan P5): same document, same hash, across runs and platforms.
pub fn frame_hash(frame: &Frame) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&frame.rgba);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn save_png(frame: &Frame, path: &Path) -> Result<(), image::ImageError> {
    image::save_buffer(
        path,
        &frame.rgba,
        frame.width,
        frame.height,
        image::ExtendedColorType::Rgba8,
    )
}

pub fn manifest_path_for(out: &Path) -> PathBuf {
    let mut name = out.file_name().unwrap_or_default().to_os_string();
    name.push(".manifest.json");
    out.with_file_name(name)
}

pub fn write_manifest(
    width: u32,
    height: u32,
    path: &Path,
    hashes: &[String],
) -> std::io::Result<()> {
    let body = json!({
        "renderer_version": RENDERER_VERSION,
        "width": width,
        "height": height,
        "frame_count": hashes.len(),
        "frames": hashes,
    });
    std::fs::write(path, serde_json::to_string_pretty(&body)? + "\n")
}

/// Mix, normalize, and mux the document's audio into the rendered MP4.
/// The video stream is stream-copied (no re-encode); audio becomes AAC.
pub fn mux_document_audio(
    doc: &Document,
    base: &Path,
    out: &Path,
    lufs: Option<f64>,
) -> Result<(), String> {
    let mut mix = quillon_audio::mix_document_decoding(doc, base)
        .map_err(|err| format!("cannot mix the document audio: {err}"))?;
    if let Some(target) = lufs {
        match quillon_audio::normalize_lufs(&mut mix.samples, mix.rate, target) {
            Ok(before) if before.is_finite() => {
                eprintln!("audio: {before:.1} LUFS -> {target:.1} LUFS")
            }
            Ok(_) => eprintln!("audio: silence, nothing to normalize"),
            Err(err) => return Err(format!("cannot normalize loudness: {err}")),
        }
    }
    let temp_dir = std::env::temp_dir().join(format!("quillon-mux-{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|err| format!("cannot create a temp dir for muxing: {err}"))?;
    let wav = temp_dir.join("audio.wav");
    let video_tmp = temp_dir.join("video.mp4");
    quillon_audio::write_wav(&wav, &mix.samples, mix.channels, mix.rate)
        .map_err(|err| format!("cannot write the mixed audio: {err}"))?;
    std::fs::rename(out, &video_tmp)
        .map_err(|err| format!("cannot stage the rendered video for muxing: {err}"))?;
    let mut command = ffmpeg_sidecar::command::FfmpegCommand::new();
    command
        .args(["-hide_banner", "-loglevel", "error"])
        .input(video_tmp.to_string_lossy().as_ref())
        .input(wav.to_string_lossy().as_ref())
        .args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest"])
        .overwrite()
        .output(out.to_string_lossy().as_ref());
    let mut child = command
        .spawn()
        .map_err(|err| format!("cannot start ffmpeg to mux audio: {err}"))?;
    let status = child
        .wait()
        .map_err(|err| format!("waiting for the muxer failed: {err}"))?;
    let _ = std::fs::remove_dir_all(&temp_dir);
    if !status.success() {
        return Err(format!("audio muxing failed (ffmpeg exit {status})"));
    }
    Ok(())
}
