//! Subtitle import/export and timing regularization (plan §4 F5, §12 v0.5;
//! research/H §1: SRT/VTT are trivial enough to own, ASS goes through
//! rsubs-lib at the CLI layer, and raw parser output is never burned
//! directly — it always passes through `regularize_cues`).
//!
//! Reading is tolerant (BOM, CRLF, `,` or `.` milliseconds, optional hours
//! in VTT, ignored cue settings, NOTE blocks); writing is strict and
//! canonical (LF endings, SRT writes `,mmm`, VTT writes `WEBVTT` and
//! hours-always timings). Millisecond <-> tick conversion is integer-exact
//! at the flicks timebase, so an SRT round-trip preserves timing exactly.

use crate::caption::CaptionCue;
use crate::time::FLICKS_PER_SECOND;

/// A subtitle parse failure. `line` is 1-based in the submitted text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for SubtitleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for SubtitleError {}

fn err(line: usize, message: impl Into<String>) -> SubtitleError {
    SubtitleError {
        line,
        message: message.into(),
    }
}

/// Most cues one subtitle file may carry before parsing stops (r1 F2):
/// the same cap the validator puts on a caption track, enforced at the
/// read boundary so a crafted file cannot build millions of cues in RAM
/// before validation ever runs.
pub const MAX_PARSED_CUES: usize = 5_000;
/// Longest single cue text accepted by the parsers (r1 F2), matching the
/// text-element cap (`MAX_TEXT_LEN`). Splitting and merging happen after
/// this check, so the work done on untrusted input stays bounded.
pub const MAX_CUE_TEXT_BYTES: usize = 10_000;

/// Hours a subtitle timestamp may carry (r3 F1). Minutes, seconds and
/// milliseconds were always range-checked; hours were not, so a giant
/// value overflowed the millisecond arithmetic and panicked. 100 h sits
/// far above any real subtitle file (a day of continuous captions is 24)
/// while keeping `hours * 3.6e6` far from i64::MAX.
pub const MAX_TIMESTAMP_HOURS: i64 = 100;

/// Milliseconds to document ticks. Exact for any timebase divisible by
/// 1000 (format v1 accepts flicks only). Wraps for inputs the product of
/// whose magnitude and the timebase exceeds i64 — the callers that face
/// untrusted timing (the subtitle parsers) use
/// [`ms_to_ticks_checked`] instead (r3 F2).
pub fn ms_to_ticks(ms: i64, timebase: i64) -> i64 {
    ms * timebase / 1000
}

/// [`ms_to_ticks`] with the overflow named instead of wrapped (r3 F2).
/// The error message states the offending milliseconds and the timebase.
pub fn ms_to_ticks_checked(ms: i64, timebase: i64) -> Result<i64, String> {
    match (ms as i128 * timebase as i128 / 1000).try_into() {
        Ok(ticks) => Ok(ticks),
        Err(_) => Err(format!(
            "a timestamp of {ms} ms does not fit the document timeline (timebase {timebase})"
        )),
    }
}

/// Document ticks to milliseconds; the exact inverse of [`ms_to_ticks`].
pub fn ticks_to_ms(ticks: i64, timebase: i64) -> i64 {
    ticks * 1000 / timebase
}

/// One parsed block before tick conversion.
struct RawCue {
    from_ms: i64,
    to_ms: i64,
    text: String,
}

/// Parse a timestamp like `HH:MM:SS,mmm`, `MM:SS.mmm` (hours optional).
/// Returns total milliseconds.
fn parse_timestamp(text: &str) -> Option<i64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    let (h_str, m_str, s_str) = match parts.as_slice() {
        [m, s] => ("0", *m, *s),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    let h: i64 = h_str.parse().ok()?;
    let m: i64 = m_str.parse().ok()?;
    let (sec, ms) = match s_str.split_once([',', '.']) {
        Some((sec, ms)) => (sec, ms),
        None => (s_str, "0"),
    };
    let sec: i64 = sec.parse().ok()?;
    let ms: i64 = ms.parse().ok()?;
    // r3 F1: hours is the only unbounded field (m/sec/ms are checked
    // below); without its own bound the millisecond arithmetic overflows
    // and panics on crafted files.
    if !(0..=MAX_TIMESTAMP_HOURS).contains(&h) {
        return None;
    }
    if m > 59 || sec > 59 || ms > 999 {
        return None;
    }
    Some(((h * 60 + m) * 60 + sec) * 1000 + ms)
}

/// Format milliseconds as `HH:MM:SS,mmm` (SRT) or `HH:MM:SS.mmm` (VTT).
fn format_timestamp(ms: i64, comma: bool) -> String {
    let sec_total = ms / 1000;
    let millis = ms % 1000;
    let (h, m, s) = (sec_total / 3600, (sec_total / 60) % 60, sec_total % 60);
    let sep = if comma { ',' } else { '.' };
    format!("{h:02}:{m:02}:{s:02}{sep}{millis:03}")
}

/// Split into cue blocks on blank lines, normalizing BOM and line endings.
/// `str::lines` already terminates on `\n` and `\r\n`.
fn cue_blocks(input: &str) -> Vec<(usize, Vec<&str>)> {
    let cleaned = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut blocks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut block_start = 1usize;
    for (i, line) in cleaned.lines().enumerate() {
        let line_no = i + 1;
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push((block_start, std::mem::take(&mut current)));
            }
            continue;
        }
        if current.is_empty() {
            block_start = line_no;
        }
        current.push(line);
    }
    if !current.is_empty() {
        blocks.push((block_start, current));
    }
    blocks
}

/// Parse the timing line `start --> end [settings]` into milliseconds.
fn parse_timing_line(line: &str) -> Option<(i64, i64)> {
    let (start, rest) = line.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    Some((parse_timestamp(start)?, parse_timestamp(end)?))
}

/// Shared cue-block walker. `vtt` relaxes the header and skips non-cue
/// blocks (NOTE/STYLE/REGION); SRT requires the arrow in every block.
fn parse_cues(input: &str, vtt: bool) -> Result<Vec<RawCue>, SubtitleError> {
    let blocks = cue_blocks(input);
    let mut first = true;
    let mut cues = Vec::new();
    for (start_line, block) in blocks {
        if first {
            first = false;
            if vtt {
                if !block[0].starts_with("WEBVTT") {
                    return Err(err(start_line, "a WebVTT file starts with a WEBVTT line"));
                }
                if block.len() == 1 {
                    continue; // header-only block
                }
                // The header block may carry more than the signature line;
                // fall through and look for a cue inside it.
            }
        }
        let timing_idx = block.iter().position(|l| l.contains("-->"));
        let Some(timing_idx) = timing_idx else {
            if vtt && block[0].split_whitespace().next() != Some("WEBVTT") {
                // NOTE/STYLE/REGION and stray id-only blocks carry no cue.
                continue;
            }
            if !vtt && timing_idx.is_none() {
                return Err(err(
                    start_line,
                    "cue block has no \"start --> end\" timing line",
                ));
            }
            continue;
        };
        let (from_ms, to_ms) = parse_timing_line(block[timing_idx]).ok_or_else(|| {
            err(
                start_line + timing_idx,
                format!(
                    "cannot read the cue timing; expected HH:MM:SS,mmm --> HH:MM:SS,mmm \
                     with hours at most {MAX_TIMESTAMP_HOURS}"
                ),
            )
        })?;
        if from_ms >= to_ms {
            return Err(err(
                start_line + timing_idx,
                "cue start must be earlier than its end",
            ));
        }
        let text_lines: Vec<&str> = block[timing_idx + 1..]
            .iter()
            .map(|l| l.trim_end())
            .collect();
        // Trailing empty lines are formatting, not content.
        let text: String = text_lines
            .iter()
            .rev()
            .skip_while(|l| l.is_empty())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .copied()
            .collect::<Vec<&str>>()
            .join("\n");
        // A cue with no text is the format's "clear the screen": skip it.
        if text.trim().is_empty() {
            continue;
        }
        if text.len() > MAX_CUE_TEXT_BYTES {
            return Err(err(
                start_line + timing_idx,
                format!(
                    "cue text is {} bytes; the limit is {MAX_CUE_TEXT_BYTES} bytes per cue",
                    text.len()
                ),
            ));
        }
        if cues.len() >= MAX_PARSED_CUES {
            return Err(err(
                start_line,
                format!(
                    "the file has more than {MAX_PARSED_CUES} cues; split it into several tracks or files"
                ),
            ));
        }
        cues.push(RawCue {
            from_ms,
            to_ms,
            text,
        });
    }
    Ok(cues)
}

fn raw_to_cues(raw: Vec<RawCue>, timebase: i64) -> Result<Vec<CaptionCue>, SubtitleError> {
    // r3 F2: the conversion is checked; the parser's hour cap makes this
    // unreachable for the SRT/VTT path, but raw_to_cues is a private seam
    // and the check costs nothing.
    let mut cues = Vec::with_capacity(raw.len());
    for c in raw {
        let from = ms_to_ticks_checked(c.from_ms, timebase).map_err(|e| err(0, e))?;
        let to = ms_to_ticks_checked(c.to_ms, timebase).map_err(|e| err(0, e))?;
        cues.push(CaptionCue {
            from,
            to,
            text: c.text,
        });
    }
    Ok(cues)
}

/// Parse an SRT subtitle file into cues on the document timeline.
pub fn parse_srt(input: &str, timebase: i64) -> Result<Vec<CaptionCue>, SubtitleError> {
    raw_to_cues(parse_cues(input, false)?, timebase)
}

/// Parse a WebVTT file into cues on the document timeline.
pub fn parse_vtt(input: &str, timebase: i64) -> Result<Vec<CaptionCue>, SubtitleError> {
    raw_to_cues(parse_cues(input, true)?, timebase)
}

/// Serialize cues as SRT (canonical: LF, `,mmm`, numbered from 1).
pub fn to_srt(cues: &[CaptionCue], timebase: i64) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            format_timestamp(ticks_to_ms(cue.from, timebase), true),
            format_timestamp(ticks_to_ms(cue.to, timebase), true),
            cue.text
        ));
    }
    out
}

/// Serialize cues as WebVTT (canonical: LF, `.mmm`, hours always written).
pub fn to_vtt(cues: &[CaptionCue], timebase: i64) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues {
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            format_timestamp(ticks_to_ms(cue.from, timebase), false),
            format_timestamp(ticks_to_ms(cue.to, timebase), false),
            cue.text
        ));
    }
    out
}

/// How regularization shapes cue timing (plan §12 v0.5, research/H §1).
/// Defaults are our own product choices for comfortable reading — they are
/// configurable, and are not claimed to be any broadcast standard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegularizeConfig {
    /// Shortest cue duration in ticks (default 1.0 s).
    pub min_duration: i64,
    /// Longest cue duration in ticks (default 6.0 s).
    pub max_duration: i64,
    /// Reading speed ceiling in characters per second (default 18).
    pub max_cps: f32,
    /// Adjacent cues whose gap is at most this (ticks) merge when the
    /// merged cue still satisfies the other bounds (default 0.5 s).
    pub merge_gap: i64,
}

impl Default for RegularizeConfig {
    fn default() -> Self {
        RegularizeConfig {
            min_duration: FLICKS_PER_SECOND,
            max_duration: 6 * FLICKS_PER_SECOND,
            max_cps: 18.0,
            merge_gap: FLICKS_PER_SECOND / 2,
        }
    }
}

impl RegularizeConfig {
    /// Ticks a text of `chars` characters needs at `max_cps` (rounded up).
    fn needed_ticks(&self, chars: usize, timebase: i64) -> i64 {
        if self.max_cps <= 0.0 {
            return 0;
        }
        (chars as f64 / f64::from(self.max_cps) * timebase as f64).ceil() as i64
    }

    /// Largest character count one cue may carry: whatever is readable
    /// within `max_duration` at `max_cps`.
    fn chars_per_cue(&self, timebase: i64) -> usize {
        if self.max_cps <= 0.0 {
            return usize::MAX;
        }
        let seconds = self.max_duration as f64 / timebase as f64;
        (f64::from(self.max_cps) * seconds).floor().max(1.0) as usize
    }
}

/// Split one cue whose text cannot be read within `max_duration` into
/// chunks that can, splitting at word boundaries (never mid-word, except a
/// single word longer than the limit). Durations divide proportionally to
/// character count, every chunk keeping at least `min_duration` when the
/// original span allows it.
fn split_long_cue(cue: &CaptionCue, cfg: &RegularizeConfig, timebase: i64) -> Vec<CaptionCue> {
    let total_chars: usize = cue.text.chars().count();
    if total_chars <= cfg.chars_per_cue(timebase) || total_chars == 0 {
        return vec![cue.clone()];
    }
    let limit = cfg.chars_per_cue(timebase);
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in cue.text.split_whitespace() {
        let word_chars = word.chars().count();
        // +1 for the joining space (the first word has none).
        let added = word_chars + usize::from(!current.is_empty());
        // `current.chars().count()` is the single source of truth; a
        // separate counter drifted from the spaces and overshot the limit.
        if !current.is_empty() && current.chars().count() + added > limit {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        // A single word longer than the limit is hard-split.
        if word_chars > limit {
            for piece in word.chars().collect::<Vec<_>>().chunks(limit) {
                current.extend(piece.iter());
                chunks.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    // Distribute the original span proportionally by chunk length.
    let total_span = cue.to - cue.from;
    let counts: Vec<usize> = chunks.iter().map(|c| c.chars().count()).collect();
    let sum: usize = counts.iter().sum();
    let mut out = Vec::with_capacity(chunks.len());
    let mut offset = cue.from;
    for (i, text) in chunks.into_iter().enumerate() {
        let span = if i + 1 == counts.len() {
            cue.to - offset // the last chunk absorbs the rounding remainder
        } else {
            ((total_span as i128 * counts[i] as i128) / sum.max(1) as i128) as i64
        };
        out.push(CaptionCue {
            from: offset,
            to: offset + span.max(1),
            text,
        });
        offset += span.max(1);
    }
    out
}

/// Regularize cue timing (design note v0.5 D5). Pipeline: drop empty
/// texts, sort by start, split unreadably long texts, merge cues separated
/// by tiny gaps, then clamp each duration into
/// [max(min_duration, reading time), max_duration] while resolving
/// overlaps left-to-right (an earlier cue keeps its end; later cues start
/// after it). The result is sorted, non-overlapping, and every duration
/// respects the bounds and the reading-speed ceiling.
pub fn regularize_cues(
    cues: &[CaptionCue],
    cfg: &RegularizeConfig,
    timebase: i64,
) -> Vec<CaptionCue> {
    // 1. Empty texts carry nothing; keep the pipeline's contract simple.
    let mut sorted: Vec<CaptionCue> = cues
        .iter()
        .filter(|c| !c.text.trim().is_empty() && c.to > c.from)
        .cloned()
        .collect();
    sorted.sort_by_key(|c| c.from);

    // 2. Split texts too long for one readable cue.
    let mut split: Vec<CaptionCue> = Vec::new();
    for cue in &sorted {
        split.extend(split_long_cue(cue, cfg, timebase));
    }

    // 3. Merge cues separated by a tiny gap when the merged cue still
    //    reads in time and does not exceed max_duration.
    let mut merged: Vec<CaptionCue> = Vec::new();
    for cue in split {
        let Some(last) = merged.last_mut() else {
            merged.push(cue);
            continue;
        };
        let gap = cue.from - last.to;
        let combined_chars = last.text.chars().count() + cue.text.chars().count() + 1;
        let combined_span = cue.to - last.from;
        if gap >= 0
            && gap <= cfg.merge_gap
            && combined_span <= cfg.max_duration
            && combined_span >= cfg.needed_ticks(combined_chars, timebase)
        {
            last.to = last.to.max(cue.to);
            last.text.push('\n');
            last.text.push_str(&cue.text);
            continue;
        }
        merged.push(cue);
    }

    // 4. Clamp durations and resolve overlaps left-to-right.
    let mut out: Vec<CaptionCue> = Vec::with_capacity(merged.len());
    let mut prev_end = i64::MIN;
    for mut cue in merged {
        cue.from = cue.from.max(prev_end);
        // Splitting guarantees the reading floor fits inside max_duration
        // for sane configs; with an extreme config (min_duration or the
        // reading time above max_duration) max_duration wins rather than
        // panicking — a public API must not panic on caller data.
        let floor = cfg
            .min_duration
            .max(cfg.needed_ticks(cue.text.chars().count(), timebase))
            .min(cfg.max_duration);
        let duration = (cue.to - cue.from).clamp(floor, cfg.max_duration);
        cue.to = cue.from + duration;
        prev_end = cue.to;
        out.push(cue);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TB: i64 = FLICKS_PER_SECOND;
    const S: i64 = FLICKS_PER_SECOND;

    fn cue(from_s: f64, to_s: f64, text: &str) -> CaptionCue {
        CaptionCue {
            from: (from_s * S as f64) as i64,
            to: (to_s * S as f64) as i64,
            text: text.into(),
        }
    }

    #[test]
    fn ms_and_ticks_convert_exactly_both_ways() {
        for ms in [0i64, 1, 500, 999, 1000, 59_999, 3_600_000, 42_421_957] {
            let ticks = ms_to_ticks(ms, TB);
            assert_eq!(ticks_to_ms(ticks, TB), ms, "ms {ms} must round-trip");
        }
    }

    #[test]
    fn srt_round_trip_preserves_timing_exactly() {
        let cues = vec![
            cue(1.0, 3.5, "hello"),
            cue(4.0, 6.25, "two\nlines"),
            cue(70.0, 72.0, "later"),
        ];
        let text = to_srt(&cues, TB);
        let parsed = parse_srt(&text, TB).expect("round trip parses");
        assert_eq!(parsed, cues, "round trip must be exact, got:\n{text}");
    }

    #[test]
    fn srt_parses_crlf_bom_and_dot_milliseconds() {
        let input = "\u{feff}1\r\n00:00:01.000 --> 00:00:02,000\r\nhi\r\n\r\n";
        let parsed = parse_srt(input, TB).expect("tolerant read");
        assert_eq!(parsed, vec![cue(1.0, 2.0, "hi")]);
    }

    #[test]
    fn srt_without_timing_line_is_a_named_error() {
        let error = parse_srt("1\nnot a cue\n\n", TB).unwrap_err();
        assert_eq!(error.line, 1);
        assert!(error.message.contains("timing"));
    }

    #[test]
    fn srt_reversed_cue_is_rejected() {
        let error = parse_srt("1\n00:00:05,000 --> 00:00:01,000\nx\n\n", TB).unwrap_err();
        assert!(error.message.contains("earlier"), "{error}");
    }

    #[test]
    fn srt_empty_text_cue_is_skipped_not_an_error() {
        // An empty cue is SRT's "clear the screen"; it maps to no cue.
        let input = "1\n00:00:01,000 --> 00:00:02,000\n\n2\n00:00:03,000 --> 00:00:04,000\nok\n\n";
        let parsed = parse_srt(input, TB).expect("parses");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].text, "ok");
    }

    #[test]
    fn vtt_round_trip_preserves_timing_exactly() {
        let cues = vec![cue(0.5, 2.5, "webvtt cue"), cue(61.0, 63.75, "an hour in")];
        let text = to_vtt(&cues, TB);
        assert!(text.starts_with("WEBVTT\n\n"));
        let parsed = parse_vtt(&text, TB).expect("round trip parses");
        assert_eq!(parsed, cues);
    }

    #[test]
    fn vtt_accepts_short_timestamps_notes_and_settings() {
        let input = "WEBVTT\n\nNOTE this is a comment\nspanning lines\n\nid-1\n01:02.500 --> 01:04.000 align:start line:0%\nsettings ignored\n\n";
        let parsed = parse_vtt(input, TB).expect("tolerant read");
        assert_eq!(parsed, vec![cue(62.5, 64.0, "settings ignored")]);
    }

    #[test]
    fn vtt_without_header_is_rejected() {
        let error = parse_vtt("00:01.000 --> 00:02.000\nno header\n\n", TB).unwrap_err();
        assert!(error.message.contains("WEBVTT"), "{error}");
    }

    #[test]
    fn short_cues_are_extended_to_the_minimum() {
        let cfg = RegularizeConfig::default();
        let out = regularize_cues(&[cue(0.0, 0.2, "hi")], &cfg, TB);
        assert_eq!(out.len(), 1);
        assert!(out[0].to - out[0].from >= cfg.min_duration);
    }

    #[test]
    fn long_cues_are_capped_at_the_maximum() {
        let cfg = RegularizeConfig::default();
        let out = regularize_cues(&[cue(0.0, 30.0, "short text, long span")], &cfg, TB);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to - out[0].from, cfg.max_duration);
    }

    #[test]
    fn tiny_gaps_merge_when_the_result_still_reads() {
        let cfg = RegularizeConfig::default();
        let out = regularize_cues(&[cue(0.0, 1.0, "hello"), cue(1.2, 2.2, "world")], &cfg, TB);
        assert_eq!(out.len(), 1, "0.2s gap merges: {out:?}");
        assert_eq!(out[0].text, "hello\nworld");
    }

    #[test]
    fn big_gaps_do_not_merge() {
        let cfg = RegularizeConfig::default();
        let out = regularize_cues(&[cue(0.0, 1.0, "hello"), cue(3.0, 4.0, "world")], &cfg, TB);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn unreadably_long_text_splits_across_cues() {
        let cfg = RegularizeConfig::default();
        // 18 cps * 6s = 108 chars per cue; this text needs two.
        let long = "word ".repeat(30); // 150 chars incl. spaces
        let out = regularize_cues(&[cue(0.0, 4.0, long.trim())], &cfg, TB);
        assert_eq!(out.len(), 2, "150 chars at 18cps/6s must split: {out:?}");
        for c in &out {
            let chars = c.text.chars().count();
            let seconds = (c.to - c.from) as f64 / TB as f64;
            assert!(
                chars as f64 / seconds <= cfg.max_cps as f64 + 1e-6,
                "cue {chars} chars in {seconds}s exceeds max_cps"
            );
        }
    }

    #[test]
    fn overlapping_transcript_drift_is_resolved_in_order() {
        // Whisper-style drift: the second cue starts before the first ends.
        let cfg = RegularizeConfig::default();
        let out = regularize_cues(
            &[
                cue(0.0, 2.0, "first"),
                cue(1.5, 4.0, "second"),
                cue(4.5, 5.0, "third"),
            ],
            &cfg,
            TB,
        );
        for pair in out.windows(2) {
            assert!(pair[0].to <= pair[1].from, "cues must not overlap: {out:?}");
        }
        for c in &out {
            let dur = c.to - c.from;
            assert!(dur >= cfg.min_duration && dur <= cfg.max_duration);
        }
    }

    #[test]
    fn empty_text_cues_are_dropped() {
        let cfg = RegularizeConfig::default();
        let out = regularize_cues(&[cue(0.0, 1.0, "  "), cue(1.0, 2.0, "real")], &cfg, TB);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "real");
    }

    #[test]
    fn unsorted_input_is_sorted_before_everything_else() {
        let cfg = RegularizeConfig::default();
        let out = regularize_cues(&[cue(5.0, 6.0, "b"), cue(0.0, 1.0, "a")], &cfg, TB);
        assert_eq!(out[0].text, "a");
        assert_eq!(out[1].text, "b");
    }
}

#[cfg(test)]
mod r1_caps_tests {
    use super::*;

    const TB: i64 = FLICKS_PER_SECOND;

    #[test]
    fn cue_text_over_the_cap_is_a_named_error() {
        let text = "x".repeat(MAX_CUE_TEXT_BYTES + 1);
        let srt = format!("1\n00:00:00,000 --> 00:00:05,000\n{text}\n");
        let error = parse_srt(&srt, TB).unwrap_err();
        assert!(
            error
                .message
                .contains(&format!("the limit is {MAX_CUE_TEXT_BYTES}")),
            "{error}"
        );
    }

    #[test]
    fn more_cues_than_the_cap_is_a_named_error() {
        let mut srt = String::new();
        for i in 0..=MAX_PARSED_CUES {
            srt.push_str(&format!(
                "{i}\n00:00:0{},000 --> 00:00:0{},500\nline\n\n",
                i % 9,
                i % 9
            ));
        }
        let error = parse_srt(&srt, TB).unwrap_err();
        assert!(
            error
                .message
                .contains(&format!("more than {MAX_PARSED_CUES} cues")),
            "{error}"
        );
    }

    #[test]
    fn cues_at_the_cap_parse_fine() {
        let mut srt = String::new();
        for i in 0..MAX_PARSED_CUES {
            srt.push_str(&format!(
                "{}\n00:00:0{},000 --> 00:00:0{},500\nline\n\n",
                i + 1,
                i % 9,
                i % 9
            ));
        }
        let cues = parse_srt(&srt, TB).expect("exactly the cap parses");
        assert_eq!(cues.len(), MAX_PARSED_CUES);
    }
}

#[cfg(test)]
mod r3_timestamp_tests {
    use super::*;

    const TB: i64 = FLICKS_PER_SECOND;

    #[test]
    fn r3_giant_hours_are_named_errors_not_panics() {
        // R3-F1: the hours field was unbounded while minutes, seconds and
        // milliseconds were checked; a giant hours value overflowed the
        // millisecond arithmetic. The parser must reject it by name.
        let srt = "1\n999999999999999999:00:00,000 --> 999999999999999999:00:01,000\nboom\n";
        let result = parse_srt(srt, TB);
        match result {
            Err(e) => assert!(
                e.message.contains("hours"),
                "names the problem: {}",
                e.message
            ),
            Ok(cues) => panic!("giant hours must not parse; got {cues:?}"),
        }
    }

    #[test]
    fn r3_legal_hours_still_parse() {
        // A day plus change of hours is beyond any real subtitle but must
        // still parse and convert; the cap sits far above real material.
        let srt = "1\n100:00:00,000 --> 100:00:05,000\nlate night\n";
        let cues = parse_srt(srt, TB).expect("100 h is within the cap");
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].from, 100 * 3600 * TB);
    }

    #[test]
    fn r3_ms_to_ticks_rejects_overflow_by_name() {
        // R3-F2: ms * timebase overflows i64 well before ms hits i64::MAX;
        // the conversion must return an error, not wrap.
        let giant_ms = i64::MAX / 4; // * 705_600_000 overflows i64
        let result = ms_to_ticks_checked(giant_ms, TB);
        assert!(result.is_err(), "giant ms must be a named error");
        // The legal range stays exact.
        assert_eq!(
            ms_to_ticks_checked(42_421_957, TB).unwrap(),
            42_421_957 * TB / 1000
        );
    }

    #[test]
    fn r3_vtt_giant_hours_are_named_errors_too() {
        // VTT shares parse_cues with SRT, so the same guard covers it.
        let vtt = "WEBVTT\n\n00:00:01.000 --> 9999999999999:00:02.000\nboom\n";
        let result = parse_vtt(vtt, TB);
        match result {
            Err(e) => assert!(e.message.contains("hours"), "{}", e.message),
            Ok(cues) => panic!("giant VTT hours must not parse; got {cues:?}"),
        }
    }
}
