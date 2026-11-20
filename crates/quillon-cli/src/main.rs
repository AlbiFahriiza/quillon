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
        /// Video encoder: "software" (libx264, default), "auto"
        /// (hardware when available, with a printed fallback), or an
        /// explicit ffmpeg encoder name.
        #[arg(long, default_value = "software")]
        encode: String,
        /// Fill the document's template variables from this CSV, one
        /// video per row (mail merge); `--out` is a `{{variable}}`
        /// pattern.
        #[arg(long)]
        data: Option<PathBuf>,
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
    /// Import or export caption tracks (plan §12 v0.5).
    Caption {
        #[command(subcommand)]
        action: CaptionAction,
    },
    /// OTIO import/export (plan §12 v0.6, subset).
    Otio {
        #[command(subcommand)]
        action: OtioAction,
    },
    /// Analyze the document's audio mix (plan §12 v0.5).
    Audio {
        #[command(subcommand)]
        action: AudioAction,
    },
}

#[derive(Subcommand)]
enum CaptionAction {
    /// Attach a subtitle file (.srt/.vtt/.ass/.ssa) as a caption track.
    Import {
        subs: PathBuf,
        /// The document to attach the track to.
        #[arg(long)]
        doc: PathBuf,
        /// Write the captioned document here (default: stdout).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Font asset id for the track (default: the document's first font).
        #[arg(long)]
        font: Option<String>,
        /// Font size in pixels.
        #[arg(long, default_value_t = 24.0)]
        size: f32,
        /// Gap to the canvas edge in pixels.
        #[arg(long, default_value_t = 48)]
        margin: i64,
        /// bottom, center or top.
        #[arg(long, default_value = "bottom")]
        position: String,
        /// Backing box color, e.g. #101418.
        #[arg(long)]
        bg: Option<String>,
        /// Keep the source timing exactly; skip regularization.
        #[arg(long)]
        no_regularize: bool,
    },
    /// Write a caption track back out as subtitles.
    Export {
        doc: PathBuf,
        /// Write subtitles here (default: stdout).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Caption track index (default 0).
        #[arg(long, default_value_t = 0)]
        track: usize,
        /// srt or vtt (default: from --out's extension, else srt).
        #[arg(long)]
        format: Option<String>,
    },
    /// Transcribe an audio file with whisper into a caption track
    /// (only built with the `transcribe` feature).
    #[cfg(feature = "transcribe")]
    Transcribe {
        audio: PathBuf,
        /// The document to attach the track to.
        #[arg(long)]
        doc: PathBuf,
        /// Write the captioned document here (default: stdout).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Whisper ggml model path (default: the pinned tiny.en in the
        /// cache, downloaded on first use).
        #[arg(long)]
        model: Option<PathBuf>,
        /// Transcription threads (default 4).
        #[arg(long, default_value_t = 4)]
        threads: u32,
        /// Font asset id for the track (default: the document's first font).
        #[arg(long)]
        font: Option<String>,
        /// Font size in pixels.
        #[arg(long, default_value_t = 24.0)]
        size: f32,
        /// Gap to the canvas edge in pixels.
        #[arg(long, default_value_t = 48)]
        margin: i64,
        /// bottom, center or top.
        #[arg(long, default_value = "bottom")]
        position: String,
        /// Backing box color, e.g. #101418.
        #[arg(long)]
        bg: Option<String>,
        /// Keep whisper's timing exactly; skip regularization.
        #[arg(long)]
        no_regularize: bool,
    },
}

#[derive(Subcommand)]
enum OtioAction {
    /// Import an OpenTimelineIO subset timeline as a document.
    Import {
        otio: PathBuf,
        /// Write the document here (default: stdout).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Where relative media URLs resolve (default: the .otio file's
        /// directory).
        #[arg(long)]
        media_dir: Option<String>,
    },
}

#[derive(Subcommand)]
enum AudioAction {
    /// Measure the mix per scene: integrated LUFS, true peak, silence.
    Analyze {
        doc: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        /// Silence threshold in dBFS (default -50).
        #[arg(long, default_value_t = -50.0)]
        silence_db: f64,
        /// Shortest silence that counts, in milliseconds (default 500).
        #[arg(long, default_value_t = 500)]
        min_silence_ms: u64,
    },
}

/// Exit from inside the command dispatch (the match arms compute codes).
fn return_exit(code: i32) -> ! {
    std::process::exit(code)
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
            encode,
            data,
            json,
        } => {
            // Resolve the encoder once for the whole run (probe on the
            // real ffmpeg; software is pinned, auto never falls back
            // silently, unknown names list what exists).
            let choice = match quillon_cli::encode::EncodeChoice::parse(&encode) {
                Ok(choice) => choice,
                Err(err) => {
                    eprintln!("{err}");
                    return_exit(1);
                }
            };
            let devices = quillon_cli::encode::HwDevices::probe();
            let plan = match quillon_cli::encode::probe_encoders()
                .and_then(|available| quillon_cli::encode::resolve(&choice, &available, &devices))
            {
                Ok(plan) => plan,
                Err(err) => {
                    eprintln!("{err}");
                    return_exit(1);
                }
            };
            if let Some(note) = &plan.fallback_note {
                eprintln!("note: {note}");
            }
            match data {
                Some(data) => {
                    let out_pattern = out.to_string_lossy().to_string();
                    let args = quillon_cli::batch::BatchArgs {
                        template: &doc,
                        data: &data,
                        out_pattern: &out_pattern,
                        format,
                        workers,
                        cache_dir: cache_dir.as_deref(),
                        lufs,
                        video_codec: &plan.codec,
                        json,
                    };
                    render_data(args)
                }
                None => render(RenderRun {
                    path: &doc,
                    out: &out,
                    format,
                    workers,
                    cache_dir: cache_dir.as_deref(),
                    lufs,
                    json,
                    codec: &plan.codec,
                    hw_encode: plan.hardware,
                }),
            }
        }
        Command::Preview { doc, port } => preview(&doc, port),
        Command::Mcp { root } => quillon_cli::mcp::run(root),
        Command::Caption { action } => match action {
            CaptionAction::Import {
                subs,
                doc,
                out,
                font,
                size,
                margin,
                position,
                bg,
                no_regularize,
            } => caption_import(
                &subs,
                &doc,
                out,
                &quillon_cli::caption_cmd::ImportFlags {
                    font,
                    size,
                    margin,
                    position,
                    bg,
                },
                no_regularize,
            ),
            CaptionAction::Export {
                doc,
                out,
                track,
                format,
            } => caption_export(&doc, out, track, format),
            #[cfg(feature = "transcribe")]
            CaptionAction::Transcribe {
                audio,
                doc,
                out,
                model,
                threads,
                font,
                size,
                margin,
                position,
                bg,
                no_regularize,
            } => caption_transcribe(
                &audio,
                &doc,
                out,
                model.as_deref(),
                threads,
                &quillon_cli::caption_cmd::ImportFlags {
                    font,
                    size,
                    margin,
                    position,
                    bg,
                },
                no_regularize,
            ),
        },
        Command::Otio { action } => match action {
            OtioAction::Import {
                otio,
                out,
                media_dir,
            } => otio_import(&otio, out, media_dir.as_deref()),
        },
        Command::Audio { action } => match action {
            AudioAction::Analyze {
                doc,
                json,
                silence_db,
                min_silence_ms,
            } => {
                // r1 F7: named errors for flag values that cannot mean
                // anything (NaN/inf thresholds, absurd windows).
                if !silence_db.is_finite() {
                    eprintln!("--silence-db must be a finite number, got {silence_db}");
                    return_exit(1);
                }
                if min_silence_ms == 0 || min_silence_ms > 3_600_000 {
                    eprintln!(
                        "--min-silence-ms must be within 1..=3600000 (1 ms to 1 h), got {min_silence_ms}"
                    );
                    return_exit(1);
                }
                audio_analyze(&doc, json, silence_db, min_silence_ms)
            }
        },
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
    // r2 R2-2: bounded read; the metadata check rejects a giant file
    // before it claims the RAM.
    let text = match quillon_cli::read_capped::read_to_string_capped(
        path,
        quillon_cli::read_capped::MAX_TEXT_FILE_BYTES,
    ) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("{err}");
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

/// `render --data`: the mail-merge batch (plan §12 v0.6). A row whose
/// filled document fails validation is an invalid-document failure (exit
/// 2); every other batch problem is operational (exit 1).
fn render_data(args: quillon_cli::batch::BatchArgs) -> i32 {
    match quillon_cli::batch::render_batch(&args) {
        Ok(summary) => {
            if args.json {
                let body = quillon_cli::batch::summary_json(&summary);
                println!("{body}");
            } else {
                println!(
                    "rendered {} row(s) -> {}",
                    summary.rows,
                    summary
                        .outputs
                        .first()
                        .map(|p| p.parent().unwrap_or(p))
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                println!(
                    "cache: {} hit(s), {} miss(es)",
                    summary.hits, summary.misses
                );
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            // The batch's validation failures always carry this marker.
            if err.contains("the filled document is invalid") {
                2
            } else {
                1
            }
        }
    }
}

/// A single render's parameters, bundled for the parameter budget.
struct RenderRun<'a> {
    path: &'a Path,
    out: &'a Path,
    format: Format,
    workers: Option<usize>,
    cache_dir: Option<&'a Path>,
    lufs: Option<f64>,
    json: bool,
    codec: &'a str,
    hw_encode: bool,
}

fn render(run: RenderRun) -> i32 {
    let RenderRun {
        path,
        out,
        format,
        workers,
        cache_dir,
        lufs,
        json,
        codec,
        hw_encode,
    } = run;
    let doc = match read_document(path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let options = quillon_cli::render_doc::RenderOptions {
        workers,
        cache_dir,
        lufs,
        video_codec: codec,
    };
    match quillon_cli::render_doc::render_document(&doc, base, out, format, &options) {
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
                    "encoder": codec,
                    // Hardware encoders change encoded bytes; the
                    // pixel-hash guarantee covers frames, not the codec.
                    "hw_encode": hw_encode,
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

/// `quillon otio import` (plan §12 v0.6): the flat OTIO subset becomes a
/// document, which must itself validate — including the media probe —
/// before it is written.
fn otio_import(otio_path: &Path, out: Option<PathBuf>, media_dir: Option<&str>) -> i32 {
    // r2 R2-2: bounded read at the import boundary.
    let input = match quillon_cli::read_capped::read_to_string_capped(
        otio_path,
        quillon_cli::read_capped::MAX_TEXT_FILE_BYTES,
    ) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    // Relative media URLs resolve against the .otio file's directory by
    // default.
    let default_dir = otio_path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| ".".to_string());
    let media_dir = media_dir.unwrap_or(&default_dir);
    match quillon_core::import_otio(&input, Some(media_dir)) {
        Ok(doc) => {
            for warning in quillon_core::subset_warnings(&input) {
                eprintln!("warning: {warning}");
            }
            let base = Path::new(media_dir);
            let errors = quillon_core::validate_document(&doc, base);
            if !errors.is_empty() {
                for error in &errors {
                    eprintln!("{error}");
                }
                eprintln!(
                    "the imported timeline would be an invalid document; check the media paths"
                );
                return 2;
            }
            write_document(&doc, out)
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn caption_import(
    subs: &Path,
    doc_path: &Path,
    out: Option<PathBuf>,
    flags: &quillon_cli::caption_cmd::ImportFlags,
    no_regularize: bool,
) -> i32 {
    let doc = match read_document(doc_path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    match quillon_cli::caption_cmd::import(subs, &doc, flags, no_regularize) {
        Ok(captioned) => {
            // The result must itself validate — a flag could carry an
            // invalid size, say — and this document is what gets rendered.
            let base = doc_path.parent().unwrap_or(Path::new("."));
            let errors = quillon_core::validate_document(&captioned, base);
            if !errors.is_empty() {
                for error in &errors {
                    eprintln!("{error}");
                }
                eprintln!(
                    "the captioned document would be invalid; fix the flags or the subtitle file"
                );
                return 2;
            }
            write_document(&captioned, out)
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

/// `quillon caption transcribe` — whisper through the same attach tail as
/// import, so regularized cues and validation are identical (feature
/// `transcribe` only).
#[cfg(feature = "transcribe")]
fn caption_transcribe(
    audio: &Path,
    doc_path: &Path,
    out: Option<PathBuf>,
    model: Option<&Path>,
    threads: u32,
    flags: &quillon_cli::caption_cmd::ImportFlags,
    no_regularize: bool,
) -> i32 {
    let doc = match read_document(doc_path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    match quillon_cli::caption_cmd::transcribe(audio, &doc, flags, model, threads, no_regularize) {
        Ok(captioned) => {
            let base = doc_path.parent().unwrap_or(Path::new("."));
            let errors = quillon_core::validate_document(&captioned, base);
            if !errors.is_empty() {
                for error in &errors {
                    eprintln!("{error}");
                }
                eprintln!(
                    "the captioned document would be invalid; fix the flags or the audio file"
                );
                return 2;
            }
            write_document(&captioned, out)
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn caption_export(
    doc_path: &Path,
    out: Option<PathBuf>,
    track: usize,
    format: Option<String>,
) -> i32 {
    let doc = match read_document(doc_path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    let format = format.as_deref().map(str::to_string).unwrap_or_else(|| {
        out.as_ref()
            .and_then(|p| p.extension().and_then(|e| e.to_str()).map(str::to_string))
            .unwrap_or_else(|| "srt".to_string())
    });
    match quillon_cli::caption_cmd::export(&doc, track, &format) {
        Ok(subtitles) => match out {
            Some(path) => {
                // Same contract as render: an output path inside new
                // directories just works.
                if let Some(parent) = path.parent() {
                    if let Err(err) = std::fs::create_dir_all(parent) {
                        eprintln!("cannot create {}: {err}", parent.display());
                        return 1;
                    }
                }
                match std::fs::write(&path, &subtitles) {
                    Ok(()) => {
                        println!("wrote {}", path.display());
                        0
                    }
                    Err(err) => {
                        eprintln!("cannot write {}: {err}", path.display());
                        1
                    }
                }
            }
            None => {
                print!("{subtitles}");
                0
            }
        },
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn audio_analyze(path: &Path, json: bool, silence_db: f64, min_silence_ms: u64) -> i32 {
    let doc = match read_document(path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    let base = path.parent().unwrap_or(Path::new("."));
    match quillon_cli::audio_cmd::analyze(&doc, base, silence_db, min_silence_ms) {
        Ok(reports) => {
            if json {
                let body =
                    quillon_cli::audio_cmd::json_body(&doc, &reports, silence_db, min_silence_ms);
                println!("{body}");
            } else {
                for line in quillon_cli::audio_cmd::text_lines(&reports, doc.timebase) {
                    println!("{line}");
                }
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

/// Serialize a document pretty (human-editable) to a file or stdout.
fn write_document(doc: &Document, out: Option<PathBuf>) -> i32 {
    let body = serde_json::to_string_pretty(doc).unwrap_or_else(|e| {
        eprintln!("cannot serialize the document: {e}");
        std::process::exit(1);
    });
    match out {
        Some(path) => {
            // Same contract as render: an output path inside new
            // directories just works.
            if let Some(parent) = path.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    eprintln!("cannot create {}: {err}", parent.display());
                    return 1;
                }
            }
            match std::fs::write(&path, body + "\n") {
                Ok(()) => {
                    println!("wrote {}", path.display());
                    0
                }
                Err(err) => {
                    eprintln!("cannot write {}: {err}", path.display());
                    1
                }
            }
        }
        None => {
            println!("{body}");
            0
        }
    }
}
