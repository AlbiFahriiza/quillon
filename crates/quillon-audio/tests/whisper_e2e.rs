//! A real whisper run, ignored by default: it needs the model weights
//! (downloaded on demand, ~75 MB) and is therefore not part of CI. Run
//! locally with `cargo test -p quillon-audio --features transcribe --
//! --ignored whisper` after placing or caching the model; set
//! QUILLON_WHISPER_MODEL to point at an explicit ggml file.

#![cfg(feature = "transcribe")]

use quillon_audio::transcribe;
use quillon_core::{RegularizeConfig, regularize_cues};

const TB: i64 = 705_600_000;

#[test]
#[ignore = "downloads the ~75 MB whisper model on first run"]
fn whisper_transcript_passes_regularization() {
    let model = match std::env::var("QUILLON_WHISPER_MODEL") {
        Ok(path) => std::path::PathBuf::from(path),
        Err(_) => {
            let path = transcribe::default_model_path()
                .expect("a cache dir is derivable; or set QUILLON_WHISPER_MODEL");
            transcribe::ensure_model(&path).expect("model available");
            path
        }
    };
    let speech = transcribe::decode_mono_16k(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/speech.wav"
    )))
    .expect("speech fixture decodes");

    let cues = transcribe::transcribe(&model, &speech.samples, TB, 4)
        .expect("whisper runs on the fixture");
    assert!(!cues.is_empty(), "the fixture contains clear speech");

    let cfg = RegularizeConfig::default();
    let out = regularize_cues(&cues, &cfg, TB);
    assert!(!out.is_empty());
    for cue in &out {
        let duration = cue.to - cue.from;
        assert!(
            duration >= cfg.min_duration && duration <= cfg.max_duration,
            "cue {:?} has duration {duration}",
            cue.text
        );
    }
    let text = out
        .iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    assert!(
        text.contains("caption") || text.contains("hello"),
        "the transcript must contain the spoken words: {text}"
    );
}
