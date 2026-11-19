//! Quillon CLI (plan §4 F10): `validate`, `render`, `preview`, `probe`,
//! `inspect` and `mcp`, machine-readable via `--json`, semantic exit codes:
//!   0 success · 1 operational error · 2 invalid document · 3 lint findings.
//! Usage errors exit 1 (clap's default 2 is reserved for document problems).

use clap::{Parser, Subcommand};
use quillon_cli::render_doc::Format;
use quillon_core::{Document, lint_document, validate_str};
use serde_json::json;
use std::path::{Path, PathBuf};

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
    /// Run the MCP agent server over stdio (plan §8).
    Mcp {
        /// Working root for documents, assets and outputs
        /// (default: current directory).
        #[arg(long)]
        root: Option<PathBuf>,
    },
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
        Command::Mcp { root } => quillon_cli::mcp::run(root),
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
    match quillon_cli::render_doc::render_document(
        &doc,
        base,
        out,
        format,
        workers,
        cache_dir.as_deref(),
        lufs,
    ) {
        Ok(outcome) => {
            if !doc.audio.is_empty() && format == Format::Png {
                eprintln!(
                    "note: the document has audio tracks; PNG output skips them (render MP4 to hear them)"
                );
            }
            if json {
                let body = json!({
                    "output": outcome.output,
                    "manifest": outcome.manifest_path,
                    "frames": outcome.frames,
                });
                println!("{body}");
            } else {
                println!(
                    "rendered {} frames -> {}",
                    outcome.frames,
                    outcome.output.display()
                );
                println!("frame manifest: {}", outcome.manifest_path.display());
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}
