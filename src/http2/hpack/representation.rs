// RFC 7541 §6: the high bits of the first byte of a field select its
// representation, the low bits start its prefixed integer (index or size).
//
//     Indexed           1xxx xxxx   flags 0x80, 7-bit prefix
//     LiteralIndexing   01xx xxxx   flags 0x40, 6-bit prefix
//     SizeUpdate        001x xxxx   flags 0x20, 5-bit prefix
//     NeverIndexed      0001 xxxx   flags 0x10, 4-bit prefix
//     NoIndexing        0000 xxxx   flags 0x00, 4-bit prefix
//
// The decoder goes byte → representation (`from_byte`), the encoder
// representation → bits (`flags`, `prefix_bits`): one table for both directions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Representation {
    Indexed,
    LiteralIndexing,
    SizeUpdate,
    NeverIndexed,
    NoIndexing,
}

impl Representation {
    // The ranges cover every u8 value, so the compiler checks nothing is missing.
    pub(super) fn from_byte(byte: u8) -> Self {
        match byte {
            // 1xxx xxxx: 1000_0000..=1111_1111
            0b1000_0000..=0b1111_1111 => Self::Indexed,
            // 01xx xxxx: 0100_0000..=0111_1111
            0b0100_0000..=0b0111_1111 => Self::LiteralIndexing,
            // 001x xxxx: 0010_0000..=0011_1111
            0b0010_0000..=0b0011_1111 => Self::SizeUpdate,
            // 0001 xxxx: 0001_0000..=0001_1111
            0b0001_0000..=0b0001_1111 => Self::NeverIndexed,
            // 0000 xxxx: 0000_0000..=0000_1111
            0b0000_0000..=0b0000_1111 => Self::NoIndexing,
        }
    }

    // The bits above the prefix, as passed to `encode_integer`.
    pub(super) fn flags(self) -> u8 {
        match self {
            Self::Indexed => 0x80,
            Self::LiteralIndexing => 0x40,
            Self::SizeUpdate => 0x20,
            Self::NeverIndexed => 0x10,
            Self::NoIndexing => 0x00,
        }
    }

    pub(super) fn prefix_bits(self) -> u8 {
        match self {
            Self::Indexed => 7,
            Self::LiteralIndexing => 6,
            Self::SizeUpdate => 5,
            Self::NeverIndexed | Self::NoIndexing => 4,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const ALL: [Representation; 5] = [
        Representation::Indexed,
        Representation::LiteralIndexing,
        Representation::SizeUpdate,
        Representation::NeverIndexed,
        Representation::NoIndexing,
    ];

    // C.2.4 (0x82), C.2.1 (0x40), C.2.2 (0x04), C.2.3 (0x10), size update to 0 (0x20).
    #[test]
    fn reads_the_representation_of_rfc_examples() {
        assert_eq!(Representation::from_byte(0x82), Representation::Indexed);
        assert_eq!(
            Representation::from_byte(0x40),
            Representation::LiteralIndexing
        );
        assert_eq!(Representation::from_byte(0x04), Representation::NoIndexing);
        assert_eq!(
            Representation::from_byte(0x10),
            Representation::NeverIndexed
        );
        assert_eq!(Representation::from_byte(0x20), Representation::SizeUpdate);
    }

    // The flags of a representation must be read back as that representation,
    // whatever the prefix holds.
    #[test]
    fn flags_are_read_back_as_the_same_representation() {
        for representation in ALL {
            let mask = u8::MAX >> (8 - representation.prefix_bits());

            assert_eq!(
                Representation::from_byte(representation.flags()),
                representation
            );
            assert_eq!(
                Representation::from_byte(representation.flags() | mask),
                representation
            );
        }
    }

    // Flags and prefix never overlap: together they fill the byte exactly.
    #[test]
    fn flags_stay_above_the_prefix() {
        for representation in ALL {
            let mask = u8::MAX >> (8 - representation.prefix_bits());

            assert_eq!(representation.flags() & mask, 0, "{representation:?}");
        }
    }
}
