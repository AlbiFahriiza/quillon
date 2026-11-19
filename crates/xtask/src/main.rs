//! Repo automation (matklad's xtask pattern; plan §6.1, research/L).
//!
//!   cargo xtask export-schema   # regenerate docs/schema.json
//!   cargo xtask golden          # regenerate testdata/golden/hashes.json
//!   cargo xtask gallery         # regenerate docs/gallery-assets/*.png
//!
//! Golden regeneration is a deliberate act: the result belongs in a commit
//! that contains nothing else and states why pixels shifted (plan §14).

use anyhow::{Context, Result, bail};
use quillon_core::frame_source::NullFrameSource;
use quillon_core::time::frame_tick;
use quillon_core::{Document, validate_str};
use quillon_render::{RENDERER_VERSION, Renderer};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    let command = std::env::args().nth(1);
    let result = match command.as_deref() {
        Some("export-schema") => export_schema(),
        Some("golden") => golden(),
        Some("gallery") => gallery(),
        _ => {
            eprintln!("usage: cargo xtask <export-schema|golden|gallery>");
            std::process::exit(1);
        }
    };
    if let Err(err) = result {
        eprintln!("xtask: {err:#}");
        std::process::exit(1);
    }
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/xtask; the workspace root is two up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root exists")
}

fn export_schema() -> Result<()> {
    let out = repo_root().join("docs/schema.json");
    let schema = quillon_core::schema::document_schema();
    let json = serde_json::to_string_pretty(schema).context("serialize schema")?;
    std::fs::write(&out, json + "\n").with_context(|| format!("write {}", out.display()))?;
    println!("wrote {}", out.display());
    Ok(())
}

fn golden() -> Result<()> {
    let corpus_dir = repo_root().join("crates/quillon-render/testdata/corpus");
    let out_path = repo_root().join("crates/quillon-render/testdata/golden/hashes.json");
    std::fs::create_dir_all(out_path.parent().unwrap())?;

    let mut corpus: Vec<PathBuf> = std::fs::read_dir(&corpus_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    corpus.sort();
    if corpus.is_empty() {
        bail!("no corpus documents in {}", corpus_dir.display());
    }

    let mut golden = serde_json::Map::new();
    for path in &corpus {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("corpus file name is not UTF-8")?
            .to_string();
        let doc = load_doc(path, &corpus_dir)?;
        let hashes = render_hashes(&doc, &corpus_dir)?;
        println!("{}: {} frames", name, hashes.len());
        golden.insert(
            name,
            serde_json::json!({ "renderer_version": RENDERER_VERSION, "frames": hashes }),
        );
    }

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(golden))?;
    std::fs::write(&out_path, json + "\n")
        .with_context(|| format!("write {}", out_path.display()))?;
    println!("wrote {}", out_path.display());
    Ok(())
}

fn load_doc(path: &Path, base: &Path) -> Result<Document> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    validate_str(&text, base)
        .map_err(|errors| anyhow::anyhow!("corpus doc {} is invalid: {errors:?}", path.display()))
}

/// Regenerate the gallery images: one mid-animation frame per template in
/// `templates/`, rendered through the same Renderer the real render path
/// uses, saved to `docs/gallery-assets/<name>.png`. What you see in
/// `docs/gallery.md` is exactly what this produces from the committed
/// documents — never a hand-edited picture.
fn gallery() -> Result<()> {
    let templates_dir = repo_root().join("templates");
    let out_dir = repo_root().join("docs/gallery-assets");
    std::fs::create_dir_all(&out_dir)?;

    let mut templates: Vec<PathBuf> = std::fs::read_dir(&templates_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    templates.sort();
    if templates.is_empty() {
        bail!("no templates in {}", templates_dir.display());
    }

    for path in &templates {
        let name = path
            .file_stem()
            .and_then(|n| n.to_str())
            .context("template file name is not UTF-8")?
            .to_string();
        // Assets are relative to the templates directory.
        let doc = load_doc(path, &templates_dir)?;
        let renderer = Renderer::new(&doc, &templates_dir)?;
        let frames = doc.frame_count();
        // The mid frame: animations have started, nothing has ended yet.
        let mid = frames / 2;
        let tick = (mid as i64) * frame_tick(doc.fps);
        let frame = renderer.render_tick(tick, &NullFrameSource)?;
        let out = out_dir.join(format!("{name}.png"));
        quillon_cli::render_doc::save_png(&frame, &out)?;
        println!("{name}: frame {mid}/{} -> {}", frames, out.display());
    }
    Ok(())
}

fn render_hashes(doc: &Document, base: &Path) -> Result<Vec<String>> {
    let renderer = Renderer::new(doc, base)?;
    let mut hashes = Vec::with_capacity(doc.frame_count() as usize);
    for i in 0..doc.frame_count() {
        let tick = (i as i64) * frame_tick(doc.fps);
        let frame = renderer.render_tick(tick, &NullFrameSource)?;
        let digest = Sha256::digest(&frame.rgba);
        hashes.push(digest.iter().map(|b| format!("{b:02x}")).collect());
    }
    Ok(hashes)
}
