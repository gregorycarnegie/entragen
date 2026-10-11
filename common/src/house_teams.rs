use std::collections::{HashMap, HashSet};

use crate::{
    ENTRA_HEADER, sanitize_name,
    types::{GeneratedFile, HouseTeamsResult, MissingMatch, ParsedData},
};

fn normalize(s: &str) -> String {
    s.trim().to_lowercase()
}

pub fn process(bromcom: &ParsedData, entra: &ParsedData, timestamp: &str) -> HouseTeamsResult {
    let mail_idx = entra.col_idx("mail").expect("validated by parser");
    let id_idx = entra.col_idx("id").expect("validated by parser");

    let mut email_to_id: HashMap<String, String> = HashMap::new();
    for row in &entra.rows {
        let email = normalize(row.get(mail_idx).map(String::as_str).unwrap_or(""));
        let id = row
            .get(id_idx)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if !email.is_empty() && !id.is_empty() {
            email_to_id.entry(email).or_insert(id);
        }
    }

    let house_idx = bromcom.col_idx("House(s)").expect("validated by parser");
    let email_idx = bromcom
        .col_idx("Student email")
        .expect("validated by parser");
    let year_idx = bromcom
        .col_idx("Year Group Name")
        .expect("validated by parser");

    // Keyed on the raw (house, year) pair. Keying on the sanitized name instead
    // would silently merge distinct houses - "Red House" and "Red/House" both
    // sanitize to "Red_House".
    let mut groups: HashMap<(String, String), HashSet<String>> = HashMap::new();
    let mut missing: Vec<MissingMatch> = Vec::new();
    let mut processed = 0usize;

    for row in &bromcom.rows {
        let email = normalize(row.get(email_idx).map(String::as_str).unwrap_or(""));
        if email.is_empty() {
            continue;
        }
        processed += 1;

        let house = row
            .get(house_idx)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let year = row
            .get(year_idx)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        match email_to_id.get(&email) {
            None => missing.push(MissingMatch {
                email,
                house,
                year,
                reason: "No Entra ID found".to_string(),
            }),
            Some(_) if house.is_empty() || year.is_empty() => {
                missing.push(MissingMatch {
                    email,
                    house,
                    year,
                    reason: "Missing house or year group".to_string(),
                });
            }
            Some(id) => {
                groups.entry((house, year)).or_default().insert(id.clone());
            }
        }
    }

    let matched = processed - missing.len();
    let group_count = groups.len();

    // Sorted by the filename each group will get, ties broken by the raw key, so
    // that the duplicate-name suffix below always lands on the same group.
    let mut groups: Vec<((String, String), HashSet<String>)> = groups.into_iter().collect();
    groups.sort_by_key(|((house, year), _)| {
        (
            sanitize_name(house),
            sanitize_name(year),
            house.clone(),
            year.clone(),
        )
    });

    let mut files: Vec<GeneratedFile> = groups
        .into_iter()
        .map(|((house, year), ids)| {
            let mut id_list: Vec<String> = ids.into_iter().collect();
            id_list.sort();
            let content = format!("{}\n{}\n", ENTRA_HEADER, id_list.join("\n"));
            GeneratedFile {
                name: format!(
                    "{}_{}_{}.csv",
                    sanitize_name(&house),
                    sanitize_name(&year),
                    timestamp
                ),
                content,
                count: id_list.len(),
            }
        })
        .collect();

    // Distinct houses that sanitize to the same name are now separate groups, so
    // suffix the duplicate filenames rather than handing back two identical ones.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for file in &mut files {
        let n = seen.entry(file.name.clone()).or_default();
        *n += 1;
        if *n > 1 {
            let stem = file.name.trim_end_matches(".csv").to_string();
            file.name = format!("{stem}_{n}.csv");
        }
    }

    HouseTeamsResult {
        processed,
        matched,
        missing_count: missing.len(),
        group_count,
        files,
        missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    fn parsed(headers: &[&str], rows: Vec<Vec<&str>>) -> ParsedData {
        ParsedData {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(|value| value.to_string()).collect())
                .collect(),
        }
    }

    #[rstest]
    #[case::normalized(&[" Red ", " ALICE@school.org ", " Year 7 "], 1, None)]
    #[case::unknown_email(&["Red", "unknown@school.org", "Year 7"], 1, Some("No Entra ID found"))]
    #[case::missing_house(&[" \t", "alice@school.org", "Year 7"], 1, Some("Missing house or year group"))]
    #[case::missing_year(&["Red", "alice@school.org", ""], 1, Some("Missing house or year group"))]
    #[case::short_row(&["Red", "alice@school.org"], 1, Some("Missing house or year group"))]
    #[case::blank_email(&["Red", " \t", "Year 7"], 0, None)]
    #[case::empty_row(&[], 0, None)]
    fn process_handles_incomplete_rows(
        #[case] row: &[&str],
        #[case] processed: usize,
        #[case] reason: Option<&str>,
    ) {
        let bromcom = parsed(
            &["House(s)", "Student email", "Year Group Name"],
            vec![row.to_vec()],
        );
        let entra = parsed(
            &["id", "mail"],
            vec![vec![" ID-ALICE ", " Alice@school.org "]],
        );
        let result = process(&bromcom, &entra, "stamp");
        assert_eq!(result.processed, processed);
        assert_eq!(result.missing_count, usize::from(reason.is_some()));
        assert_eq!(result.matched, processed - result.missing_count);
        assert_eq!(result.files.len(), result.matched);
        if let Some(reason) = reason {
            assert_eq!(result.missing[0].reason, reason);
        } else if processed > 0 {
            assert_eq!(result.files[0].name, "Red_Year_7_stamp.csv");
            assert_eq!(
                result.files[0].content,
                format!("{ENTRA_HEADER}\nID-ALICE\n")
            );
        }
    }

    proptest! {
        #[test]
        fn house_groups_conserve_rows_deduplicate_members_and_ignore_row_order(
            rows in prop::collection::vec((0usize..12, 0usize..4, 0usize..3), 0..40)
        ) {
            let houses = ["", "Red House", "Red/House", "Blue"];
            let years = ["", "Year 7", "Year 8"];
            let mut bromcom = parsed(&["House(s)", "Student email", "Year Group Name"], vec![]);
            let mut entra = parsed(&["id", "mail"], vec![]);
            for i in 0..8 {
                entra.rows.push(vec![format!("ID-{i}"), format!("student{i}@school.org")]);
            }
            let mut expected = std::collections::BTreeMap::<(&str, &str), std::collections::BTreeSet<String>>::new();
            let mut matched = 0;
            for &(i, house, year) in &rows {
                bromcom.rows.push(vec![houses[house].into(), format!(" STUDENT{i}@school.org "), years[year].into()]);
                if i < 8 && house > 0 && year > 0 {
                    matched += 1;
                    expected.entry((houses[house], years[year])).or_default().insert(format!("ID-{i}"));
                }
            }
            let result = process(&bromcom, &entra, "stamp");
            prop_assert_eq!(result.processed, rows.len());
            prop_assert_eq!(result.matched, matched);
            prop_assert_eq!(result.missing_count, rows.len() - matched);
            prop_assert_eq!(result.missing.len(), result.missing_count);
            prop_assert_eq!(result.group_count, expected.len());
            prop_assert_eq!(result.files.len(), expected.len());
            let names = result.files.iter().map(|f| &f.name).collect::<HashSet<_>>();
            prop_assert_eq!(names.len(), result.files.len());
            let mut actual_members = Vec::new();
            for file in &result.files {
                prop_assert!(file.content.starts_with(&format!("{ENTRA_HEADER}\n")), "missing Entra header");
                let ids = file.content.lines().skip(3).map(str::to_string).collect::<Vec<_>>();
                prop_assert_eq!(file.count, ids.len());
                prop_assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
                actual_members.push(ids);
            }
            actual_members.sort();
            let mut expected_members = expected.into_values().map(|ids| ids.into_iter().collect::<Vec<_>>()).collect::<Vec<_>>();
            expected_members.sort();
            prop_assert_eq!(actual_members, expected_members);
            bromcom.rows.reverse();
            entra.rows.reverse();
            let reversed = process(&bromcom, &entra, "stamp");
            let files = |r: HouseTeamsResult| r.files.into_iter().map(|f| (f.name, f.content, f.count)).collect::<Vec<_>>();
            prop_assert_eq!(files(result), files(reversed));
        }
    }

    #[test]
    fn process_groups_matches_and_reports_missing_students() {
        let bromcom = parsed(
            &["House(s)", "Student email", "Year Group Name"],
            vec![
                vec!["Red House", "Alice@School.org ", "Year 7"],
                vec!["Blue/House", "missing@school.org", "Year 8"],
                vec!["", "blankhouse@school.org", "Year 9"],
                vec!["Green", "   ", "Year 10"],
                // House name keeps its hyphen but loses its apostrophe, so the
                // two halves of the sanitize_name predicate are distinguishable.
                vec!["St-Mary's", "noid@school.org", "Year 11"],
                vec!["St-Mary's", "bob@school.org", "Year 11"],
            ],
        );
        let entra = parsed(
            &["id", "mail"],
            vec![
                vec!["ID-ALICE", "alice@school.org"],
                vec!["ID-BLANKHOUSE", "blankhouse@school.org"],
                // Mail present, ID blank: only one half of the lookup guard fails.
                vec!["", "noid@school.org"],
                vec!["ID-BOB", "bob@school.org"],
            ],
        );

        let result = process(&bromcom, &entra, "20260510");

        // 5 and 3 chosen so processed - missing is not also processed / missing.
        assert_eq!(result.processed, 5);
        assert_eq!(result.matched, 2);
        assert_eq!(result.missing_count, 3);
        assert_eq!(result.group_count, 2);

        assert_eq!(result.files.len(), 2);
        assert_eq!(result.files[0].name, "Red_House_Year_7_20260510.csv");
        assert_eq!(result.files[0].count, 1);
        assert!(result.files[0].content.contains("ID-ALICE"));
        assert_eq!(result.files[1].name, "St-Mary_s_Year_11_20260510.csv");
        assert_eq!(result.files[1].count, 1);
        assert!(result.files[1].content.contains("ID-BOB"));

        let unmatched = result
            .missing
            .iter()
            .find(|m| m.email == "missing@school.org")
            .expect("unmatched student should be reported");
        assert_eq!(unmatched.house, "Blue/House");
        assert_eq!(unmatched.year, "Year 8");
        assert_eq!(unmatched.reason, "No Entra ID found");

        let incomplete = result
            .missing
            .iter()
            .find(|m| m.email == "blankhouse@school.org")
            .expect("student missing house should be reported");
        assert_eq!(incomplete.house, "");
        assert_eq!(incomplete.year, "Year 9");
        assert_eq!(incomplete.reason, "Missing house or year group");

        let blank_id = result
            .missing
            .iter()
            .find(|m| m.email == "noid@school.org")
            .expect("student whose Entra row has a blank id should be reported");
        assert_eq!(blank_id.reason, "No Entra ID found");
    }

    #[test]
    fn process_keeps_distinct_houses_that_sanitize_to_the_same_name() {
        let bromcom = parsed(
            &["House(s)", "Student email", "Year Group Name"],
            vec![
                vec!["Red House", "a@school.org", "Year 7"],
                vec!["Red/House", "b@school.org", "Year 7"],
                vec!["Red House", "c@school.org", "Year 7"],
            ],
        );
        let entra = parsed(
            &["id", "mail"],
            // Deliberately not in sorted order, so the file contents prove the sort.
            vec![
                vec!["ID-C", "c@school.org"],
                vec!["ID-A", "a@school.org"],
                vec!["ID-B", "b@school.org"],
            ],
        );

        let result = process(&bromcom, &entra, "20260510");

        // Two houses in, two groups out - not one merged group of three.
        assert_eq!(result.group_count, 2);
        assert_eq!(result.matched, 3);
        assert_eq!(
            result
                .files
                .iter()
                .map(|f| (f.name.as_str(), f.count))
                .collect::<Vec<_>>(),
            vec![
                ("Red_House_Year_7_20260510.csv", 2),
                ("Red_House_Year_7_20260510_2.csv", 1),
            ]
        );
        // Members are sorted, so the same input always produces the same file.
        assert!(result.files[0].content.ends_with(
            "ID-A
ID-C
"
        ));
        assert!(result.files[1].content.ends_with(
            "ID-B
"
        ));
    }
}
