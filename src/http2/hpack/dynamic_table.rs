use std::collections::VecDeque;

use crate::http2::hpack::HpackError;

pub const SETTINGS_HEADER_TABLE_SIZE: usize = 4096;

// RFC 7541 §2.3.2 and §4: the per-connection table of recently seen headers.
// The fields are yours to choose.
pub struct DynamicTable {
    value: VecDeque<(Vec<u8>, Vec<u8>)>,
    max_size: usize,
    limit: usize,
}

fn get_size(name: &Vec<u8>, value: &Vec<u8>) -> usize {
    name.len() + value.len() + 32
}

impl DynamicTable {
    // `limit` is the SETTINGS_HEADER_TABLE_SIZE we announced: the maximum size
    // starts there, and a size update can never go above it.
    pub fn new(limit: usize) -> Self {
        Self {
            value: VecDeque::new(),
            max_size: limit,
            limit,
        }
    }

    // Returns the entry at `index`, 1 = most recent (the HPACK index minus 61).
    pub fn get(&self, index: usize) -> Result<(&[u8], &[u8]), HpackError> {
        if index < 1 {
            return Err(HpackError::InvalidIndex);
        }

        let (name, value) = &self.value.get(index - 1).ok_or(HpackError::InvalidIndex)?;
        Ok((name.as_slice(), value.as_slice()))
    }

    // Adds an entry as the most recent one, evicting the oldest ones to make room.
    pub fn insert(&mut self, name: Vec<u8>, value: Vec<u8>) {
        let header_size = get_size(&name, &value);

        while self.size() + header_size > self.max_size {
            if self.value.pop_back().is_none() {
                return;
            }
        }

        self.value.push_front((name, value));
    }

    // RFC 7541 §6.3: dynamic table size update sent by the peer.
    pub fn set_max_size(&mut self, max_size: usize) -> Result<(), HpackError> {
        if max_size > self.limit {
            return Err(HpackError::InvalidTableSize);
        }

        let mut size = 0usize;
        for (index, (name, value)) in self.value.iter().enumerate() {
            size += name.len() + value.len() + 32usize;

            if size > max_size {
                self.value.drain(index..self.value.len());
                break;
            }
        }

        self.max_size = max_size;

        Ok(())
    }

    // Sum of the entry sizes (name + value + 32 each).
    pub fn size(&self) -> usize {
        let mut size = 0usize;
        for (name, value) in self.value.iter() {
            size += get_size(&name, &value);
        }

        size
    }

    pub fn len(&self) -> usize {
        self.value.len()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const DEFAULT_LIMIT: usize = 4_096;

    fn table_with(limit: usize, entries: &[(&str, &str)]) -> DynamicTable {
        let mut table = DynamicTable::new(limit);
        for (name, value) in entries {
            table.insert(name.as_bytes().to_vec(), value.as_bytes().to_vec());
        }
        table
    }

    // Every entry from index 1 (most recent) to the last one, as strings.
    fn entries(table: &DynamicTable) -> Vec<(String, String)> {
        (1..=table.len())
            .map(|index| {
                let (name, value) = table.get(index).unwrap();
                (
                    String::from_utf8(name.to_vec()).unwrap(),
                    String::from_utf8(value.to_vec()).unwrap(),
                )
            })
            .collect()
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    // --- Empty table ---

    #[test]
    fn new_table_is_empty() {
        let table = DynamicTable::new(DEFAULT_LIMIT);

        assert_eq!(table.len(), 0);
        assert_eq!(table.size(), 0);
    }

    #[test]
    fn empty_table_has_no_entry() {
        let table = DynamicTable::new(DEFAULT_LIMIT);

        assert_eq!(table.get(1), Err(HpackError::InvalidIndex));
    }

    // --- Indexing ---

    #[test]
    fn inserted_entry_is_at_index_1() {
        let table = table_with(DEFAULT_LIMIT, &[("custom-key", "custom-header")]);

        assert_eq!(
            table.get(1),
            Ok((&b"custom-key"[..], &b"custom-header"[..]))
        );
    }

    // The most recent entry is always 1: older ones shift up.
    #[test]
    fn newer_entries_push_older_ones_up() {
        let table = table_with(DEFAULT_LIMIT, &[("a", "1"), ("b", "2"), ("c", "3")]);

        assert_eq!(
            entries(&table),
            pairs(&[("c", "3"), ("b", "2"), ("a", "1")])
        );
    }

    // Like the static table, indices start at 1.
    #[test]
    fn index_0_is_invalid() {
        let table = table_with(DEFAULT_LIMIT, &[("a", "1")]);

        assert_eq!(table.get(0), Err(HpackError::InvalidIndex));
    }

    #[test]
    fn index_past_the_last_entry_is_invalid() {
        let table = table_with(DEFAULT_LIMIT, &[("a", "1"), ("b", "2")]);

        assert_eq!(table.get(3), Err(HpackError::InvalidIndex));
    }

    // The index comes from the peer: a huge value must not panic.
    #[test]
    fn huge_index_is_invalid() {
        let table = table_with(DEFAULT_LIMIT, &[("a", "1")]);

        assert_eq!(table.get(usize::MAX), Err(HpackError::InvalidIndex));
    }

    // --- Size (RFC 7541 §4.1) ---

    // C.3.1: 10 + 15 + 32 = 57.
    #[test]
    fn entry_size_includes_a_32_byte_overhead() {
        let table = table_with(DEFAULT_LIMIT, &[(":authority", "www.example.com")]);

        assert_eq!(table.size(), 57);
    }

    // An empty value still costs the name and the overhead.
    #[test]
    fn empty_value_still_costs_the_overhead() {
        let table = table_with(DEFAULT_LIMIT, &[("cache-control", "")]);

        assert_eq!(table.size(), 13 + 32);
    }

    // --- RFC 7541 Appendix C.3: three requests, no eviction ---

    #[test]
    fn c_3_table_after_three_requests() {
        let table = table_with(
            DEFAULT_LIMIT,
            &[
                (":authority", "www.example.com"), // C.3.1
                ("cache-control", "no-cache"),     // C.3.2
                ("custom-key", "custom-value"),    // C.3.3
            ],
        );

        assert_eq!(
            entries(&table),
            pairs(&[
                ("custom-key", "custom-value"),
                ("cache-control", "no-cache"),
                (":authority", "www.example.com"),
            ])
        );
        assert_eq!(table.size(), 164);
    }

    // --- RFC 7541 Appendix C.5: responses with a 256-byte table ---

    const C_5_LIMIT: usize = 256;

    const C_5_1: [(&str, &str); 4] = [
        (":status", "302"),
        ("cache-control", "private"),
        ("date", "Mon, 21 Oct 2013 20:13:21 GMT"),
        ("location", "https://www.example.com"),
    ];

    #[test]
    fn c_5_1_fills_the_table_without_eviction() {
        let table = table_with(C_5_LIMIT, &C_5_1);

        assert_eq!(table.len(), 4);
        assert_eq!(table.size(), 222);
    }

    // 222 + 42 = 264 > 256: the oldest entry (:status 302) is evicted.
    #[test]
    fn c_5_2_evicts_the_oldest_entry() {
        let mut table = table_with(C_5_LIMIT, &C_5_1);
        table.insert(b":status".to_vec(), b"307".to_vec());

        assert_eq!(
            entries(&table),
            pairs(&[
                (":status", "307"),
                ("location", "https://www.example.com"),
                ("date", "Mon, 21 Oct 2013 20:13:21 GMT"),
                ("cache-control", "private"),
            ])
        );
        assert_eq!(table.size(), 222);
    }

    // The set-cookie entry (98 bytes) needs two evictions on its own.
    #[test]
    fn c_5_3_evicts_several_entries_for_one_insert() {
        let mut table = table_with(C_5_LIMIT, &C_5_1);
        for (name, value) in [
            (":status", "307"),
            ("date", "Mon, 21 Oct 2013 20:13:22 GMT"),
            ("content-encoding", "gzip"),
            (
                "set-cookie",
                "foo=ASDJKHQKBZXOQWEOPIUAXQWEOIU; max-age=3600; version=1",
            ),
        ] {
            table.insert(name.as_bytes().to_vec(), value.as_bytes().to_vec());
        }

        assert_eq!(
            entries(&table),
            pairs(&[
                (
                    "set-cookie",
                    "foo=ASDJKHQKBZXOQWEOPIUAXQWEOIU; max-age=3600; version=1"
                ),
                ("content-encoding", "gzip"),
                ("date", "Mon, 21 Oct 2013 20:13:22 GMT"),
            ])
        );
        assert_eq!(table.size(), 215);
    }

    // --- Eviction limits (RFC 7541 §4.4) ---

    // 16 + 16 + 32 = 64: an entry exactly the size of the table fits.
    #[test]
    fn entry_exactly_as_large_as_the_table_fits() {
        let table = table_with(64, &[("aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb")]);

        assert_eq!(table.len(), 1);
        assert_eq!(table.size(), 64);
    }

    // Not an error: the table is emptied and the entry is not added.
    #[test]
    fn entry_larger_than_the_table_empties_it() {
        let mut table = table_with(64, &[("a", "1")]);
        table.insert(b"aaaaaaaaaaaaaaaa".to_vec(), b"bbbbbbbbbbbbbbbbb".to_vec());

        assert_eq!(table.len(), 0);
        assert_eq!(table.size(), 0);
    }

    // A literal with indexing may take its name from the very entry the insert evicts:
    // the name must be copied before anything is evicted.
    #[test]
    fn name_taken_from_an_entry_that_the_insert_evicts() {
        let mut table = table_with(64, &[("aaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbb")]);
        let name = table.get(1).unwrap().0.to_vec();
        table.insert(name, b"c".to_vec());

        assert_eq!(entries(&table), pairs(&[("aaaaaaaaaaaaaaaa", "c")]));
    }

    // --- Size update (RFC 7541 §4.3 and §6.3) ---

    #[test]
    fn size_update_to_0_empties_the_table() {
        let mut table = table_with(DEFAULT_LIMIT, &[("a", "1"), ("b", "2")]);

        assert_eq!(table.set_max_size(0), Ok(()));
        assert_eq!(table.len(), 0);
        assert_eq!(table.size(), 0);
    }

    // Each entry is 34 bytes: a 70-byte maximum keeps the two most recent.
    #[test]
    fn shrinking_evicts_the_oldest_entries() {
        let mut table = table_with(DEFAULT_LIMIT, &[("a", "1"), ("b", "2"), ("c", "3")]);

        assert_eq!(table.set_max_size(70), Ok(()));
        assert_eq!(entries(&table), pairs(&[("c", "3"), ("b", "2")]));
        assert_eq!(table.size(), 68);
    }

    // After shrinking, inserts respect the new maximum, not the limit.
    #[test]
    fn inserts_respect_the_new_maximum() {
        let mut table = DynamicTable::new(DEFAULT_LIMIT);
        table.set_max_size(70).unwrap();
        for (name, value) in [("a", "1"), ("b", "2"), ("c", "3")] {
            table.insert(name.as_bytes().to_vec(), value.as_bytes().to_vec());
        }

        assert_eq!(entries(&table), pairs(&[("c", "3"), ("b", "2")]));
    }

    // Growing back up to the limit is allowed and keeps every entry.
    #[test]
    fn growing_back_to_the_limit_keeps_the_entries() {
        let mut table = table_with(DEFAULT_LIMIT, &[("a", "1")]);
        table.set_max_size(100).unwrap();

        assert_eq!(table.set_max_size(DEFAULT_LIMIT), Ok(()));
        assert_eq!(entries(&table), pairs(&[("a", "1")]));
    }

    // RFC 7541 §6.3: above SETTINGS_HEADER_TABLE_SIZE is a decoding error.
    #[test]
    fn size_update_above_the_limit_is_rejected() {
        let mut table = table_with(DEFAULT_LIMIT, &[("a", "1")]);

        assert_eq!(
            table.set_max_size(DEFAULT_LIMIT + 1),
            Err(HpackError::InvalidTableSize)
        );
        assert_eq!(entries(&table), pairs(&[("a", "1")]));
    }
}
