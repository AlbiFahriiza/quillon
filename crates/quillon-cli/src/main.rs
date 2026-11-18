//! Quillon CLI (plan §4 F10): `validate` and `render`, machine-readable via
//! `--json`, semantic exit codes:
//!   0 success · 1 operational error · 2 invalid document · 3 lint findings.
//! Usage errors exit 1 (clap's default 2 is reserved for document problems).

use clap::{Parser, Subcommand, ValueEnum};
use quillon_core::{Document, lint_document, validate_str};
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
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
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
        Command::Render {
            doc,
            out,
            format,
            workers,
            cache_dir,
            json,
        } => render(&doc, &out, format, workers, cache_dir, json),
    };
    std::process::exit(code);
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

    let rx = match render_document_parallel(&doc, base, workers, cache_get, cache_put) {
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
