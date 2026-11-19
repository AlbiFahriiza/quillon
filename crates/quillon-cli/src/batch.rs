//! `quillon render --data rows.csv` (plan §12 v0.6, design note D2): a
//! mail-merge batch over one shared frame-cache directory. Every row is
//! substituted, fully validated, and rendered; identical static scenes hit
//! the content-addressed cache instead of re-rendering — the acceptance
//! proof is the cache hit counter, not a promise.

use crate::csv;
use crate::render_doc::{Format, RenderOutcome, render_document};
use quillon_core::{Document, apply_template_json, fill_pattern, validate_str};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Everything `render --data` needs, bundled to respect the parameter
/// budget.
pub struct BatchArgs<'a> {
    pub template: &'a Path,
    pub data: &'a Path,
    /// Output pattern; `{{variable}}` markers fill per row.
    pub out_pattern: &'a str,
    pub format: Format,
    pub workers: Option<usize>,
    pub cache_dir: Option<&'a Path>,
    pub lufs: Option<f64>,
    pub json: bool,
}

pub struct BatchSummary {
    pub rows: usize,
    pub outputs: Vec<PathBuf>,
    pub hits: u64,
    pub misses: u64,
}

/// Run the batch. Errors print `row N:` context and return early; the
/// caller maps the error kind to an exit code.
pub fn render_batch(args: &BatchArgs) -> Result<BatchSummary, String> {
    let raw = std::fs::read_to_string(args.template)
        .map_err(|e| format!("cannot read {}: {e}", args.template.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|e| format!("{}: not valid JSON: {e}", args.template.display()))?;
    // The declared variables come from the template document itself.
    let base_doc: Document = serde_json::from_value(value.clone())
        .map_err(|e| format!("{}: not a Quillon document: {e}", args.template.display()))?;
    let rows = csv::parse_rows(
        &std::fs::read_to_string(args.data)
            .map_err(|e| format!("cannot read {}: {e}", args.data.display()))?,
    )?;

    let base = args
        .template
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let mut outputs = Vec::with_capacity(rows.len());
    let mut hits = 0u64;
    let mut misses = 0u64;
    for (i, row) in rows.iter().enumerate() {
        let filled = apply_template_json(&value, &base_doc.variables, row)
            .map_err(|e| format!("row {}: {}", i + 2, e))?;
        let filled_text = serde_json::to_string(&filled).map_err(|e| {
            format!(
                "row {}: cannot re-serialize the filled document: {e}",
                i + 2
            )
        })?;
        let doc = validate_str(&filled_text, &base).map_err(|errors| {
            format!(
                "row {}: the filled document is invalid:\n{}",
                i + 2,
                errors
                    .iter()
                    .map(|e| format!("  {e}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })?;
        let out_text = fill_pattern(args.out_pattern, &base_doc.variables, row)
            .map_err(|e| format!("row {}: the output pattern is bad: {e}", i + 2))?;
        let out = PathBuf::from(out_text);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("row {}: cannot create {}: {e}", i + 2, parent.display()))?;
        }
        let outcome: RenderOutcome = render_document(
            &doc,
            &base,
            &out,
            args.format,
            args.workers,
            args.cache_dir,
            args.lufs,
        )
        .map_err(|e| format!("row {}: render failed: {e}", i + 2))?;
        if let Some((h, m)) = outcome.cache_stats {
            hits += h;
            misses += m;
        }
        outputs.push(outcome.output);
    }
    Ok(BatchSummary {
        rows: rows.len(),
        outputs,
        hits,
        misses,
    })
}

/// The `--json` body of a finished batch.
pub fn summary_json(summary: &BatchSummary) -> serde_json::Value {
    serde_json::json!({
        "rows": summary.rows,
        "outputs": summary.outputs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "cache": { "hits": summary.hits, "misses": summary.misses },
    })
}

/// Row maps are also built by tests directly.
pub fn row(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
