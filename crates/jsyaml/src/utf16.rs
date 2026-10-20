//! Byte offset → UTF-16 offset conversion. js-yaml's events and marks are UTF-16 offsets
//! (`String.prototype.slice` positions), while this crate works in bytes.

/// Converts byte offsets to UTF-16 offsets. Cheap when successive queries are close
/// together, as they are when walking an event stream; free for ASCII input.
#[derive(Debug, Clone)]
pub struct Utf16Cursor<'a> {
    input: &'a str,
    ascii: bool,
    byte: usize,
    unit: usize,
}

impl<'a> Utf16Cursor<'a> {
    /// A cursor at the start of `input`.
    pub fn new(input: &'a str) -> Self {
        Utf16Cursor {
            input,
            ascii: input.is_ascii(),
            byte: 0,
            unit: 0,
        }
    }

    /// The UTF-16 offset of byte offset `byte` (clamped to the input). Inside a char, the
    /// result counts that char as if it ended there.
    pub fn to_utf16(&mut self, byte: usize) -> usize {
        if self.ascii {
            return byte.min(self.input.len());
        }
        let bytes = self.input.as_bytes();
        let byte = byte.min(bytes.len());
        if byte >= self.byte {
            self.unit += utf16_len(&bytes[self.byte..byte]);
        } else {
            self.unit -= utf16_len(&bytes[byte..self.byte]);
        }
        self.byte = byte;
        self.unit
    }
}

/// UTF-16 length of UTF-8 `bytes`: chars count once, 4-byte chars (surrogate pairs) twice.
pub(crate) fn utf16_len(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .map(|&b| usize::from(b & 0xC0 != 0x80) + usize::from(b >= 0xF0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_encode_utf16_in_any_order() {
        let input = "aé€😀b\nz";
        let mut cursor = Utf16Cursor::new(input);
        let mut offsets: Vec<usize> = input.char_indices().map(|(i, _)| i).collect();
        offsets.push(input.len());
        offsets.reverse();
        offsets.rotate_left(3);
        for byte in offsets {
            assert_eq!(
                cursor.to_utf16(byte),
                input[..byte].encode_utf16().count(),
                "{byte}"
            );
        }
    }
}
