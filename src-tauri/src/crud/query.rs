//! Small shared helpers for safe, parameterised queries.

/// Escape user input before it reaches a `LIKE` pattern so that `%` and `_`
/// are matched literally. Must be paired with `ESCAPE '\'` in the SQL.
pub fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Case-insensitive "contains" pattern for `LOWER(column) LIKE ? ESCAPE '\'`.
pub fn like_contains(input: &str) -> String {
    format!("%{}%", escape_like(&input.to_lowercase()))
}

/// Validate a sort direction, defaulting to descending on anything unexpected.
pub fn sort_direction(value: Option<&str>) -> &'static str {
    if value == Some("asc") {
        "ASC"
    } else {
        "DESC"
    }
}

/// Build a `LIMIT ? OFFSET ?` clause plus its bound values, if a limit was set.
pub fn limit_offset(limit: Option<i64>, offset: Option<i64>) -> (String, Option<(i64, i64)>) {
    match limit {
        Some(limit) if limit > 0 => {
            (" LIMIT ? OFFSET ?".to_string(), Some((limit, offset.unwrap_or(0).max(0))))
        }
        _ => (String::new(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_wildcards() {
        assert_eq!(escape_like("100%_done\\x"), "100\\%\\_done\\\\x");
        assert_eq!(like_contains("50%"), "%50\\%%");
        assert_eq!(like_contains("ABC"), "%abc%");
    }

    #[test]
    fn validates_direction_and_limit() {
        assert_eq!(sort_direction(Some("asc")), "ASC");
        assert_eq!(sort_direction(Some("desc")), "DESC");
        assert_eq!(sort_direction(Some("'; DROP TABLE")), "DESC");
        assert_eq!(limit_offset(None, None), (String::new(), None));
        assert_eq!(limit_offset(Some(0), None), (String::new(), None));
        assert_eq!(
            limit_offset(Some(50), Some(-3)),
            (" LIMIT ? OFFSET ?".to_string(), Some((50, 0)))
        );
    }
}
