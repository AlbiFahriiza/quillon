//! Bounded file reads for every external-input boundary (r2 R2-2).
//!
//! The per-cue and per-field caps from r1 fire after a file is fully in
//! memory; a multi-gigabyte input paid for the RAM before any of them
//! ran. This module reads with an explicit ceiling instead: metadata
//! first, then at most `max_bytes` into a String. Every limit error
//! names the limit and the actual size, like the rest of the guard set.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Documents, OTIO timelines and subtitle files: 64 MiB. The largest
/// legal subtitle file is ~50 MiB (5000 cues x 10 kB), documents are
/// kilobytes; anything past this is not a real file.
pub const MAX_TEXT_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Mail-merge CSVs: 512 MiB. One million rows (the r1 row cap) of small
/// fields is tens of MB; half a gigabyte stays generous without letting
/// a swapped-in disk image claim the RAM.
pub const MAX_CSV_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// Read a UTF-8 text file of at most `max_bytes`. Errors name the path,
/// the limit and the actual size, or the underlying I/O failure.
pub fn read_to_string_capped(path: &Path, max_bytes: u64) -> Result<String, String> {
    let len = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if len > max_bytes {
        return Err(format!(
            "{} is {} bytes; the limit is {max_bytes} bytes",
            path.display(),
            len
        ));
    }
    let mut file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut text = String::with_capacity(len as usize);
    // Read to end, then re-check the length: the file may have grown
    // between the metadata call and the read. The re-check bounds the
    // damage to one buffer, never an unbounded one.
    file.read_to_string(&mut text)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if text.len() as u64 > max_bytes {
        return Err(format!(
            "{} grew past {max_bytes} bytes while reading",
            path.display()
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(tag: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("quillon-capped-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{tag}.txt"));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn files_within_the_limit_are_read() {
        let path = temp_file("ok", b"hello");
        assert_eq!(read_to_string_capped(&path, 1024).as_deref(), Ok("hello"));
    }

    #[test]
    fn files_past_the_limit_are_named_errors() {
        let path = temp_file("big", &[b'x'; 300]);
        let err = read_to_string_capped(&path, 128).unwrap_err();
        assert!(err.contains("300 bytes"), "{err}");
        assert!(err.contains("128"), "names the limit: {err}");
    }

    #[test]
    fn missing_files_are_named_errors() {
        let err = read_to_string_capped(Path::new("/nonexistent/doc.json"), 10).unwrap_err();
        assert!(err.contains("/nonexistent/doc.json"), "{err}");
    }

    #[test]
    fn invalid_utf8_is_a_named_error() {
        let path = temp_file("utf8", &[0xff, 0xfe, 0xfd]);
        assert!(read_to_string_capped(&path, 1024).is_err());
    }
}
