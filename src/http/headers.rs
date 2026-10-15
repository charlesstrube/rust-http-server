use std::fmt::Display;

#[derive(Default)]
pub struct Headers(Vec<(String, String)>);

impl From<Vec<(String, String)>> for Headers {
    fn from(value: Vec<(String, String)>) -> Self {
        Self(value)
    }
}

impl Display for Headers {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for (key, value) in &self.0 {
            write!(f, "{}: {}\r\n", key, value)?;
        }
        Ok(())
    }
}

impl Headers {
    pub fn new(values: Vec<(String, String)>) -> Self {
        Self(values)
    }

    pub fn add(&mut self, key: &str, value: &str) {
        self.0.push((key.into(), value.into()));
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find_map(|(k, v)| {
            if k.eq_ignore_ascii_case(key) {
                Some(v.as_str())
            } else {
                None
            }
        })
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let mut found = false;
        self.0.retain_mut(|(current_key, current_value)| {
            if current_key.eq_ignore_ascii_case(key) {
                if found == false {
                    *current_value = value.into();
                    found = true;
                    return true;
                }

                false
            } else {
                true
            }
        });

        if found == false {
            self.add(key, value);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // Case-insensitive count of field lines named `name` in the serialized headers.
    fn count_lines(headers: &Headers, name: &str) -> usize {
        let prefix = format!("{}:", name.to_ascii_lowercase());
        headers
            .to_string()
            .to_ascii_lowercase()
            .split("\r\n")
            .filter(|line| line.starts_with(&prefix))
            .count()
    }

    #[test]
    fn new_is_empty() {
        assert_eq!(Headers::new(vec![]).to_string(), "");
    }

    #[test]
    fn default_is_empty() {
        assert_eq!(Headers::default().to_string(), "");
    }

    #[test]
    fn get_returns_added_value() {
        let headers = Headers::new(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.get("Content-Type"), Some("text/plain"));
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    #[test]
    fn get_is_case_insensitive() {
        let headers = Headers::new(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.get("content-type"), Some("text/plain"));
        assert_eq!(headers.get("CONTENT-TYPE"), Some("text/plain"));
    }

    #[test]
    fn get_missing_is_none() {
        assert_eq!(Headers::default().get("Content-Type"), None);
    }

    #[test]
    fn displays_one_crlf_terminated_line_per_header() {
        let headers = Headers::new(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.to_string(), "Content-Type: text/plain\r\n");
    }

    #[test]
    fn displays_in_insertion_order() {
        let headers = Headers::new(vec![("B".into(), "2".into()), ("A".into(), "1".into())]);

        assert_eq!(headers.to_string(), "B: 2\r\nA: 1\r\n");
    }

    #[test]
    fn displays_name_with_original_case() {
        let headers = Headers::new(vec![("x-CuStOm".into(), "v".into())]);

        assert_eq!(headers.to_string(), "x-CuStOm: v\r\n");
    }

    // RFC 9110 §5.3: a field may appear several times (e.g. Set-Cookie).
    #[test]
    fn add_keeps_duplicates() {
        let mut headers = Headers::default();
        headers.add("Set-Cookie", "a=1");
        headers.add("Set-Cookie", "b=2");

        assert_eq!(
            headers.to_string(),
            "Set-Cookie: a=1\r\nSet-Cookie: b=2\r\n"
        );
    }

    #[test]
    fn get_returns_first_value_of_duplicates() {
        let headers = Headers::new(vec![
            ("Set-Cookie".into(), "a=1".into()),
            ("Set-Cookie".into(), "b=2".into()),
        ]);

        assert_eq!(headers.get("Set-Cookie"), Some("a=1"));
    }

    #[test]
    fn set_adds_missing_header() {
        let mut headers = Headers::default();
        headers.set("Content-Length", "3");

        assert_eq!(headers.get("Content-Length"), Some("3"));
    }

    #[test]
    fn set_replaces_existing_value() {
        let mut headers = Headers::default();
        headers.add("Content-Length", "999");
        headers.set("Content-Length", "3");

        assert_eq!(headers.get("Content-Length"), Some("3"));
        assert_eq!(count_lines(&headers, "Content-Length"), 1);
    }

    #[test]
    fn set_matches_existing_name_case_insensitively() {
        let mut headers = Headers::default();
        headers.add("content-length", "999");
        headers.set("Content-Length", "3");

        assert_eq!(headers.get("Content-Length"), Some("3"));
        assert_eq!(count_lines(&headers, "Content-Length"), 1);
    }

    #[test]
    fn set_collapses_duplicates_into_one() {
        let mut headers = Headers::default();
        headers.add("Content-Length", "1");
        headers.add("Content-Length", "2");
        headers.set("Content-Length", "3");

        assert_eq!(count_lines(&headers, "Content-Length"), 1);
        assert_eq!(headers.get("Content-Length"), Some("3"));
    }

    #[test]
    fn set_leaves_other_headers_untouched() {
        let mut headers = Headers::default();
        headers.add("Content-Type", "text/plain");
        headers.set("Content-Length", "3");

        assert_eq!(headers.get("Content-Type"), Some("text/plain"));
    }
}
