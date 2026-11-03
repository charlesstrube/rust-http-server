use crate::http2::hpack::HpackError;

static STATIC_TABLE: &[(&str, &str)] = &[
    (":authority", ""),                   // 1
    (":method", "GET"),                   // 2
    (":method", "POST"),                  // 3
    (":path", "/"),                       // 4
    (":path", "/index.html"),             // 5
    (":scheme", "http"),                  // 6
    (":scheme", "https"),                 // 7
    (":status", "200"),                   // 8
    (":status", "204"),                   // 9
    (":status", "206"),                   // 10
    (":status", "304"),                   // 11
    (":status", "400"),                   // 12
    (":status", "404"),                   // 13
    (":status", "500"),                   // 14
    ("accept-charset", ""),               // 15
    ("accept-encoding", "gzip, deflate"), // 16
    ("accept-language", ""),              // 17
    ("accept-ranges", ""),                // 18
    ("accept", ""),                       // 19
    ("access-control-allow-origin", ""),  // 20
    ("age", ""),                          // 21
    ("allow", ""),                        // 22
    ("authorization", ""),                // 23
    ("cache-control", ""),                // 24
    ("content-disposition", ""),          // 25
    ("content-encoding", ""),             // 26
    ("content-language", ""),             // 27
    ("content-length", ""),               // 28
    ("content-location", ""),             // 29
    ("content-range", ""),                // 30
    ("content-type", ""),                 // 31
    ("cookie", ""),                       // 32
    ("date", ""),                         // 33
    ("etag", ""),                         // 34
    ("expect", ""),                       // 35
    ("expires", ""),                      // 36
    ("from", ""),                         // 37
    ("host", ""),                         // 38
    ("if-match", ""),                     // 39
    ("if-modified-since", ""),            // 40
    ("if-none-match", ""),                // 41
    ("if-range", ""),                     // 42
    ("if-unmodified-since", ""),          // 43
    ("last-modified", ""),                // 44
    ("link", ""),                         // 45
    ("location", ""),                     // 46
    ("max-forwards", ""),                 // 47
    ("proxy-authenticate", ""),           // 48
    ("proxy-authorization", ""),          // 49
    ("range", ""),                        // 50
    ("referer", ""),                      // 51
    ("refresh", ""),                      // 52
    ("retry-after", ""),                  // 53
    ("server", ""),                       // 54
    ("set-cookie", ""),                   // 55
    ("strict-transport-security", ""),    // 56
    ("transfer-encoding", ""),            // 57
    ("user-agent", ""),                   // 58
    ("vary", ""),                         // 59
    ("via", ""),                          // 60
    ("www-authenticate", ""),             // 61
];

// RFC 7541 Appendix A: returns the (name, value) pair at `index` (1..=61).
// An empty value means the entry only provides a name.
pub fn get(index: usize) -> Result<(&'static str, &'static str), HpackError> {
    let index = index.checked_sub(1).ok_or(HpackError::InvalidIndex)?;

    match STATIC_TABLE.get(index) {
        Some(value) => Ok(*value),
        None => Err(HpackError::InvalidIndex),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // --- Bounds ---

    // RFC 7541 §6.1: index 0 is never valid.
    #[test]
    fn index_0_is_invalid() {
        assert_eq!(get(0), Err(HpackError::InvalidIndex));
    }

    #[test]
    fn first_entry() {
        assert_eq!(get(1), Ok((":authority", "")));
    }

    #[test]
    fn last_entry() {
        assert_eq!(get(61), Ok(("www-authenticate", "")));
    }

    // 62 and above belong to the dynamic table (sub-step 4.5), not to this one.
    #[test]
    fn index_62_is_invalid() {
        assert_eq!(get(62), Err(HpackError::InvalidIndex));
    }

    // The index comes from the peer: a huge value must not panic.
    #[test]
    fn huge_index_is_invalid() {
        assert_eq!(get(usize::MAX), Err(HpackError::InvalidIndex));
    }

    // --- Entries with a value ---

    // C.2.4: 0x82 = indexed field, index 2.
    #[test]
    fn method_get() {
        assert_eq!(get(2), Ok((":method", "GET")));
    }

    #[test]
    fn method_post() {
        assert_eq!(get(3), Ok((":method", "POST")));
    }

    #[test]
    fn paths() {
        assert_eq!(get(4), Ok((":path", "/")));
        assert_eq!(get(5), Ok((":path", "/index.html")));
    }

    #[test]
    fn schemes() {
        assert_eq!(get(6), Ok((":scheme", "http")));
        assert_eq!(get(7), Ok((":scheme", "https")));
    }

    #[test]
    fn statuses() {
        let statuses: Vec<_> = (8..=14).map(|index| get(index).unwrap()).collect();

        assert_eq!(
            statuses,
            [
                (":status", "200"),
                (":status", "204"),
                (":status", "206"),
                (":status", "304"),
                (":status", "400"),
                (":status", "404"),
                (":status", "500"),
            ]
        );
    }

    #[test]
    fn accept_encoding() {
        assert_eq!(get(16), Ok(("accept-encoding", "gzip, deflate")));
    }

    // --- Name-only entries ---

    // C.3.2: 0x58 = literal with indexing, name from index 24.
    #[test]
    fn cache_control() {
        assert_eq!(get(24), Ok(("cache-control", "")));
    }

    #[test]
    fn common_request_headers() {
        assert_eq!(get(28), Ok(("content-length", "")));
        assert_eq!(get(32), Ok(("cookie", "")));
        assert_eq!(get(38), Ok(("host", "")));
        assert_eq!(get(58), Ok(("user-agent", "")));
    }

    // --- Whole table ---

    // Only the pseudo-headers 2..=14 and accept-encoding (16) carry a value.
    #[test]
    fn only_expected_entries_have_a_value() {
        for index in 1..=61 {
            let (_, value) = get(index).unwrap();
            let expected = (2..=14).contains(&index) || index == 16;

            assert_eq!(!value.is_empty(), expected, "index {index}");
        }
    }

    // RFC 9113 §8.2.1: HTTP/2 field names are lowercase.
    #[test]
    fn names_are_lowercase_and_not_empty() {
        for index in 1..=61 {
            let (name, _) = get(index).unwrap();

            assert!(!name.is_empty(), "index {index}");
            assert_eq!(name, name.to_lowercase(), "index {index}");
        }
    }
}
