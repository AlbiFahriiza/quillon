//! E2E acceptance tests for the MCP agent surface (plan §12 v0.4), driving
//! the real `quillon mcp` binary as an MCP client over stdio:
//!
//! 1. the scripted agent loop: invalid document → actionable JSON-pointer
//!    error → fixed → valid → 3 preview frames → final render via the
//!    two-call async API (start → poll);
//! 2. the served schema resource is identical to docs/schema.json;
//! 3. a validate response for a 10-scene document stays under 200 tokens
//!    (plan: cheapest-first, ~20-token checks).

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ReadResourceRequestParams};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Value, json};

/// Fresh working root per test; the server is spawned with `--root` so it
/// never depends on the caller's current directory.
fn workdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("quillon-mcp-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the working root");
    dir
}

async fn spawn_server(
    root: &std::path::Path,
) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let transport = TokioChildProcess::new(
        tokio::process::Command::new(env!("CARGO_BIN_EXE_quillon")).configure(|cmd| {
            cmd.arg("mcp").arg("--root").arg(root);
        }),
    )
    .expect("spawn `quillon mcp`");
    ().serve(transport)
        .await
        .expect("initialize the MCP session")
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool: &str,
    args: Value,
) -> rmcp::model::CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(tool.to_owned()).with_arguments(
                json!({ "document": args })
                    .as_object()
                    .expect("object")
                    .clone(),
            ),
        )
        .await
        .expect("tool call transports")
}

/// Call a tool with a raw arguments object (not wrapped in `document`).
async fn call_args(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool: &str,
    args: Value,
) -> rmcp::model::CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(tool.to_owned())
                .with_arguments(args.as_object().expect("object").clone()),
        )
        .await
        .expect("tool call transports")
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.clone())
        .expect("the first content block is text")
}

/// Two tiny scenes of plain rects, one second each (the lint read-time
/// floor is 0.8 s): 20 frames at 10 fps, no assets, no audio — a fast,
/// deterministic render target.
fn valid_document() -> Value {
    json!({
        "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
        "scenes": [
            { "id": "s1", "duration": 705600000,
              "elements": [ { "type": "rect", "x": 10, "y": 10, "w": 100, "h": 80,
                              "color": "#ffffff" } ] },
            { "id": "s2", "duration": 705600000,
              "elements": [ { "type": "rect", "x": 50, "y": 50, "w": 120, "h": 90,
                              "color": "#d0d0d0" } ] }
        ]
    })
}

/// Same shape but the element points at an asset that does not exist — the
/// classic agent mistake the validator must catch with a pointer.
fn invalid_document() -> Value {
    json!({
        "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
        "assets": { "ghost": { "type": "image", "src": "missing.png" } },
        "scenes": [
            { "id": "s1", "duration": 705600000,
              "elements": [ { "type": "image", "asset": "ghost", "x": 10, "y": 10,
                              "w": 100, "h": 80 } ] }
        ]
    })
}

#[tokio::test]
async fn agent_loop_invalid_to_final_render() {
    let dir = workdir("loop");
    let client = spawn_server(&dir).await;

    // The nine tools of the plan §8 surface are advertised.
    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<String> = tools.iter().map(|tool| tool.name.to_string()).collect();
    for expected in [
        "validate_document",
        "preview_frames",
        "diff_documents",
        "list_assets",
        "list_fonts",
        "probe_media",
        "render_frame",
        "render_document",
        "render_status",
    ] {
        assert!(
            names.iter().any(|name| name == expected),
            "tool {expected} must be advertised, got {names:?}"
        );
    }

    // 1. Invalid document: the response must be an actionable tool error
    //    naming the failing step and a JSON-pointer path (plan §8).
    let bad = call(&client, "validate_document", invalid_document()).await;
    assert_eq!(
        bad.is_error,
        Some(true),
        "invalid document must flag an error"
    );
    let body = text_of(&bad);
    assert!(body.contains("\"step\""), "error names its step: {body}");
    assert!(
        body.contains("/assets/ghost"),
        "error carries a pointer into the document: {body}"
    );
    assert!(
        body.contains("missing.png"),
        "error says what is missing: {body}"
    );

    // 2. Fixed document: valid and lint-clean.
    let good = call(&client, "validate_document", valid_document()).await;
    assert_eq!(good.is_error, Some(false));
    let verdict: Value = serde_json::from_str(&text_of(&good)).expect("validate returns JSON");
    assert_eq!(verdict["valid"], json!(true));
    assert_eq!(
        verdict["lint"],
        json!([]),
        "the sample document is lint-clean"
    );

    // 3. Preview three frames: one text block with the tick/sha256 list,
    //    then one image block per frame (plan §8: PNG + sha256).
    let preview = call_args(
        &client,
        "preview_frames",
        json!({ "document": valid_document(), "ticks": [0, 705600000, 1411199999] }),
    )
    .await;
    assert_eq!(preview.is_error, Some(false));
    assert_eq!(preview.content.len(), 4, "text meta + 3 images");
    let meta: Value = serde_json::from_str(&text_of(&preview)).expect("preview meta JSON");
    let frames = meta["frames"].as_array().expect("frames list");
    assert_eq!(frames.len(), 3);
    for frame in frames {
        assert_eq!(
            frame["sha256"].as_str().map(str::len),
            Some(64),
            "sha256 per frame"
        );
    }
    assert!(
        preview
            .content
            .iter()
            .any(|block| block.as_image().is_some())
    );

    // 4. Final render, two-call async (plan §6.4): start returns a run id
    //    and the price before the cost.
    let start = call_args(
        &client,
        "render_document",
        json!({ "document": valid_document(), "format": "mp4", "out": "final.mp4" }),
    )
    .await;
    assert_eq!(start.is_error, Some(false));
    let started: Value = serde_json::from_str(&text_of(&start)).expect("start JSON");
    let run_id = started["run_id"].as_str().expect("run id").to_string();
    assert_eq!(started["estimated_frames"], json!(20), "price before cost");

    // 5. Poll until done, then verify the manifest and the file on disk.
    let manifest = loop {
        let status = call_args(&client, "render_status", json!({ "run_id": run_id })).await;
        let body: Value = serde_json::from_str(&text_of(&status)).expect("status JSON");
        match body["status"].as_str() {
            Some("running") => tokio::time::sleep(std::time::Duration::from_millis(250)).await,
            Some("done") => break body["manifest"].clone(),
            other => panic!("render failed: {other:?} in {body}"),
        }
    };
    assert_eq!(manifest["frame_count"], json!(20));
    let output = dir.join("final.mp4");
    let size = std::fs::metadata(&output)
        .map(|meta| meta.len())
        .unwrap_or_default();
    assert!(size > 0, "the rendered MP4 exists and is not empty");

    // 6. Reproducible pixels (plan P5): render_frame at tick 0 matches the
    //    first manifest hash — agent self-verification is a string compare.
    let frame = call_args(
        &client,
        "render_frame",
        json!({ "document": valid_document(), "tick": 0 }),
    )
    .await;
    let frame_meta: Value = serde_json::from_str(&text_of(&frame)).expect("frame meta");
    assert_eq!(
        frame_meta["sha256"], manifest["frames"][0],
        "same document + tick must hash the same"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn served_schema_matches_published_schema() {
    let dir = workdir("schema");
    let client = spawn_server(&dir).await;

    let listed = client.list_all_resources().await.expect("list resources");
    assert!(
        listed
            .iter()
            .any(|resource| resource.uri == "quillon://schema/document"),
        "the schema resource is listed"
    );

    let read = client
        .read_resource(ReadResourceRequestParams::new("quillon://schema/document"))
        .await
        .expect("read the schema");
    let served_text = match read.contents.first() {
        Some(rmcp::model::ResourceContents::TextResourceContents { text, .. }) => text.clone(),
        other => panic!("expected text contents, got {other:?}"),
    };
    let served: Value = serde_json::from_str(&served_text).expect("served schema parses");
    let published_text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/schema.json"
    ))
    .expect("docs/schema.json exists in the repo checkout");
    let published: Value = serde_json::from_str(&published_text).expect("published schema parses");
    assert_eq!(
        served, published,
        "resource == docs/schema.json (plan parity)"
    );

    client.cancel().await.expect("clean shutdown");
}

#[tokio::test]
async fn validate_response_under_200_tokens_for_ten_scenes() {
    let dir = workdir("tokens");
    let client = spawn_server(&dir).await;

    // Ten lint-clean scenes: white rects on the default black background,
    // one second each.
    let scenes: Vec<_> = (0..10)
        .map(|index| {
            json!({
                "id": format!("s{index}"), "duration": 705600000,
                "elements": [ { "type": "rect", "x": 10, "y": 10, "w": 100, "h": 80,
                                "color": "#ffffff" } ]
            })
        })
        .collect();
    let document = json!({
        "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
        "scenes": scenes
    });

    let result = call(&client, "validate_document", document).await;
    assert_eq!(result.is_error, Some(false));
    let body = text_of(&result);
    let verdict: Value = serde_json::from_str(&body).expect("JSON");
    assert_eq!(verdict["valid"], json!(true));

    // Token approximation: ~4 characters per token, the standard rule of
    // thumb used for the plan's token budgets (research/C §7).
    let tokens = body.len() / 4;
    assert!(
        tokens < 200,
        "validate for a 10-scene document must stay under 200 tokens, got {tokens} ({body})"
    );

    client.cancel().await.expect("clean shutdown");
}
