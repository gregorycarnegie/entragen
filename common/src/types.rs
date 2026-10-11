#[derive(Debug, Clone)]
pub struct ParsedData {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl ParsedData {
    pub fn col_idx(&self, name: &str) -> Option<usize> {
        self.headers.iter().position(|h| h == name)
    }

    pub fn get(&self, row: &[String], name: &str) -> String {
        self.col_idx(name)
            .and_then(|i| row.get(i))
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::first("Name", &["Ada", "ada@example.com"], Some(0), "Ada")]
    #[case::second("Email", &["Ada", "ada@example.com"], Some(1), "ada@example.com")]
    #[case::missing_column("House", &["Ada"], None, "")]
    #[case::case_sensitive("name", &["Ada"], None, "")]
    #[case::short_row("Email", &["Ada"], Some(1), "")]
    #[case::empty_row("Name", &[], Some(0), "")]
    fn parsed_data_finds_columns_and_returns_default_for_missing_values(
        #[case] column: &str,
        #[case] row: &[&str],
        #[case] index: Option<usize>,
        #[case] expected: &str,
    ) {
        let data = ParsedData {
            headers: vec!["Name".to_string(), "Email".to_string()],
            rows: vec![row.iter().map(|s| s.to_string()).collect()],
        };

        assert_eq!(data.col_idx(column), index);
        assert_eq!(data.get(&data.rows[0], column), expected);
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedFile {
    pub name: String,
    pub content: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct MissingMatch {
    pub email: String,
    pub house: String,
    pub year: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct HouseTeamsResult {
    pub processed: usize,
    pub matched: usize,
    pub missing_count: usize,
    pub group_count: usize,
    pub files: Vec<GeneratedFile>,
    pub missing: Vec<MissingMatch>,
}

#[derive(Debug, Clone)]
pub struct YearGroupStat {
    pub year: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct ClassDistResult {
    pub total: usize,
    pub matched: usize,
    pub filtered: usize,
    pub files: Vec<GeneratedFile>,
    pub year_groups: Vec<YearGroupStat>,
    pub warnings: Vec<String>,
}
