//! A minimal RFC-4180 CSV reader for `render --data` (plan §4 F12): header
//! row plus data rows, quoted fields with `""` escapes, commas inside
//! quotes, LF or CRLF. No new dependency — mail-merge CSVs are small and
//! this parser is fully under test.

use std::collections::BTreeMap;

/// Hardening D4: a mail-merge CSV beyond this many data rows is almost
/// certainly a mistake (or a resource attack); the error names the
/// counts. Chosen far above real template batches (the acceptance run
/// is 1000 rows).
pub const MAX_CSV_ROWS: usize = 1_000_000;
/// Hardening D4: one field beyond this many bytes is rejected by name
/// instead of silently building giant row maps.
pub const MAX_FIELD_BYTES: usize = 1024 * 1024;

/// Parse CSV text into rows keyed by the header columns. The first line is
/// the header; empty trailing lines are ignored.
pub fn parse_rows(input: &str) -> Result<Vec<BTreeMap<String, String>>, String> {
    // A UTF-8 BOM (spreadsheet exports on Windows prepend one) would
    // otherwise corrupt the first header name into "\u{feff}name".
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let records = split_records(input)?;
    let (header, data) = records
        .split_first()
        .ok_or_else(|| "the CSV file is empty; the first line must be a header row".to_string())?;
    if header.iter().any(|c| c.trim().is_empty()) {
        return Err("the CSV header has an empty column name".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for column in header {
        if !seen.insert(column) {
            return Err(format!("the CSV header repeats column \"{column}\""));
        }
    }
    let mut rows = Vec::with_capacity(data.len());
    for (i, record) in data.iter().enumerate() {
        if record.len() != header.len() {
            return Err(format!(
                "CSV row {} has {} field(s), the header has {}",
                i + 2,
                record.len(),
                header.len()
            ));
        }
        for field in record {
            if field.len() > MAX_FIELD_BYTES {
                return Err(format!(
                    "CSV row {} has a field of {} bytes; the limit is {MAX_FIELD_BYTES} bytes",
                    i + 2,
                    field.len()
                ));
            }
        }
        let mut row = BTreeMap::new();
        for (column, value) in header.iter().zip(record) {
            row.insert(column.clone(), value.clone());
        }
        rows.push(row);
        // Checked inside the loop so a runaway input fails after the
        // limit, not after building every row map.
        if rows.len() > MAX_CSV_ROWS {
            return Err(format!(
                "the CSV has more than {MAX_CSV_ROWS} data rows; the limit is {MAX_CSV_ROWS}"
            ));
        }
    }
    Ok(rows)
}

/// Split into records (lines, but quotes may span newlines) of fields.
fn split_records(input: &str) -> Result<Vec<Vec<String>>, String> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut fields: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut field));
            }
            '\r' if !in_quotes && chars.peek() == Some(&'\n') => {
                chars.next();
                end_record(&mut records, &mut fields, &mut field);
            }
            '\n' if !in_quotes => {
                end_record(&mut records, &mut fields, &mut field);
            }
            other => field.push(other),
        }
    }
    if in_quotes {
        return Err("the CSV file ends inside a quoted field".into());
    }
    end_record(&mut records, &mut fields, &mut field);
    Ok(records)
}

fn end_record(records: &mut Vec<Vec<String>>, fields: &mut Vec<String>, field: &mut String) {
    fields.push(std::mem::take(field));
    // Skip completely empty records (trailing blank lines).
    if fields.len() > 1 || !fields[0].is_empty() {
        records.push(std::mem::take(fields));
    } else {
        fields.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_quoted_commas_and_escapes() {
        let csv = "name,score,note\nAda,9.5,\"she said \"\"hi\"\", twice\"\nBo,7,\"plain\"\n";
        let rows = parse_rows(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], "Ada");
        assert_eq!(rows[0]["score"], "9.5");
        assert_eq!(rows[0]["note"], "she said \"hi\", twice");
        assert_eq!(rows[1]["note"], "plain");
    }

    #[test]
    fn crlf_and_trailing_blank_lines_are_tolerated() {
        let csv = "name\r\nAda\r\nBo\r\n\r\n";
        let rows = parse_rows(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["name"], "Bo");
    }

    #[test]
    fn ragged_rows_and_duplicate_headers_are_named_errors() {
        assert!(parse_rows("a,b\n1\n").unwrap_err().contains("row 2"));
        assert!(parse_rows("a,a\n1,2\n").unwrap_err().contains("repeats"));
        assert!(parse_rows("a,b\n1,\"2\n").unwrap_err().contains("quoted"));
        assert!(parse_rows("").unwrap_err().contains("header"));
    }

    #[test]
    fn a_utf8_bom_is_stripped_not_baked_into_the_header() {
        // Hardening D4: spreadsheet exports on Windows prepend a BOM;
        // without stripping, the first column becomes "\u{feff}name" and
        // every row fails the undeclared-column check confusingly.
        let rows = parse_rows("\u{feff}name\nAda\n").unwrap();
        assert_eq!(rows[0]["name"], "Ada");
    }

    #[test]
    fn oversized_fields_are_named_errors() {
        let big = "x".repeat(MAX_FIELD_BYTES + 1);
        let error = parse_rows(&format!("name\n{big}\n")).unwrap_err();
        assert!(
            error.contains("row 2") && error.contains(&MAX_FIELD_BYTES.to_string()),
            "{error}"
        );
        // Exactly at the limit stays legal.
        let edge = "x".repeat(MAX_FIELD_BYTES);
        assert!(parse_rows(&format!("name\n{edge}\n")).is_ok());
    }

    #[test]
    fn runaway_row_counts_fail_early_with_a_named_error() {
        let mut csv = String::from("name\n");
        csv.push_str(&"x\n".repeat(MAX_CSV_ROWS + 1));
        let error = parse_rows(&csv).unwrap_err();
        assert!(error.contains(&MAX_CSV_ROWS.to_string()), "{error}");
    }
}
