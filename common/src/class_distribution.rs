use std::collections::{HashMap, HashSet};

use crate::{
    ENTRA_HEADER,
    errors::ProcessError,
    sanitize_name,
    types::{ClassDistResult, GeneratedFile, ParsedData, YearGroupStat},
};

pub fn parse_tags(input: &str) -> Vec<String> {
    let seen: HashSet<String> = input
        .split(|c: char| c == ',' || c == '\n' || c.is_whitespace())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    let mut tags: Vec<String> = seen.into_iter().collect();
    tags.sort();
    tags
}

/// Trimmed cell at `idx`, or "" when the row is short.
fn cell(row: &[String], idx: usize) -> &str {
    row.get(idx).map_or("", |s| s.trim())
}

/// How many distinct people these records cover. Keyed on the Entra ID, which is
/// what the generated files dedupe on, so a stat can never exceed a file's rows.
fn distinct_students<'a>(records: impl Iterator<Item = &'a StudentRecord>) -> usize {
    records
        .map(|s| s.entra_id.as_str())
        .collect::<HashSet<_>>()
        .len()
}

struct StudentRecord {
    /// Already resolved to "Unspecified" when blank, so the file split and the
    /// year-group stats below cannot disagree about which bucket a student is in.
    year_group: String,
    class_list: String,
    entra_id: String,
}

pub fn process(
    emails: &ParsedData,
    class_list: &ParsedData,
    entra: &ParsedData,
    tags: &[String],
    yeargroup_mode: bool,
    timestamp: &str,
) -> Result<ClassDistResult, ProcessError> {
    if tags.is_empty() {
        return Err(ProcessError::NoTags);
    }

    let mail_idx = entra.col_idx("mail").expect("validated by parser");
    let id_idx = entra.col_idx("id").expect("validated by parser");
    let mut entra_lookup: HashMap<String, String> = HashMap::new();
    for row in &entra.rows {
        let mail = cell(row, mail_idx).to_lowercase();
        let id = cell(row, id_idx);
        if !mail.is_empty() && !id.is_empty() {
            entra_lookup.entry(mail).or_insert(id.to_string());
        }
    }

    let adm_idx = emails
        .col_idx("Admission Number")
        .expect("validated by parser");
    let email_idx = emails
        .col_idx("Student email")
        .expect("validated by parser");
    let year_idx = emails
        .col_idx("Year Group Name")
        .expect("validated by parser");
    let mut student_lookup: HashMap<String, (String, String)> = HashMap::new(); // adm -> (email_lower, year)
    for row in &emails.rows {
        let adm = cell(row, adm_idx);
        let email = cell(row, email_idx);
        if !adm.is_empty() && !email.is_empty() {
            student_lookup
                .entry(adm.to_string())
                .or_insert((email.to_lowercase(), cell(row, year_idx).to_string()));
        }
    }

    let cl_adm_idx = class_list
        .col_idx("AdmissionNo")
        .expect("validated by parser");
    let cl_list_idx = class_list
        .col_idx("StudentClassList")
        .expect("validated by parser");
    let cl_year_idx = class_list
        .col_idx("StudentYearGroup")
        .expect("validated by parser");
    let cl_name_idx = class_list
        .col_idx("StudentFullName")
        .expect("validated by parser");

    let mut warnings: Vec<String> = Vec::new();
    let mut students: Vec<StudentRecord> = Vec::new();

    let tags_lower: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();

    for row in &class_list.rows {
        let adm = cell(row, cl_adm_idx);
        if adm.is_empty() {
            continue;
        }
        let name = cell(row, cl_name_idx);

        let Some((email_lower, student_year)) = student_lookup.get(adm) else {
            warnings.push(format!(
                "AdmissionNo {adm} ({name}) in class list but not in emails file"
            ));
            continue;
        };

        let Some(entra_id) = entra_lookup.get(email_lower) else {
            warnings.push(format!("No Entra ID for {email_lower} ({name})"));
            continue;
        };

        // Emails file wins; the class list is the fallback; "Unspecified" last.
        let year_group = match (student_year.as_str(), cell(row, cl_year_idx)) {
            ("", "") => "Unspecified",
            ("", fallback) => fallback,
            (year, _) => year,
        };

        students.push(StudentRecord {
            year_group: year_group.to_string(),
            class_list: cell(row, cl_list_idx).to_string(),
            entra_id: entra_id.clone(),
        });
    }

    let total = student_lookup.len();
    // Distinct students, not class-list rows: an export with one row per class
    // would otherwise count a student once per class and overshoot both `total`
    // and the row count of the file they land in.
    let matched = distinct_students(students.iter());

    // Filter by tags. Kept per-row, because a student's tags are spread across
    // their rows - only the counts collapse to one entry per student.
    let filtered_students: Vec<&StudentRecord> = students
        .iter()
        .filter(|s| {
            let cl_lower = s.class_list.to_lowercase();
            tags_lower.iter().any(|t| cl_lower.contains(t.as_str()))
        })
        .collect();

    let filtered = distinct_students(filtered_students.iter().copied());

    // take(20) on chars, not a byte slice: a multi-byte tag would panic on a
    // char boundary. Sanitized like the year is: tags are typed by hand, and
    // this goes straight into a filename.
    let tag_token: String = sanitize_name(
        &tags
            .iter()
            .take(3)
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join("_")
            .chars()
            .take(20)
            .collect::<String>(),
    );

    let files: Vec<GeneratedFile> = if yeargroup_mode {
        let mut year_map: HashMap<String, HashSet<String>> = HashMap::new();
        for s in &filtered_students {
            year_map
                .entry(s.year_group.clone())
                .or_default()
                .insert(s.entra_id.clone());
        }
        let mut entries: Vec<(String, HashSet<String>)> = year_map.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
            .into_iter()
            .map(|(year, ids)| {
                let mut id_vec: Vec<String> = ids.into_iter().collect();
                id_vec.sort();
                let content = format!("{}\n{}\n", ENTRA_HEADER, id_vec.join("\n"));
                GeneratedFile {
                    name: format!(
                        "distribution_group_{}_{}_{}.csv",
                        sanitize_name(&year),
                        tag_token,
                        timestamp
                    ),
                    content,
                    count: id_vec.len(),
                }
            })
            .collect()
    } else {
        let all_ids: HashSet<String> = filtered_students
            .iter()
            .map(|s| s.entra_id.clone())
            .collect();
        let mut id_vec: Vec<String> = all_ids.into_iter().collect();
        id_vec.sort();
        let content = format!("{}\n{}\n", ENTRA_HEADER, id_vec.join("\n"));
        vec![GeneratedFile {
            name: format!("distribution_group_all_{}_{}.csv", tag_token, timestamp),
            content,
            count: id_vec.len(),
        }]
    };

    // Year group stats (across all filtered students). Deduped like the files
    // are, so in year-group mode each stat equals its file's row count.
    let mut year_counts: HashMap<&str, HashSet<&str>> = HashMap::new();
    for s in &filtered_students {
        year_counts
            .entry(&s.year_group)
            .or_default()
            .insert(&s.entra_id);
    }
    let mut year_groups: Vec<YearGroupStat> = year_counts
        .into_iter()
        .map(|(year, ids)| YearGroupStat {
            year: year.to_string(),
            count: ids.len(),
        })
        .collect();
    year_groups.sort_by(|a, b| a.year.cmp(&b.year));

    Ok(ClassDistResult {
        total,
        matched,
        filtered,
        files,
        year_groups,
        warnings,
    })
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

    fn emails_data() -> ParsedData {
        parsed(
            &["Admission Number", "Student email", "Year Group Name"],
            vec![
                vec!["A1", "Alice@school.org", "Year 7"],
                vec!["A2", "bob@school.org", "Year 8"],
                vec!["A3", "charlie@school.org", "Year 8"],
                // Admission number present, email blank: only one half of the
                // student_lookup guard fails, so && and || disagree here.
                vec!["A5", "", "Year 9"],
                // Hyphenated year survives sanitize(); "Year 7" would not show it.
                vec!["A6", "erin@school.org", "Sixth-Form"],
            ],
        )
    }

    fn class_list_data() -> ParsedData {
        parsed(
            &[
                "AdmissionNo",
                "StudentClassList",
                "StudentYearGroup",
                "StudentFullName",
            ],
            vec![
                vec!["A1", "7MA Biology", "Y7", "Alice A"],
                vec!["A2", "8EN English", "Y8", "Bob B"],
                vec!["A3", "8MA Maths", "Y8", "Charlie C"],
                vec!["A4", "7MA Biology", "Y7", "Dana D"],
                vec!["A6", "12MA Maths", "S6", "Erin E"],
            ],
        )
    }

    fn entra_data() -> ParsedData {
        parsed(
            &["id", "mail"],
            vec![
                vec!["ID-ALICE", "alice@school.org"],
                vec!["ID-BOB", "bob@school.org"],
                // Mail present, ID blank: Charlie stays unmatched only because
                // the guard is && - under || he would resolve to an empty ID.
                vec!["", "charlie@school.org"],
                vec!["ID-ERIN", "erin@school.org"],
            ],
        )
    }

    #[rstest]
    #[case::mixed(" 7MA,8SC\n7MA\tScience ", &["7MA", "8SC", "Science"])]
    #[case::empty("", &[])]
    #[case::separators(", ,\r\n\t\u{2003}", &[])]
    #[case::unicode("数学\u{2003}SCI,数学", &["SCI", "数学"])]
    #[case::case_preserved("mat,MAT,mat", &["MAT", "mat"])]
    fn parse_tags_splits_deduplicates_and_sorts_tokens(
        #[case] input: &str,
        #[case] expected: &[&str],
    ) {
        assert_eq!(parse_tags(input), expected);
    }

    #[rstest]
    #[case::emails_win(" Year 7 ", "Year 8", "Year 7")]
    #[case::class_fallback(" \t", " Year 8 ", "Year 8")]
    #[case::unspecified("", " \t", "Unspecified")]
    fn process_resolves_year_group(
        #[case] email_year: &str,
        #[case] class_year: &str,
        #[case] expected: &str,
        #[values(false, true)] split: bool,
    ) {
        let emails = parsed(
            &["Admission Number", "Student email", "Year Group Name"],
            vec![vec!["A1", " ALICE@school.org ", email_year]],
        );
        let classes = parsed(
            &[
                "AdmissionNo",
                "StudentClassList",
                "StudentYearGroup",
                "StudentFullName",
            ],
            vec![vec![" A1 ", "7mat1", class_year, "Alice"]],
        );
        let result = process(
            &emails,
            &classes,
            &entra_data(),
            &["MAT".into()],
            split,
            "stamp",
        )
        .unwrap();
        assert_eq!((result.total, result.matched, result.filtered), (1, 1, 1));
        assert!(result.warnings.is_empty());
        assert_eq!(result.year_groups[0].year, expected);
        assert_eq!(result.year_groups[0].count, 1);
        assert_eq!(result.files[0].count, 1);
        let bucket = if split {
            sanitize_name(expected)
        } else {
            "all".into()
        };
        assert_eq!(
            result.files[0].name,
            format!("distribution_group_{bucket}_MAT_stamp.csv")
        );
        assert_eq!(
            result.files[0].content,
            format!("{ENTRA_HEADER}\nID-ALICE\n")
        );
    }

    proptest! {
        #[test]
        fn tags_round_trip_as_a_sorted_set(
            tokens in prop::collection::vec("[A-Za-z0-9日本語_-]{1,12}", 0..30),
            separator in prop::sample::select(vec![",", " ", "\t", "\r\n", "\u{2003}"])
        ) {
            let mut expected = tokens.clone();
            expected.sort();
            expected.dedup();
            let input = tokens.join(separator);
            let tags = parse_tags(&format!("{input}{separator}{input}"));
            prop_assert_eq!(&tags, &expected);
            prop_assert_eq!(parse_tags(&tags.join(",")), tags);
        }

        #[test]
        fn distribution_counts_and_members_ignore_duplicate_class_rows(
            students in prop::collection::vec((0u8..4, any::<bool>(), any::<bool>(), 1usize..5), 0..25)
        ) {
            let mut emails = emails_data();
            let mut classes = class_list_data();
            let mut entra = entra_data();
            emails.rows.clear();
            classes.rows.clear();
            entra.rows.clear();
            let mut expected_by_year = std::collections::BTreeMap::<String, Vec<String>>::new();
            let mut matched = 0;
            for (i, (year, selected, has_id, copies)) in students.iter().enumerate() {
                let adm = format!("A{i}");
                let email = format!("student{i}@school.org");
                let id = format!("ID-{i:02}");
                let year = format!("Year {year}");
                emails.rows.push(vec![adm.clone(), email.clone(), year.clone()]);
                let class = if *selected { "7mAt1" } else { "7SCI1" };
                for _ in 0..*copies {
                    classes.rows.push(vec![adm.clone(), class.into(), year.clone(), "Student".into()]);
                }
                if *has_id {
                    entra.rows.push(vec![id.clone(), email]);
                    matched += 1;
                    if *selected {
                        expected_by_year.entry(year).or_default().push(id);
                    }
                }
            }
            let expected_ids = expected_by_year.values().flatten().cloned().collect::<std::collections::BTreeSet<_>>();
            for split in [false, true] {
                let result = process(&emails, &classes, &entra, &["MAT".into()], split, "stamp").unwrap();
                prop_assert_eq!(result.total, students.len());
                prop_assert_eq!(result.matched, matched);
                prop_assert_eq!(result.filtered, expected_ids.len());
                let stats = result.year_groups.iter().map(|s| (s.year.clone(), s.count)).collect::<Vec<_>>();
                let expected_stats = expected_by_year.iter().map(|(year, ids)| (year.clone(), ids.len())).collect::<Vec<_>>();
                prop_assert_eq!(stats, expected_stats);
                prop_assert_eq!(result.files.len(), if split { expected_by_year.len() } else { 1 });
                let mut actual_ids = std::collections::BTreeSet::new();
                for file in &result.files {
                    prop_assert!(file.content.starts_with(&format!("{ENTRA_HEADER}\n")), "missing Entra header");
                    let ids = file.content.lines().skip(3).filter(|line| !line.is_empty()).collect::<Vec<_>>();
                    prop_assert_eq!(file.count, ids.len());
                    prop_assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
                    if split {
                        let (year, expected) = expected_by_year.iter().find(|(year, _)| {
                            file.name == format!("distribution_group_{}_MAT_stamp.csv", sanitize_name(year))
                        }).expect("each file should have an expected year");
                        prop_assert_eq!(&ids, expected, "wrong members for {}", year);
                    }
                    actual_ids.extend(ids.into_iter().map(str::to_string));
                }
                prop_assert_eq!(&actual_ids, &expected_ids);
                classes.rows.reverse();
                let reversed = process(&emails, &classes, &entra, &["MAT".into()], split, "stamp").unwrap();
                let files = |r: ClassDistResult| r.files.into_iter().map(|f| (f.name, f.content, f.count)).collect::<Vec<_>>();
                prop_assert_eq!(files(result), files(reversed));
            }
        }
    }

    #[test]
    fn process_requires_at_least_one_tag() {
        let err = process(
            &emails_data(),
            &class_list_data(),
            &entra_data(),
            &[],
            false,
            "20260510",
        )
        .expect_err("empty tags should fail");

        assert!(matches!(err, ProcessError::NoTags));
    }

    #[test]
    fn process_filters_by_tag_and_generates_combined_file() {
        let result = process(
            &emails_data(),
            &class_list_data(),
            &entra_data(),
            &["MA".to_string()],
            false,
            "20260510",
        )
        .expect("class distribution should process");

        assert_eq!(result.total, 4);
        assert_eq!(result.matched, 3);
        assert_eq!(result.filtered, 2);
        assert_eq!(result.warnings.len(), 2);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("No Entra ID for charlie@school.org"))
        );
        assert!(result.warnings.iter().any(|w| w.contains("AdmissionNo A4")));

        assert_eq!(result.files.len(), 1);
        assert_eq!(
            result.files[0].name,
            "distribution_group_all_MA_20260510.csv"
        );
        assert_eq!(result.files[0].count, 2);
        assert!(result.files[0].content.contains("ID-ALICE"));
        assert!(result.files[0].content.contains("ID-ERIN"));
        assert!(!result.files[0].content.contains("ID-BOB"));

        assert_eq!(
            result
                .year_groups
                .iter()
                .map(|stat| (stat.year.as_str(), stat.count))
                .collect::<Vec<_>>(),
            vec![("Sixth-Form", 1), ("Year 7", 1)]
        );
    }

    #[test]
    fn process_can_split_filtered_students_by_year_group() {
        let result = process(
            &emails_data(),
            &class_list_data(),
            &entra_data(),
            &["EN".to_string(), "MA".to_string()],
            true,
            "20260510",
        )
        .expect("class distribution should process");

        assert_eq!(result.filtered, 3);
        assert_eq!(result.files.len(), 3);
        assert_eq!(
            result
                .files
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "distribution_group_Sixth-Form_EN_MA_20260510.csv",
                "distribution_group_Year_7_EN_MA_20260510.csv",
                "distribution_group_Year_8_EN_MA_20260510.csv"
            ]
        );
        assert!(result.files[0].content.contains("ID-ERIN"));
        assert!(result.files[1].content.contains("ID-ALICE"));
        assert!(result.files[2].content.contains("ID-BOB"));
        assert_eq!(
            result
                .year_groups
                .iter()
                .map(|stat| (stat.year.as_str(), stat.count))
                .collect::<Vec<_>>(),
            vec![("Sixth-Form", 1), ("Year 7", 1), ("Year 8", 1)]
        );
    }

    #[test]
    fn process_truncates_a_multibyte_tag_without_panicking() {
        // 24 chars, 72 bytes: a byte slice at 20 would land mid-character.
        let result = process(
            &emails_data(),
            &class_list_data(),
            &entra_data(),
            &["日本語のクラスタグですよとても長いタグの名前です".to_string()],
            false,
            "20260510",
        )
        .expect("a multi-byte tag should process");

        assert_eq!(
            result.files[0].name,
            "distribution_group_all_日本語のクラスタグですよとても長いタグの_20260510.csv"
        );
        assert_eq!(result.filtered, 0);
    }

    #[test]
    fn process_sanitizes_a_tag_before_it_reaches_the_filename() {
        // Tags are typed by hand, so a path separator can reach the download name.
        let result = process(
            &emails_data(),
            &class_list_data(),
            &entra_data(),
            &["../MA".to_string()],
            false,
            "20260510",
        )
        .expect("an awkward tag should still process");

        assert_eq!(
            result.files[0].name,
            "distribution_group_all____MA_20260510.csv"
        );
    }

    #[test]
    fn process_counts_a_student_once_across_several_class_list_rows() {
        // A class list exported one row per class: Alice appears three times,
        // twice under a matching tag. The files have always deduped her; the
        // stats used to count the rows.
        let class_list = parsed(
            &[
                "AdmissionNo",
                "StudentClassList",
                "StudentYearGroup",
                "StudentFullName",
            ],
            vec![
                vec!["A1", "7MAT1", "Y7", "Alice A"],
                vec!["A1", "7MAT2", "Y7", "Alice A"],
                vec!["A1", "7ENG1", "Y7", "Alice A"],
                vec!["A2", "8MAT1", "Y8", "Bob B"],
            ],
        );

        let result = process(
            &emails_data(),
            &class_list,
            &entra_data(),
            &["MAT".to_string()],
            false,
            "20260510",
        )
        .expect("a duplicated class list should process");

        // Two people, not four rows - and never more than the emails file holds.
        assert_eq!(result.matched, 2);
        assert_eq!(result.filtered, 2);
        assert!(result.matched <= result.total);
        // The stat and the file it describes agree.
        assert_eq!(result.files[0].count, result.filtered);

        let by_year = process(
            &emails_data(),
            &class_list,
            &entra_data(),
            &["MAT".to_string()],
            true,
            "20260510",
        )
        .expect("a duplicated class list should split by year");

        assert_eq!(
            by_year
                .year_groups
                .iter()
                .map(|stat| (stat.year.as_str(), stat.count))
                .collect::<Vec<_>>(),
            vec![("Year 7", 1), ("Year 8", 1)]
        );
        assert_eq!(
            by_year.files.iter().map(|f| f.count).collect::<Vec<_>>(),
            vec![1, 1]
        );
    }
}
