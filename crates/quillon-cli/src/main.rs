//! Quillon CLI (plan §4 F10): `validate` and `render`, machine-readable via
//! `--json`, semantic exit codes:
//!   0 success · 1 operational error · 2 invalid document · 3 lint findings.
//! Usage errors exit 1 (clap's default 2 is reserved for document problems).

use clap::{Parser, Subcommand, ValueEnum};
use quillon_core::frame_source::{FrameSource, NullFrameSource};
use quillon_core::{Document, lint_document, validate_str};
use quillon_media::DocumentFrameSource;
use quillon_media::Mp4Sink;
use quillon_render::{RENDERER_VERSION, render_document_parallel};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "quillon",
    version,
    about = "Video as Code: create and edit video from code"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a document: schema, semantics, asset probe, then lint.
    Validate {
        doc: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Exit 0 even when lint findings exist.
        #[arg(long)]
        allow_lint: bool,
    },
    /// Serve a live draft filmstrip of a document on localhost.
    Preview {
        doc: PathBuf,
        /// Port to serve on (localhost only).
        #[arg(long, default_value_t = 4721)]
        port: u16,
    },
    /// Report what a media file contains: duration, resolution, fps, codecs.
    Probe {
        file: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Summarize a validated document: scenes, timing, assets, audio mix.
    Inspect {
        doc: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Render a document to MP4 (system ffmpeg) or PNG frames.
    Render {
        doc: PathBuf,
        /// Output: a .mp4 file, or a directory for PNG frames.
        #[arg(short, long)]
        out: PathBuf,
        /// Output container: `mp4` (default) or `png`.
        #[arg(long, default_value = "mp4")]
        format: Format,
        /// Parallel render workers (default: one per CPU core).
        #[arg(long)]
        workers: Option<usize>,
        /// Store and reuse rendered frames in this directory: re-renders
        /// only compute frames whose scene content changed.
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// Normalize the audio mix to this integrated loudness (LUFS);
        /// the EBU R128 broadcast preset is -23.
        #[arg(long)]
        lufs: Option<f64>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Format {
    Png,
    Mp4,
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            // Exit 1, not clap's 2: that code belongs to invalid documents.
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let code = match cli.command {
        Command::Validate {
            doc,
            json,
            allow_lint,
        } => validate(&doc, json, allow_lint),
        Command::Probe { file, json } => probe_cmd(&file, json),
        Command::Inspect { doc, json } => inspect(&doc, json),
        Command::Render {
            doc,
            out,
            format,
            workers,
            cache_dir,
            lufs,
            json,
        } => render(&doc, &out, format, workers, cache_dir, lufs, json),
        Command::Preview { doc, port } => preview(&doc, port),
    };
    std::process::exit(code);
}

fn preview(path: &Path, port: u16) -> i32 {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cannot start the preview server: {err}");
            return 1;
        }
    };
    let cache_dir = quillon_cli::preview::preview_cache_dir();
    let cache = match quillon_cache::FrameCache::open(
        &cache_dir,
        quillon_cli::preview::PREVIEW_CACHE_MEMORY,
    ) {
        Ok(cache) => cache,
        Err(err) => {
            eprintln!("cannot open the preview cache: {err}");
            return 1;
        }
    };
    let started = std::time::Instant::now();
    let address = runtime.block_on(quillon_cli::preview::spawn_preview(path, port, cache));
    let address = match address {
        Ok(address) => address,
        Err(err) => {
            eprintln!("cannot bind the preview server: {err}");
            return 1;
        }
    };
    println!(
        "preview ready in {:.0?} — http://{} (Ctrl-C stops)",
        started.elapsed(),
        address
    );
    // Serve until the process is stopped; there is no in-band quit.
    runtime.block_on(std::future::pending::<()>());
    0
}

fn probe_cmd(path: &Path, json: bool) -> i32 {
    match quillon_media::probe::probe(path) {
        Ok(info) => {
            if json {
                let fps: serde_json::Value = info
                    .fps
                    .map(|(num, den)| json!({ "num": num, "den": den }))
                    .unwrap_or(serde_json::Value::Null);
                let body = json!({
                    "file": path,
                    "duration_seconds": info.duration,
                    "width": info.width,
                    "height": info.height,
                    "fps": fps,
                    "video_codec": info.video_codec,
                    "audio_codec": info.audio_codec,
                });
                println!("{body}");
            } else {
                let fps = info
                    .fps
                    .map(|(num, den)| format!("{num}/{den}"))
                    .unwrap_or_else(|| "n/a".into());
                println!(
                    "{}: {}x{} @ {fps} fps",
                    path.display(),
                    info.width.map(|w| w.to_string()).unwrap_or("-".into()),
                    info.height.map(|h| h.to_string()).unwrap_or("-".into())
                );
                println!(
                    "  duration: {}",
                    info.duration
                        .map(|d| format!("{d:.3} s"))
                        .unwrap_or("n/a".into())
                );
                println!("  video: {}", info.video_codec.as_deref().unwrap_or("none"));
                println!("  audio: {}", info.audio_codec.as_deref().unwrap_or("none"));
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn inspect(path: &Path, json: bool) -> i32 {
    let doc = match read_document(path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    let scenes: Vec<_> = doc
        .scenes
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "duration": s.duration,
                "elements": s.elements.len(),
                "transition_in": s.transition_in.as_ref().map(|t| t.kind_str()),
            })
        })
        .collect();
    let assets: Vec<_> = doc
        .assets
        .iter()
        .map(|(id, a)| json!({ "id": id, "type": a.kind_str(), "src": a.src() }))
        .collect();
    let audio: Vec<_> = doc
        .audio
        .iter()
        .map(|t| {
            json!({
                "asset": t.asset, "gain": t.gain, "start_at": t.start_at,
                "fades": t.fades.len(),
            })
        })
        .collect();
    if json {
        let body = json!({
            "file": path,
            "size": { "w": doc.size.w, "h": doc.size.h },
            "fps": doc.fps,
            "timebase": doc.timebase,
            "audio_rate": doc.audio_rate,
            "duration_ticks": doc.total_duration(),
            "frames": doc.frame_count(),
            "scenes": scenes,
            "assets": assets,
            "audio": audio,
        });
        println!("{body}");
    } else {
        println!(
            "{}: {} scene(s), {} frame(s), {} asset(s), {} audio track(s)",
            path.display(),
            doc.scenes.len(),
            doc.frame_count(),
            doc.assets.len(),
            doc.audio.len()
        );
    }
    0
}

fn read_document(path: &Path) -> Result<Document, i32> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("cannot read {}: {err}", path.display());
            return Err(1);
        }
    };
    let base = path.parent().unwrap_or(Path::new("."));
    match validate_str(&text, base) {
        Ok(doc) => Ok(doc),
        Err(errors) => {
            for error in &errors {
                eprintln!("{error}");
            }
            eprintln!(
                "{} is not a valid document ({} problems)",
                path.display(),
                errors.len()
            );
            Err(2)
        }
    }
}

fn validate(path: &Path, json: bool, allow_lint: bool) -> i32 {
    let doc = match read_document(path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    let findings = lint_document(&doc);
    if json {
        let body = json!({
            "valid": true,
            "errors": [],
            "lint": findings,
        });
        println!("{body}");
    } else {
        for finding in &findings {
            eprintln!(
                "lint {} [{}]: {}",
                finding.path, finding.code, finding.message
            );
        }
    }
    if !findings.is_empty() && !allow_lint {
        if !json {
            eprintln!(
                "{} is valid but has {} lint finding(s); pass --allow-lint to accept them",
                path.display(),
                findings.len()
            );
        }
        return 3;
    }
    if !json {
        println!("{}: valid", path.display());
    }
    0
}

/// In-memory cache layer budget; the disk layer is bounded by the caller's
/// directory. 128 MiB is roughly sixteen 1080p frames.
const CACHE_MEMORY_BYTES: usize = 128 << 20;

fn render(
    path: &Path,
    out: &Path,
    format: Format,
    workers: Option<usize>,
    cache_dir: Option<PathBuf>,
    lufs: Option<f64>,
    json: bool,
) -> i32 {
    let doc = match read_document(path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let workers = workers.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    // The document's video assets, decoded on demand (plan §6.3); documents
    // without video render against the null source.
    let src: Arc<dyn FrameSource> = match DocumentFrameSource::open(&doc, base) {
        Ok(source) if !source.is_empty() => Arc::new(source),
        _ => Arc::new(NullFrameSource),
    };

    // Opt-in cache: content-addressed frames keyed by document subtree,
    // renderer version, asset hashes and tick (research/G).
    let cache = match cache_dir {
        Some(dir) => match quillon_cache::FrameCache::open(&dir, CACHE_MEMORY_BYTES) {
            Ok(cache) => Some(cache),
            Err(err) => {
                eprintln!("cannot open the cache directory {}: {err}", dir.display());
                return 1;
            }
        },
        None => None,
    };
    let keys = match &cache {
        Some(_) => match quillon_cache::SessionKeys::new(&doc, base, RENDERER_VERSION, 0) {
            Ok(keys) => Some(Arc::new(keys)),
            Err(err) => {
                eprintln!("cannot hash the document assets: {err}");
                return 1;
            }
        },
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

    let rx =
        match render_document_parallel(&doc, base, workers, Arc::clone(&src), cache_get, cache_put)
        {
            Ok(rx) => rx,
            Err(err) => {
                eprintln!("cannot prepare the render: {err}");
                return 1;
            }
        };

    // Stream frames to the output as they arrive: peak memory is the
    // pipeline chunk plus the channel, never the whole video (research/R).
    let mut sink = match format {
        Format::Mp4 => match Mp4Sink::new(out, doc.size.w, doc.size.h, doc.fps) {
            Ok(sink) => Some(sink),
            Err(err) => {
                eprintln!("cannot start the encoder: {err}");
                return 1;
            }
        },
        Format::Png => {
            if let Err(err) = std::fs::create_dir_all(out) {
                eprintln!("cannot create {}: {err}", out.display());
                return 1;
            }
            None
        }
    };
    let mut hashes = Vec::new();
    let mut frames = 0u64;
    for job in rx {
        let job = match job {
            Ok(job) => job,
            Err(err) => {
                eprintln!("render failed at frame {frames}: {err}");
                return 1;
            }
        };
        let write = match &mut sink {
            Some(sink) => sink.write_frame(&job.frame).map_err(|e| e.to_string()),
            None => save_png(&job.frame, &out.join(format!("frame_{frames:06}.png")))
                .map_err(|e| e.to_string()),
        };
        if let Err(err) = write {
            eprintln!("writing the output stopped: {err}");
            return 1;
        }
        hashes.push(quillon_render_frame_hash(&job.frame));
        frames += 1;
    }
    if let Some(sink) = sink {
        if let Err(err) = sink.finish() {
            eprintln!("encoding failed: {err}");
            return 1;
        }
    }
    let manifest_path = match format {
        Format::Mp4 => manifest_path_for(out),
        Format::Png => out.join("manifest.json"),
    };
    if let Err(err) = write_manifest(doc.size.w, doc.size.h, &manifest_path, &hashes) {
        eprintln!("cannot write {}: {err}", manifest_path.display());
        return 1;
    }

    if let Some(cache) = &cache {
        let (hits, misses) = cache.stats();
        eprintln!("cache: {hits} hit(s), {misses} miss(es)");
    }

    // Audio: mix the document's tracks, optionally normalize loudness, and
    // mux them into the MP4 (video stream copied, audio AAC-encoded once).
    let has_audio = !doc.audio.is_empty();
    if has_audio && format == Format::Mp4 {
        if let Err(code) = mux_document_audio(&doc, base, out, lufs) {
            return code;
        }
    } else if has_audio {
        eprintln!(
            "note: the document has audio tracks; PNG output skips them (render MP4 to hear them)"
        );
    }

    if json {
        let body = json!({
            "output": out,
            "manifest": manifest_path,
            "frames": frames,
        });
        println!("{body}");
    } else {
        println!("rendered {frames} frames -> {}", out.display());
        println!("frame manifest: {}", manifest_path.display());
    }
    0
}

fn save_png(frame: &quillon_core::Frame, path: &Path) -> Result<(), image::ImageError> {
    image::save_buffer(
        path,
        &frame.rgba,
        frame.width,
        frame.height,
        image::ExtendedColorType::Rgba8,
    )
}

fn manifest_path_for(out: &Path) -> PathBuf {
    let mut name = out.file_name().unwrap_or_default().to_os_string();
    name.push(".manifest.json");
    out.with_file_name(name)
}

fn write_manifest(width: u32, height: u32, path: &Path, hashes: &[String]) -> std::io::Result<()> {
    let body = json!({
        "renderer_version": RENDERER_VERSION,
        "width": width,
        "height": height,
        "frame_count": hashes.len(),
        "frames": hashes,
    });
    std::fs::write(path, serde_json::to_string_pretty(&body)? + "\n")
}

fn quillon_render_frame_hash(frame: &quillon_core::Frame) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&frame.rgba);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Mix, normalize, and mux the document's audio into the rendered MP4.
/// The video stream is stream-copied (no re-encode); audio becomes AAC.
fn mux_document_audio(
    doc: &Document,
    base: &Path,
    out: &Path,
    lufs: Option<f64>,
) -> Result<(), i32> {
    let mut mix = match quillon_audio::mix_document_decoding(doc, base) {
        Ok(mix) => mix,
        Err(err) => {
            eprintln!("cannot mix the document audio: {err}");
            return Err(1);
        }
    };
    if let Some(target) = lufs {
        match quillon_audio::normalize_lufs(&mut mix.samples, mix.rate, target) {
            Ok(before) if before.is_finite() => {
                eprintln!("audio: {before:.1} LUFS -> {target:.1} LUFS")
            }
            Ok(_) => eprintln!("audio: silence, nothing to normalize"),
            Err(err) => {
                eprintln!("cannot normalize loudness: {err}");
                return Err(1);
            }
        }
    }
    let temp_dir = std::env::temp_dir().join(format!("quillon-mux-{}", std::process::id()));
    if let Err(err) = std::fs::create_dir_all(&temp_dir) {
        eprintln!("cannot create a temp dir for muxing: {err}");
        return Err(1);
    }
    let wav = temp_dir.join("audio.wav");
    let video_tmp = temp_dir.join("video.mp4");
    if let Err(err) = quillon_audio::write_wav(&wav, &mix.samples, mix.channels, mix.rate) {
        eprintln!("cannot write the mixed audio: {err}");
        return Err(1);
    }
    if let Err(err) = std::fs::rename(out, &video_tmp) {
        eprintln!("cannot stage the rendered video for muxing: {err}");
        return Err(1);
    }
    let mut command = ffmpeg_sidecar::command::FfmpegCommand::new();
    command
        .args(["-hide_banner", "-loglevel", "error"])
        .input(video_tmp.to_string_lossy().as_ref())
        .input(wav.to_string_lossy().as_ref())
        .args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest"])
        .overwrite()
        .output(out.to_string_lossy().as_ref());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            eprintln!("cannot start ffmpeg to mux audio: {err}");
            return Err(1);
        }
    };
    let status = match child.wait() {
        Ok(status) => status,
        Err(err) => {
            eprintln!("waiting for the muxer failed: {err}");
            return Err(1);
        }
    };
    let _ = std::fs::remove_dir_all(&temp_dir);
    if !status.success() {
        eprintln!("audio muxing failed (ffmpeg exit {status})");
        return Err(1);
    }
    Ok(())
}
