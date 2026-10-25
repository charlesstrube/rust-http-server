pub mod encoding;

#[derive(Default, Clone)]
pub struct Headers(Vec<(String, String)>);

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ParseHeadersError {
    #[error("Missing colon in header line")]
    MissingColon,
    #[error("Invalid header name")]
    InvalidName,
}

impl From<Vec<(String, String)>> for Headers {
    fn from(value: Vec<(String, String)>) -> Self {
        Self(value)
    }
}

impl Headers {
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

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub fn delete(&mut self, key: &str) {
        self.0
            .retain_mut(|(current_key, _)| !current_key.eq_ignore_ascii_case(key));
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let mut found = false;
        self.0.retain_mut(|(current_key, current_value)| {
            if current_key.eq_ignore_ascii_case(key) {
                if !found {
                    *current_value = value.into();
                    found = true;
                    return true;
                }

                false
            } else {
                true
            }
        });

        if !found {
            self.add(key, value);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // Case-insensitive count of fields named `name`.
    fn count_lines(headers: &Headers, name: &str) -> usize {
        headers
            .0
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .count()
    }

    #[test]
    fn new_is_empty() {
        assert!(Headers::from(vec![]).0.is_empty());
    }

    #[test]
    fn default_is_empty() {
        assert!(Headers::default().0.is_empty());
    }

    #[test]
    fn get_returns_added_value() {
        let headers = Headers::from(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.get("Content-Type"), Some("text/plain"));
    }

    // RFC 9110 §5.1: field names are case-insensitive.
    #[test]
    fn get_is_case_insensitive() {
        let headers = Headers::from(vec![("Content-Type".into(), "text/plain".into())]);

        assert_eq!(headers.get("content-type"), Some("text/plain"));
        assert_eq!(headers.get("CONTENT-TYPE"), Some("text/plain"));
    }

    #[test]
    fn get_missing_is_none() {
        assert_eq!(Headers::default().get("Content-Type"), None);
    }

    // RFC 9110 §5.3: a field may appear several times (e.g. Set-Cookie).
    #[test]
    fn add_keeps_duplicates() {
        let mut headers = Headers::default();
        headers.add("Set-Cookie", "a=1");
        headers.add("Set-Cookie", "b=2");

        assert_eq!(
            headers.0,
            vec![
                ("Set-Cookie".to_string(), "a=1".to_string()),
                ("Set-Cookie".to_string(), "b=2".to_string()),
            ]
        );
    }

    #[test]
    fn get_returns_first_value_of_duplicates() {
        let headers = Headers::from(vec![
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
