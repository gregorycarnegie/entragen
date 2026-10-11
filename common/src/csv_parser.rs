use crate::{errors::ParseError, types::ParsedData, validate_headers};

pub(crate) fn parse_csv(text: &str, required: &[&str]) -> Result<ParsedData, ParseError> {
    let text = text.trim_start_matches('\u{FEFF}');

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());

    let raw_headers = reader
        .headers()
        .map_err(|e| ParseError::CsvError(e.to_string()))?;

    let headers: Vec<String> = raw_headers.iter().map(|h| h.trim().to_string()).collect();

    validate_headers(&headers, required)?;

    let mut rows: Vec<Vec<String>> = Vec::new();
    for record in reader.records() {
        // Surface a bad record rather than dropping it - a short student list is
        // worse than an error, because nobody notices it.
        let record = record.map_err(|e| ParseError::CsvError(e.to_string()))?;
        if record.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        let mut fields: Vec<String> = record.iter().map(|f| f.to_string()).collect();
        // Pad short rows to header length so col_idx lookups are safe
        while fields.len() < headers.len() {
            fields.push(String::new());
        }
        rows.push(fields);
    }

    Ok(ParsedData { headers, rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    #[test]
    fn parse_csv_trims_headers_skips_empty_rows_and_pads_short_rows() {
        let parsed = parse_csv(
            "\u{FEFF} Name , Email , Year \nAlice,alice@example.com,7\n,,\nBob,bob@example.com\n",
            &["Name", "Email"],
        )
        .expect("CSV should parse");

        assert_eq!(
            parsed.headers,
            vec!["Name".to_string(), "Email".to_string(), "Year".to_string()]
        );
        assert_eq!(
            parsed.rows,
            vec![
                vec![
                    "Alice".to_string(),
                    "alice@example.com".to_string(),
                    "7".to_string()
                ],
                vec![
                    "Bob".to_string(),
                    "bob@example.com".to_string(),
                    String::new()
                ]
            ]
        );
    }

    #[rstest]
    #[case::absent("Year")]
    #[case::case_sensitive("name")]
    fn parse_csv_reports_missing_required_column(#[case] required: &str) {
        let err = parse_csv("Name,Email\nAlice,alice@example.com\n", &[required])
            .expect_err("missing required column should fail");

        match err {
            ParseError::MissingColumn(column) => assert_eq!(column, required),
            other => panic!("expected MissingColumn, got {other:?}"),
        }
    }

    #[rstest]
    #[case::empty("")]
    #[case::bom_only("\u{FEFF}")]
    #[case::blank_headers(", ,\n")]
    #[case::whitespace(" \t\r\n")]
    fn parse_csv_reports_empty_file_when_headers_are_blank(#[case] input: &str) {
        let err = parse_csv(input, &[]).expect_err("blank headers should fail");

        assert!(matches!(err, ParseError::EmptyFile));
    }

    #[rstest]
    #[case::quoted("\"Ada, A\",\"a\"\"b\"\n", &["Ada, A", "a\"b"])]
    #[case::multiline("\"Ada\nLovelace\",7\r\n", &["Ada\nLovelace", "7"])]
    #[case::short("Ada\n", &["Ada", ""])]
    #[case::long("Ada,7,extra\n", &["Ada", "7", "extra"])]
    #[case::preserves_spaces(" Ada , 7 \n", &[" Ada ", " 7 "])]
    fn parse_csv_preserves_fields(#[case] record: &str, #[case] expected: &[&str]) {
        let parsed = parse_csv(&format!("Name,Year\n{record}"), &["Name", "Year"]).unwrap();
        assert_eq!(parsed.rows, vec![expected]);
    }

    proptest! {
        #[test]
        fn csv_round_trip_preserves_unicode_fields_and_pads_short_rows(
            records in prop::collection::vec(prop::collection::vec("(?s).{0,30}", 0..5), 0..20)
        ) {
            let mut writer = csv::WriterBuilder::new().flexible(true).from_writer(Vec::new());
            writer.write_record(["Name", "Year", "Class"]).unwrap();
            let mut expected = Vec::new();
            for (i, fields) in records.into_iter().enumerate() {
                // An anchor keeps an all-blank generated row from being intentionally skipped.
                let mut row = vec![format!("student-{i}")];
                row.extend(fields);
                writer.write_record(&row).unwrap();
                row.resize(row.len().max(3), String::new());
                expected.push(row);
            }
            let bytes = writer.into_inner().unwrap();
            let parsed = crate::parse_any(&bytes, &["Name", "Year", "Class"]).unwrap();
            prop_assert_eq!(parsed.headers, ["Name", "Year", "Class"]);
            prop_assert_eq!(parsed.rows, expected);
        }
    }
}
