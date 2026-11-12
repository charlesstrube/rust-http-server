use crate::http2::frame::DEFAULT_MAX_FRAME_SIZE;

enum SettingId {
    HeaderTableSize,
    EnablePush,
    MaxConcurrentStreams,
    InitialWindowSize,
    MaxFrameSize,
    MaxHeaderListSize,
    Unknown(u16),
}

impl From<u16> for SettingId {
    fn from(value: u16) -> Self {
        match value {
            0x1 => SettingId::HeaderTableSize,
            0x2 => SettingId::EnablePush,
            0x3 => SettingId::MaxConcurrentStreams,
            0x4 => SettingId::InitialWindowSize,
            0x5 => SettingId::MaxFrameSize,
            0x6 => SettingId::MaxHeaderListSize,
            value => SettingId::Unknown(value),
        }
    }
}

impl From<SettingId> for u16 {
    fn from(value: SettingId) -> Self {
        match value {
            SettingId::HeaderTableSize => 0x1,
            SettingId::EnablePush => 0x2,
            SettingId::MaxConcurrentStreams => 0x3,
            SettingId::InitialWindowSize => 0x4,
            SettingId::MaxFrameSize => 0x5,
            SettingId::MaxHeaderListSize => 0x6,
            SettingId::Unknown(value) => value,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Settings {
    header_table_size: u32,
    enable_push: bool,
    max_concurrent_streams: Option<u32>,
    initial_window_size: u32,
    max_frame_size: u32,
    max_header_list_size: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            header_table_size: 4_096,
            enable_push: true,
            max_concurrent_streams: None,
            initial_window_size: 65_535,
            max_frame_size: DEFAULT_MAX_FRAME_SIZE,
            max_header_list_size: None,
        }
    }
}

impl Settings {
    pub fn max_frame_size(&self) -> u32 {
        self.max_frame_size
    }

    pub fn apply(&mut self, payload: &[u8]) -> Result<(), SettingsError> {
        let (chunks, remainder) = payload.as_chunks::<6>();
        if !remainder.is_empty() {
            return Err(SettingsError::Length);
        }
        for chunk in chunks {
            let id = u16::from_be_bytes([chunk[0], chunk[1]]);
            let value = u32::from_be_bytes([chunk[2], chunk[3], chunk[4], chunk[5]]);

            match SettingId::from(id) {
                SettingId::HeaderTableSize => self.header_table_size = value,
                SettingId::EnablePush => {
                    self.enable_push = match value {
                        0 => false,
                        1 => true,
                        _ => return Err(SettingsError::EnablePush),
                    }
                }
                SettingId::MaxConcurrentStreams => self.max_concurrent_streams = Some(value),
                SettingId::InitialWindowSize => {
                    if value > 0x7FFF_FFFF {
                        return Err(SettingsError::InitialWindowSize);
                    }
                    self.initial_window_size = value;
                }
                SettingId::MaxFrameSize => {
                    if !(16_384..=16_777_215).contains(&value) {
                        return Err(SettingsError::MaxFrameSize);
                    }
                    self.max_frame_size = value;
                }
                SettingId::MaxHeaderListSize => self.max_header_list_size = Some(value),
                // RFC 9113 §6.5.2: unknown settings MUST be ignored
                SettingId::Unknown(_) => (),
            }
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SettingsError {
    #[error("Invalid Length")]
    Length,
    #[error("Invalid Enable Push")]
    EnablePush,
    #[error("Invalid Initial Window Size")]
    InitialWindowSize,
    #[error("Invalid Max Frame Size")]
    MaxFrameSize,
}

#[cfg(test)]
mod test {
    use super::*;

    // RFC 9113 §6.5.1: a setting is a 16-bit identifier followed by a 32-bit value.
    fn entry(id: u16, value: u32) -> Vec<u8> {
        let mut bytes = id.to_be_bytes().to_vec();
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    fn applied(payload: &[u8]) -> Result<Settings, SettingsError> {
        let mut settings = Settings::default();
        settings.apply(payload)?;
        Ok(settings)
    }

    // --- Defaults (RFC 9113 §6.5.2) ---

    #[test]
    fn defaults_match_the_rfc() {
        let settings = Settings::default();

        assert_eq!(settings.header_table_size, 4_096);
        assert!(settings.enable_push);
        assert_eq!(settings.max_concurrent_streams, None); // unlimited
        assert_eq!(settings.initial_window_size, 65_535);
        assert_eq!(settings.max_frame_size, 16_384);
        assert_eq!(settings.max_header_list_size, None); // unlimited
    }

    #[test]
    fn empty_payload_changes_nothing() {
        let settings = applied(&[]).unwrap();

        assert_eq!(settings, Settings::default());
    }

    // --- Applying known identifiers ---

    #[test]
    fn applies_each_known_setting() {
        let payload = [
            entry(0x1, 8_192),
            entry(0x2, 0),
            entry(0x3, 100),
            entry(0x4, 1_000_000),
            entry(0x5, 32_768),
            entry(0x6, 10_000),
        ]
        .concat();

        let settings = applied(&payload).unwrap();

        assert_eq!(settings.header_table_size, 8_192);
        assert!(!settings.enable_push);
        assert_eq!(settings.max_concurrent_streams, Some(100));
        assert_eq!(settings.initial_window_size, 1_000_000);
        assert_eq!(settings.max_frame_size, 32_768);
        assert_eq!(settings.max_header_list_size, Some(10_000));
    }

    // RFC 9113 §6.5.3: values are processed in the order they appear.
    #[test]
    fn last_occurrence_of_an_identifier_wins() {
        let payload = [entry(0x1, 1_000), entry(0x1, 2_000)].concat();

        let settings = applied(&payload).unwrap();

        assert_eq!(settings.header_table_size, 2_000);
    }

    // A SETTINGS frame is a delta: what it does not mention keeps its current value.
    #[test]
    fn unmentioned_settings_keep_their_previous_value() {
        let mut settings = Settings::default();

        settings.apply(&entry(0x5, 20_000)).unwrap();
        settings.apply(&entry(0x1, 0)).unwrap();

        assert_eq!(settings.max_frame_size, 20_000);
        assert_eq!(settings.header_table_size, 0);
    }

    // RFC 9113 §6.5.2: "An endpoint that receives a SETTINGS frame with any unknown
    // or unsupported identifier MUST ignore that setting."
    #[test]
    fn unknown_identifier_is_ignored() {
        let payload = [entry(0x42, 123), entry(0x1, 1_024), entry(0xffff, 7)].concat();

        let settings = applied(&payload).unwrap();

        assert_eq!(
            settings,
            Settings {
                header_table_size: 1_024,
                ..Settings::default()
            }
        );
    }

    // 0 is a legal value: the peer forbids us from opening streams, it is not "unlimited".
    #[test]
    fn zero_max_concurrent_streams_is_allowed() {
        let settings = applied(&entry(0x3, 0)).unwrap();

        assert_eq!(settings.max_concurrent_streams, Some(0));
    }

    // --- Validation ---

    // RFC 9113 §6.5: a length that is not a multiple of 6 is a FRAME_SIZE_ERROR.
    #[test]
    fn payload_not_a_multiple_of_six_is_rejected() {
        for length in [1, 5, 7, 11] {
            let payload = vec![0u8; length];

            assert_eq!(
                applied(&payload),
                Err(SettingsError::Length),
                "for length {length}"
            );
        }
    }

    // RFC 9113 §6.5.2: any ENABLE_PUSH value other than 0 or 1 is a PROTOCOL_ERROR.
    #[test]
    fn enable_push_accepts_only_zero_or_one() {
        assert!(!applied(&entry(0x2, 0)).unwrap().enable_push);
        assert!(applied(&entry(0x2, 1)).unwrap().enable_push);

        assert_eq!(applied(&entry(0x2, 2)), Err(SettingsError::EnablePush));
    }

    // RFC 9113 §6.5.2: above 2^31-1 is a FLOW_CONTROL_ERROR.
    #[test]
    fn initial_window_size_is_capped_at_31_bits() {
        let max = 0x7fff_ffff;

        assert_eq!(applied(&entry(0x4, max)).unwrap().initial_window_size, max);
        assert_eq!(
            applied(&entry(0x4, max + 1)),
            Err(SettingsError::InitialWindowSize)
        );
    }

    // RFC 9113 §6.5.2: must stay within [2^14, 2^24-1], otherwise PROTOCOL_ERROR.
    #[test]
    fn max_frame_size_must_stay_within_bounds() {
        assert_eq!(applied(&entry(0x5, 16_384)).unwrap().max_frame_size, 16_384);
        assert_eq!(
            applied(&entry(0x5, 16_777_215)).unwrap().max_frame_size,
            16_777_215
        );

        for value in [0, 16_383, 16_777_216, u32::MAX] {
            assert_eq!(
                applied(&entry(0x5, value)),
                Err(SettingsError::MaxFrameSize),
                "for {value}"
            );
        }
    }
}
