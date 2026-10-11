use calamine::{Error as WorkbookError, Reader, Sheets, open_workbook_auto_from_rs};
use std::io::Cursor;

use crate::{errors::ParseError, types::ParsedData};

/// Reads the first sheet of a zip-based workbook: .xlsx, .xlsm, .xlsb or .ods.
///
/// Not public: `crate::parse_any` is the entry point, and it is what decides
/// which formats are allowed through. calamine's auto reader would also open a
/// legacy .xls, which `parse_any` refuses before reaching here.
pub(crate) fn parse_workbook(bytes: &[u8], required: &[&str]) -> Result<ParsedData, ParseError> {
    let mut workbook: Sheets<Cursor<&[u8]>> = open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|e: WorkbookError| ParseError::InvalidFormat(e.to_string()))?;

    let sheet_names: Vec<String> = workbook.sheet_names().to_owned();
    let first = sheet_names.first().ok_or(ParseError::EmptyFile)?.clone();

    let range = workbook
        .worksheet_range(&first)
        .map_err(|e: WorkbookError| ParseError::InvalidFormat(e.to_string()))?;

    let mut row_iter = range.rows();

    let header_row = row_iter.next().ok_or(ParseError::EmptyFile)?;
    let headers: Vec<String> = header_row
        .iter()
        .map(|c| c.to_string().trim().to_string())
        .collect();

    crate::validate_headers(&headers, required)?;

    let rows: Vec<Vec<String>> = row_iter
        .filter(|row| row.iter().any(|c| !c.to_string().trim().is_empty()))
        // No padding needed: a calamine Range is rectangular, so every row is
        // already header width. (Unlike CSV, which is flexible.)
        .map(|row| row.iter().map(|c| c.to_string()).collect())
        .collect();

    Ok(ParsedData { headers, rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_any;
    use rstest::rstest;

    // Generated, not hand-edited; regenerate with common/fixtures/mkxlsx.py.
    const STUDENTS_XLSX: &[u8] = include_bytes!("../fixtures/students.xlsx");
    const STUDENTS_ODS: &[u8] = include_bytes!("../fixtures/students.ods");
    const BLANK_HEADERS: &[u8] = include_bytes!("../fixtures/blank_headers.xlsx");

    fn alice_and_bob() -> Vec<Vec<String>> {
        vec![
            vec!["Alice".to_string(), "7".to_string(), "7A".to_string()],
            vec!["Bob".to_string(), String::new(), String::new()],
        ]
    }

    #[rstest]
    #[case::xlsx(STUDENTS_XLSX)]
    #[case::ods(STUDENTS_ODS)]
    fn parse_workbook_reads_xlsx_and_ods_identically(#[case] bytes: &[u8]) {
        let parsed = parse_workbook(bytes, &["Name", "Class"]).expect("workbook should parse");
        assert_eq!(parsed.headers, ["Name", "Year", "Class"]);
        assert_eq!(parsed.rows, alice_and_bob());
    }

    #[rstest]
    #[case::xlsx(STUDENTS_XLSX)]
    #[case::ods(STUDENTS_ODS)]
    fn parse_workbook_reports_missing_required_column(#[case] bytes: &[u8]) {
        let err = parse_workbook(bytes, &["Name", "House"])
            .expect_err("missing required column should fail");

        match err {
            ParseError::MissingColumn(column) => assert_eq!(column, "House"),
            other => panic!("expected MissingColumn, got {other:?}"),
        }
    }

    #[test]
    fn parse_workbook_reports_empty_file_when_headers_are_blank() {
        let err = parse_workbook(BLANK_HEADERS, &[]).expect_err("blank headers should fail");

        assert!(matches!(err, ParseError::EmptyFile));
    }

    #[rstest]
    #[case::xlsx(STUDENTS_XLSX)]
    #[case::ods(STUDENTS_ODS)]
    #[case::csv(b"Name,Year,Class\nAlice,7,7A\n\nBob\n")]
    fn parse_any_dispatches_on_content_not_on_a_filename(#[case] bytes: &[u8]) {
        let parsed = parse_any(bytes, &["Name", "Class"]).expect("file should parse");
        assert_eq!(parsed.headers, ["Name", "Year", "Class"]);
        assert_eq!(parsed.rows, alice_and_bob());
    }

    #[test]
    fn parse_any_refuses_a_legacy_xls_with_advice() {
        // Compound File Binary header - the start of every pre-2007 .xls.
        let mut xls = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        xls.extend_from_slice(&[0u8; 64]);

        let err = parse_any(&xls, &["Name"]).expect_err("legacy .xls should be refused");

        let ParseError::InvalidFormat(message) = err else {
            panic!("expected InvalidFormat, got {err:?}");
        };
        assert!(message.contains("save it as .xlsx or .ods"), "{message}");
    }

    #[test]
    fn parse_any_reports_invalid_format_for_a_corrupt_workbook() {
        // Zip magic, but not a workbook: must not fall through to the CSV path.
        let err = parse_any(b"PK\x03\x04 not really a workbook", &["Name"])
            .expect_err("a truncated zip is not a workbook");

        assert!(matches!(err, ParseError::InvalidFormat(_)));
    }
}
