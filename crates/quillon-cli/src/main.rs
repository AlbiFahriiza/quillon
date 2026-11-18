//! Quillon CLI (plan §4 F10): `validate` and `render`, machine-readable via
//! `--json`, semantic exit codes:
//!   0 success · 1 operational error · 2 invalid document · 3 lint findings.
//! Usage errors exit 1 (clap's default 2 is reserved for document problems).

use clap::{Parser, Subcommand, ValueEnum};
use quillon_core::frame_source::NullFrameSource;
use quillon_core::time::frame_tick;
use quillon_core::{Document, lint_document, validate_str};
use quillon_media::Mp4Sink;
use quillon_render::{RENDERER_VERSION, Renderer};
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
    /// Render a document to MP4 (system ffmpeg) or PNG frames.
    Render {
        doc: PathBuf,
        /// Output: a .mp4 file, or a directory for PNG frames.
        #[arg(short, long)]
        out: PathBuf,
        /// Output container: `mp4` (default) or `png`.
        #[arg(long, default_value = "mp4")]
        format: Format,
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
            json,
        } => render(&doc, &out, format, json),
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

fn render(path: &Path, out: &Path, format: Format, json: bool) -> i32 {
    let doc = match read_document(path) {
        Ok(doc) => doc,
        Err(code) => return code,
    };
    let base = path.parent().unwrap_or(Path::new("."));
    let renderer = match Renderer::new(&doc, base) {
        Ok(renderer) => renderer,
        Err(err) => {
            eprintln!("cannot prepare the render: {err}");
            return 1;
        }
    };

    let mut frames = Vec::new();
    for i in 0..doc.frame_count() {
        let tick = (i as i64) * frame_tick(doc.fps);
        let frame = match renderer.render_tick(tick, &NullFrameSource) {
            Ok(frame) => frame,
            Err(err) => {
                eprintln!("render failed at frame {i}: {err}");
                return 1;
            }
        };
        frames.push(frame);
    }

    let manifest_path = match format {
        Format::Mp4 => write_mp4(&doc, out, &frames),
        Format::Png => write_pngs(&doc, out, &frames),
    };
    let manifest_path = match manifest_path {
        Ok(path) => path,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };

    let frame_count = frames.len();
    if json {
        let body = json!({
            "output": out,
            "manifest": manifest_path,
            "frames": frame_count,
        });
        println!("{body}");
    } else {
        println!("rendered {frame_count} frames -> {}", out.display());
        println!("frame manifest: {}", manifest_path.display());
    }
    0
}

fn write_mp4(
    doc: &Document,
    out: &Path,
    frames: &[quillon_core::Frame],
) -> Result<PathBuf, String> {
    let mut sink = Mp4Sink::new(out, doc.size.w, doc.size.h, doc.fps)
        .map_err(|err| format!("cannot start the encoder: {err}"))?;
    for frame in frames {
        sink.write_frame(frame)
            .map_err(|err| format!("encoding stopped: {err}"))?;
    }
    sink.finish()
        .map_err(|err| format!("encoding failed: {err}"))?;
    let manifest_path = manifest_path_for(out);
    write_manifest(doc.size.w, doc.size.h, &manifest_path, frames).map_err(|e| e.to_string())?;
    Ok(manifest_path)
}

fn write_pngs(
    doc: &Document,
    out: &Path,
    frames: &[quillon_core::Frame],
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    for (i, frame) in frames.iter().enumerate() {
        let path = out.join(format!("frame_{i:06}.png"));
        save_png(frame, &path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let manifest_path = out.join("manifest.json");
    write_manifest(doc.size.w, doc.size.h, &manifest_path, frames).map_err(|e| e.to_string())?;
    Ok(manifest_path)
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

fn write_manifest(
    width: u32,
    height: u32,
    path: &Path,
    frames: &[quillon_core::Frame],
) -> std::io::Result<()> {
    let hashes: Vec<_> = frames.iter().map(quillon_render_frame_hash).collect();
    let body = json!({
        "renderer_version": RENDERER_VERSION,
        "width": width,
        "height": height,
        "frame_count": frames.len(),
        "frames": hashes,
    });
    std::fs::write(path, serde_json::to_string_pretty(&body)? + "\n")
}

fn quillon_render_frame_hash(frame: &quillon_core::Frame) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&frame.rgba);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
