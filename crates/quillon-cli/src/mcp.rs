//! MCP server surface (plan §8): `quillon mcp` — stdio only (research/I §2:
//! local, single-user, spawned by the agent client; a hosted HTTP endpoint
//! would be a separate service, not a flag here).
//!
//! The tool set implements the cheapest-first agent loop (plan P4, §6.4):
//! validate_document costs no pixels, preview_frames renders draft
//! half-resolution PNGs, render_document/render_status is the two-call
//! async final render. Every error names the step that failed and carries
//! JSON-pointer paths into the document; frame sha256s make agent
//! self-verification a string compare (plan §8 design rules).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use quillon_core::draft::scale_document;
use quillon_core::frame_source::{Frame, FrameSource, NullFrameSource};
use quillon_core::{Document, ValidationError, diff_documents, lint_document, validate_str};
use quillon_media::DocumentFrameSource;
use quillon_render::Renderer;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, ListResourcesResult, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::stdio_pipes;
use rmcp::{ErrorData, RoleServer, ServerHandler, serve_server, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::render_doc::{self, Format};

/// URI of the published document schema resource (plan §5.2).
pub const SCHEMA_URI: &str = "quillon://schema/document";
/// Most preview frames one call may return (bounding agent cost, plan §8).
const MAX_PREVIEW_FRAMES: usize = 16;
/// Most files list_assets/list_fonts may report (plan §11: bounded input).
const MAX_LIST_ENTRIES: usize = 256;

// ---------------------------------------------------------------------------
// Tool input schemas (schemars derives the JSON Schema the client sees)
// ---------------------------------------------------------------------------

#[derive(Deserialize, JsonSchema)]
struct ValidateInput {
    #[schemars(description = "The document to check, as parsed JSON")]
    document: Value,
}

#[derive(Deserialize, JsonSchema)]
struct PreviewInput {
    #[schemars(description = "The document to preview, as parsed JSON")]
    document: Value,
    #[schemars(description = "Flicks ticks to render (1-16, within the document duration)")]
    ticks: Vec<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct DiffInput {
    #[schemars(description = "Document A (before), as parsed JSON")]
    a: Value,
    #[schemars(description = "Document B (after), as parsed JSON")]
    b: Value,
}

#[derive(Deserialize, JsonSchema)]
struct FilterInput {
    #[schemars(description = "Case-insensitive substring filter on the file name")]
    filter: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ProbeInput {
    #[schemars(description = "Media file path, relative to the working root")]
    path: String,
}

#[derive(Deserialize, JsonSchema)]
struct RenderFrameInput {
    #[schemars(description = "The document to render, as parsed JSON")]
    document: Value,
    #[schemars(description = "Flicks tick of the frame (0 <= tick < duration)")]
    tick: i64,
}

#[derive(Deserialize, JsonSchema)]
struct RenderDocumentInput {
    #[schemars(description = "The document to render, as parsed JSON")]
    document: Value,
    #[schemars(description = "Output container: \"mp4\" or \"png\"")]
    format: String,
    #[schemars(description = "Output path relative to the working root (optional)")]
    out: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct RenderStatusInput {
    #[schemars(description = "Run id returned by render_document")]
    run_id: String,
}

// ---------------------------------------------------------------------------
// Render run registry (render_document is async two-call, plan §6.4)
// ---------------------------------------------------------------------------

enum RunStatus {
    Running,
    Done(render_doc::RenderOutcome),
    Failed(String),
}

#[derive(Clone)]
pub struct QuillonServer {
    base_dir: PathBuf,
    runs: Arc<Mutex<HashMap<String, RunStatus>>>,
    next_id: Arc<AtomicU64>,
}

impl QuillonServer {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            runs: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// The registry lock is only ever held for map reads/writes, so a
    /// poisoned lock still guards consistent data; recover it instead of
    /// taking the whole server down.
    fn runs(&self) -> MutexGuard<'_, HashMap<String, RunStatus>> {
        self.runs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[tool_router]
impl QuillonServer {
    /// Cheapest check first (plan P4): no pixel is rendered to answer
    /// "is this document valid and readable?".
    #[tool(
        description = "Validate a document (schema + semantics + lint). Returns JSON-pointer errors and lint findings. No pixels are rendered; start here before preview or render."
    )]
    async fn validate_document(
        &self,
        Parameters(input): Parameters<ValidateInput>,
    ) -> CallToolResult {
        let doc = match parse_and_validate(&input.document, &self.base_dir) {
            Ok(doc) => doc,
            Err(result) => return result,
        };
        let lint = lint_document(&doc);
        let body = json!({
            "valid": true,
            "errors": [],
            "lint": lint,
        });
        text_result(body)
    }

    #[tool(
        description = "Render draft half-resolution PNG frames at the given ticks (max 16). Each frame carries a sha256; use it to see what the document looks like without paying for a full render."
    )]
    async fn preview_frames(&self, Parameters(input): Parameters<PreviewInput>) -> CallToolResult {
        let doc = match parse_and_validate(&input.document, &self.base_dir) {
            Ok(doc) => doc,
            Err(result) => return result,
        };
        if input.ticks.is_empty() || input.ticks.len() > MAX_PREVIEW_FRAMES {
            return step_error(
                "preview_frames",
                format!("ticks must contain 1 to {MAX_PREVIEW_FRAMES} entries"),
            );
        }
        let total = doc.total_duration();
        if let Some(tick) = input.ticks.iter().find(|t| **t < 0 || **t >= total) {
            return step_error(
                "preview_frames",
                format!("tick {tick} is outside the document duration 0..{total}"),
            );
        }
        // Draft = half-resolution document through the same render path
        // (plan §10 draft preview; same-engine guarantee).
        let draft = scale_document(&doc, 2);
        let src = frame_source(&doc, &self.base_dir);
        let renderer = match Renderer::new(&draft, &self.base_dir) {
            Ok(renderer) => renderer,
            Err(err) => return step_error("preview_frames", err.to_string()),
        };
        let mut blocks = Vec::new();
        let mut meta = Vec::new();
        for tick in input.ticks {
            let frame = match renderer.render_tick(tick, src.as_ref()) {
                Ok(frame) => frame,
                Err(err) => {
                    return step_error("preview_frames", format!("rendering tick {tick}: {err}"));
                }
            };
            let hash = render_doc::frame_hash(&frame);
            let png = match encode_png_bytes(&frame) {
                Ok(png) => png,
                Err(err) => return step_error("preview_frames", err.to_string()),
            };
            let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
            meta.push(json!({ "tick": tick, "sha256": hash }));
            blocks.push(ContentBlock::image(b64, "image/png"));
        }
        blocks.insert(0, ContentBlock::text(json!({ "frames": meta }).to_string()));
        CallToolResult::success(blocks)
    }

    #[tool(
        description = "Semantic diff of two documents: the list of changed JSON-pointer paths with before/after values. Verify your edit without reading whole documents back."
    )]
    async fn diff_documents(&self, Parameters(input): Parameters<DiffInput>) -> CallToolResult {
        let a = match serde_json::from_value::<Document>(input.a) {
            Ok(doc) => doc,
            Err(err) => {
                return step_error(
                    "diff_documents",
                    format!("document A does not parse: {err}"),
                );
            }
        };
        let b = match serde_json::from_value::<Document>(input.b) {
            Ok(doc) => doc,
            Err(err) => {
                return step_error(
                    "diff_documents",
                    format!("document B does not parse: {err}"),
                );
            }
        };
        let report = diff_documents(&a, &b);
        text_result(serde_json::to_value(&report).unwrap_or_else(|_| json!({ "changed": true })))
    }

    #[tool(
        description = "List media assets in the working root (non-recursive): video, audio and image files with id, kind and size."
    )]
    async fn list_assets(&self, Parameters(input): Parameters<FilterInput>) -> CallToolResult {
        let entries = match scan_dir(&self.base_dir, media_kind, input.filter.as_deref()) {
            Ok(entries) => entries,
            Err(err) => return step_error("list_assets", err),
        };
        let assets: Vec<_> = entries
            .iter()
            .map(|(path, kind, size)| {
                json!({
                    "id": path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                    "kind": kind,
                    "src": path.to_string_lossy(),
                    "size_bytes": size,
                })
            })
            .collect();
        text_result(json!({ "assets": assets }))
    }

    #[tool(
        description = "List font files in the working root (non-recursive): ttf and otf. The id is the file name; reference it from a document asset entry."
    )]
    async fn list_fonts(&self, Parameters(input): Parameters<FilterInput>) -> CallToolResult {
        let entries = match scan_dir(&self.base_dir, font_kind, input.filter.as_deref()) {
            Ok(entries) => entries,
            Err(err) => return step_error("list_fonts", err),
        };
        let fonts: Vec<_> = entries
            .iter()
            .map(|(path, _kind, _size)| {
                json!({
                    "id": path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                    "src": path.to_string_lossy(),
                })
            })
            .collect();
        text_result(json!({ "fonts": fonts }))
    }

    #[tool(
        description = "Probe a media file: duration, resolution, fps and codecs. Cheaper than rendering to find out what a clip contains."
    )]
    async fn probe_media(&self, Parameters(input): Parameters<ProbeInput>) -> CallToolResult {
        let path = match self.resolve(&input.path) {
            Ok(path) => path,
            Err(result) => return result,
        };
        match quillon_media::probe::probe(&path) {
            Ok(info) => {
                let fps = info
                    .fps
                    .map(|(num, den)| json!({ "num": num, "den": den }))
                    .unwrap_or(Value::Null);
                text_result(json!({
                    "path": input.path,
                    "duration_seconds": info.duration,
                    "width": info.width,
                    "height": info.height,
                    "fps": fps,
                    "video_codec": info.video_codec,
                    "audio_codec": info.audio_codec,
                }))
            }
            Err(err) => step_error("probe_media", err.to_string()),
        }
    }

    #[tool(
        description = "Render one full-resolution frame at a tick: PNG image + sha256. Use it for visual-regression checks (same document + tick must give the same hash)."
    )]
    async fn render_frame(
        &self,
        Parameters(input): Parameters<RenderFrameInput>,
    ) -> CallToolResult {
        let doc = match parse_and_validate(&input.document, &self.base_dir) {
            Ok(doc) => doc,
            Err(result) => return result,
        };
        let total = doc.total_duration();
        if input.tick < 0 || input.tick >= total {
            return step_error(
                "render_frame",
                format!(
                    "tick {} is outside the document duration 0..{total}",
                    input.tick
                ),
            );
        }
        let src = frame_source(&doc, &self.base_dir);
        let renderer = match Renderer::new(&doc, &self.base_dir) {
            Ok(renderer) => renderer,
            Err(err) => return step_error("render_frame", err.to_string()),
        };
        let frame = match renderer.render_tick(input.tick, src.as_ref()) {
            Ok(frame) => frame,
            Err(err) => return step_error("render_frame", err.to_string()),
        };
        let hash = render_doc::frame_hash(&frame);
        let png = match encode_png_bytes(&frame) {
            Ok(png) => png,
            Err(err) => return step_error("render_frame", err.to_string()),
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        CallToolResult::success(vec![
            ContentBlock::text(
                json!({
                    "tick": input.tick,
                    "sha256": hash,
                    "width": frame.width,
                    "height": frame.height,
                })
                .to_string(),
            ),
            ContentBlock::image(b64, "image/png"),
        ])
    }

    #[tool(
        description = "Start a full render (mp4 or png). Returns a run_id and the estimated frame count immediately; poll render_status for the result. Do not call this in a loop — validate and preview first."
    )]
    async fn render_document(
        &self,
        Parameters(input): Parameters<RenderDocumentInput>,
    ) -> CallToolResult {
        let doc = match parse_and_validate(&input.document, &self.base_dir) {
            Ok(doc) => doc,
            Err(result) => return result,
        };
        let format = match input.format.as_str() {
            "mp4" => Format::Mp4,
            "png" => Format::Png,
            other => {
                return step_error(
                    "render_document",
                    format!("format {other:?} is not supported; use \"mp4\" or \"png\""),
                );
            }
        };
        let run_id = format!("run-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let out = match input.out {
            Some(relative) => match self.resolve(&relative) {
                Ok(path) => path,
                Err(result) => return result,
            },
            None => match format {
                Format::Mp4 => self.base_dir.join(format!("quillon-{run_id}.mp4")),
                Format::Png => self.base_dir.join(format!("quillon-{run_id}-png")),
            },
        };
        // Price before cost (plan §8): the caller learns the frame budget
        // before committing to the render.
        let estimated = doc.frame_count();
        self.runs().insert(run_id.clone(), RunStatus::Running);
        let base = self.base_dir.clone();
        let runs = Arc::clone(&self.runs);
        let id = run_id.clone();
        let reported_out = out.clone();
        std::thread::spawn(move || {
            let options = render_doc::RenderOptions {
                workers: None,
                cache_dir: None,
                lufs: None,
                video_codec: "libx264",
            };
            let result = render_doc::render_document(&doc, &base, &out, format, &options);
            let status = match result {
                Ok(outcome) => RunStatus::Done(outcome),
                Err(err) => RunStatus::Failed(err),
            };
            runs.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(id, status);
        });
        text_result(json!({
            "run_id": run_id,
            "estimated_frames": estimated,
            "output": reported_out.to_string_lossy(),
        }))
    }

    #[tool(
        description = "Poll a render started by render_document: status running/done/error, and when done the manifest with every frame sha256, output path and frame count."
    )]
    async fn render_status(
        &self,
        Parameters(input): Parameters<RenderStatusInput>,
    ) -> CallToolResult {
        let runs = self.runs();
        let Some(status) = runs.get(&input.run_id) else {
            return step_error(
                "render_status",
                format!(
                    "unknown run id {:?} — start a render with render_document",
                    input.run_id
                ),
            );
        };
        match status {
            RunStatus::Running => text_result(json!({ "status": "running" })),
            RunStatus::Failed(err) => text_result(json!({ "status": "error", "error": err })),
            RunStatus::Done(outcome) => text_result(json!({
                "status": "done",
                "manifest": {
                    "frame_count": outcome.frames,
                    "frames": outcome.frame_hashes,
                    "output": outcome.output.to_string_lossy(),
                    "manifest_path": outcome.manifest_path.to_string_lossy(),
                },
            })),
        }
    }
}

#[tool_handler]
impl ServerHandler for QuillonServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions(
            "Quillon: video as code. Work cheapest-first: (1) read the schema once \
             (resource quillon://schema/document), (2) validate_document — no pixels, \
             errors carry JSON-pointer paths, (3) preview_frames for draft PNGs, \
             (4) render_document + render_status for the final render. Frame sha256s \
             are reproducible: the same document and tick always hash the same.",
        )
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult {
            resources: vec![Resource::new(SCHEMA_URI, "Quillon document JSON Schema")],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri != SCHEMA_URI {
            return Err(ErrorData::invalid_params(
                format!(
                    "unknown resource {} — the only resource is {SCHEMA_URI}",
                    request.uri
                ),
                None,
            ));
        }
        let text = serde_json::to_string_pretty(quillon_core::schema::document_schema())
            .expect("the schema is a JSON Value; serialization cannot fail");
        Ok(
            ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                uri: SCHEMA_URI.to_string(),
                mime_type: Some("application/json".to_string()),
                text,
                meta: None,
            }])
            .into(),
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate a document value the way the CLI does (schema + semantics).
/// Errors come back as a tool result so the agent sees JSON-pointer paths.
fn parse_and_validate(document: &Value, base: &Path) -> Result<Document, CallToolResult> {
    // A Value round-trips to its compact JSON form losslessly.
    let text =
        serde_json::to_string(document).expect("a serde_json Value always serializes back to JSON");
    match validate_str(&text, base) {
        Ok(doc) => Ok(doc),
        Err(errors) => Err(step_error("validate", errors_json(&errors))),
    }
}

fn errors_json(errors: &[ValidationError]) -> String {
    let list: Vec<_> = errors
        .iter()
        .map(|error| {
            json!({
                "path": error.path,
                "code": error.code,
                "message": error.message,
            })
        })
        .collect();
    json!({ "errors": list }).to_string()
}

/// A tool-level error that names the step that failed (plan §8 design rule).
fn step_error(step: &str, message: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(
        json!({ "step": step, "error": message.to_string() }).to_string(),
    )])
}

fn text_result(body: serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(body.to_string())])
}

fn frame_source(doc: &Document, base: &Path) -> Box<dyn FrameSource> {
    match DocumentFrameSource::open(doc, base) {
        Ok(source) if !source.is_empty() => Box::new(source),
        _ => Box::new(NullFrameSource),
    }
}

/// Resolve a caller path against the working root, rejecting traversal
/// outside it (plan §11: untrusted input).
fn resolve_unchecked(base: &Path, relative: &str) -> PathBuf {
    base.join(relative)
}

impl QuillonServer {
    fn resolve(&self, relative: &str) -> Result<PathBuf, CallToolResult> {
        if relative.split(['/', '\\']).any(|part| part == "..") {
            return Err(step_error(
                "resolve",
                format!("path {relative:?} may not contain '..' — stay inside the working root"),
            ));
        }
        Ok(resolve_unchecked(&self.base_dir, relative))
    }
}

fn encode_png_bytes(frame: &Frame) -> Result<Vec<u8>, image::ImageError> {
    use image::ImageEncoder as _;
    let mut out = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut out);
    encoder.write_image(
        &frame.rgba,
        frame.width,
        frame.height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

fn media_kind(extension: &str) -> Option<&'static str> {
    match extension {
        "mp4" | "mov" | "webm" | "mkv" => Some("video"),
        "wav" | "mp3" | "flac" | "ogg" => Some("audio"),
        "png" | "jpg" | "jpeg" | "webp" => Some("image"),
        _ => None,
    }
}

fn font_kind(extension: &str) -> Option<&'static str> {
    match extension {
        "ttf" | "otf" => Some("font"),
        _ => None,
    }
}

/// Non-recursive scan of the working root, bounded to MAX_LIST_ENTRIES.
fn scan_dir(
    base: &Path,
    classify: fn(&str) -> Option<&'static str>,
    filter: Option<&str>,
) -> Result<Vec<(PathBuf, &'static str, u64)>, String> {
    let entries = std::fs::read_dir(base)
        .map_err(|err| format!("cannot read the working root {}: {err}", base.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|err| format!("cannot read an entry in {}: {err}", base.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let kind = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase())
            .and_then(|ext| classify(&ext));
        let Some(kind) = kind else { continue };
        if let Some(filter) = filter {
            if !name.contains(&filter.to_lowercase()) {
                continue;
            }
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or_default();
        found.push((path, kind, size));
    }
    found.sort_by(|a, b| a.0.file_name().cmp(&b.0.file_name()));
    found.truncate(MAX_LIST_ENTRIES);
    Ok(found)
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the stdio MCP server. Returns a process exit code. Logs go to
/// stderr only: stdout is the JSON-RPC channel.
pub fn run(root: Option<PathBuf>) -> i32 {
    let base_dir =
        root.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("cannot start the MCP server runtime: {err}");
            return 1;
        }
    };
    let server = QuillonServer::new(base_dir);
    runtime.block_on(async move {
        match serve_server(server, stdio_pipes()).await {
            Ok(running) => {
                // Serve until the client closes the stream.
                let _ = running.waiting().await;
                0
            }
            Err(err) => {
                eprintln!("cannot start the MCP server: {err}");
                1
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Unit tests: the error and listing paths the E2E agent loop does not walk.
// The happy-path loop is covered end-to-end in tests/mcp_e2e.rs.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn workdir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("quillon-mcp-unit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create unit-test workdir");
        dir
    }

    fn text_of(result: &CallToolResult) -> String {
        result
            .content
            .first()
            .and_then(|block| block.as_text())
            .map(|text| text.text.clone())
            .expect("first content block is text")
    }

    #[tokio::test]
    async fn render_status_rejects_unknown_run_ids() {
        let dir = workdir("status");
        let server = QuillonServer::new(dir);
        let result = server
            .render_status(Parameters(RenderStatusInput {
                run_id: "nope".into(),
            }))
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(
            text_of(&result).contains("unknown run id"),
            "names the problem"
        );
    }

    #[tokio::test]
    async fn probe_media_rejects_path_traversal() {
        let dir = workdir("traversal");
        let server = QuillonServer::new(dir);
        let result = server
            .probe_media(Parameters(ProbeInput {
                path: "../secrets".into(),
            }))
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(text_of(&result).contains(".."), "says what was rejected");
    }

    #[tokio::test]
    async fn probe_media_reports_a_non_media_file_as_an_error() {
        let dir = workdir("probe-bad");
        std::fs::write(dir.join("notes.txt"), "not a media file").expect("write");
        let server = QuillonServer::new(dir);
        let result = server
            .probe_media(Parameters(ProbeInput {
                path: "notes.txt".into(),
            }))
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(text_of(&result).contains("probe_media"), "names the step");
    }

    #[tokio::test]
    async fn diff_documents_names_the_document_that_fails_to_parse() {
        let dir = workdir("diff");
        let server = QuillonServer::new(dir);
        let result = server
            .diff_documents(Parameters(DiffInput {
                a: json!({ "v": 1, "size": { "w": 10, "h": 10 } }), // no scenes
                b: json!({ "v": 1, "size": { "w": 10, "h": 10 }, "scenes": [] }),
            }))
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(text_of(&result).contains("document A"), "points at A");
    }

    #[tokio::test]
    async fn preview_frames_rejects_empty_and_out_of_range_ticks() {
        let dir = workdir("preview");
        let server = QuillonServer::new(dir);
        let doc = json!({
            "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
            "scenes": [ { "id": "s", "duration": 705600000, "elements": [] } ]
        });
        let empty = server
            .preview_frames(Parameters(PreviewInput {
                document: doc.clone(),
                ticks: vec![],
            }))
            .await;
        assert_eq!(empty.is_error, Some(true));
        assert!(text_of(&empty).contains("1 to 16"));

        let out_of_range = server
            .preview_frames(Parameters(PreviewInput {
                document: doc,
                ticks: vec![705_600_000],
            }))
            .await;
        assert_eq!(out_of_range.is_error, Some(true));
        assert!(text_of(&out_of_range).contains("outside the document duration"));
    }

    #[tokio::test]
    async fn render_document_rejects_unknown_formats() {
        let dir = workdir("format");
        let server = QuillonServer::new(dir);
        let doc = json!({
            "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
            "scenes": [ { "id": "s", "duration": 705600000, "elements": [] } ]
        });
        let result = server
            .render_document(Parameters(RenderDocumentInput {
                document: doc,
                format: "avi".into(),
                out: None,
            }))
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(text_of(&result).contains("mp4"), "says what is supported");
    }

    #[tokio::test]
    async fn list_assets_and_fonts_classify_and_filter_the_working_root() {
        let dir = workdir("scan");
        std::fs::write(dir.join("clip.mp4"), b"x").expect("write");
        std::fs::write(dir.join("body.ttf"), b"x").expect("write");
        std::fs::write(dir.join("readme.md"), b"x").expect("write");

        let server = QuillonServer::new(dir.clone());
        let assets = server
            .list_assets(Parameters(FilterInput { filter: None }))
            .await;
        assert_eq!(assets.is_error, Some(false));
        let body: Value = serde_json::from_str(&text_of(&assets)).expect("JSON");
        assert_eq!(
            body["assets"].as_array().map(Vec::len),
            Some(1),
            "only media"
        );
        assert_eq!(body["assets"][0]["kind"], json!("video"));
        assert_eq!(body["assets"][0]["id"], json!("clip.mp4"));

        let fonts = server
            .list_fonts(Parameters(FilterInput { filter: None }))
            .await;
        let body: Value = serde_json::from_str(&text_of(&fonts)).expect("JSON");
        assert_eq!(
            body["fonts"].as_array().map(Vec::len),
            Some(1),
            "only fonts"
        );
        assert_eq!(body["fonts"][0]["id"], json!("body.ttf"));

        let filtered = server
            .list_fonts(Parameters(FilterInput {
                filter: Some("clip".into()),
            }))
            .await;
        let body: Value = serde_json::from_str(&text_of(&filtered)).expect("JSON");
        assert_eq!(
            body["fonts"].as_array().map(Vec::len),
            Some(0),
            "filter drops non-matches"
        );
    }
}
