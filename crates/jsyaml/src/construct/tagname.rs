//! Port of `tagNameFull` and `tagPercentDecode` from `src/common/tagname.ts`.

use std::collections::HashMap;

/// `tagPercentDecode`: `%XX` UTF-8 escapes → text, without validating the encoding.
/// Works on UTF-16 units like upstream; unpaired surrogates become U+FFFD.
fn tag_percent_decode(source: &str) -> String {
    let hex = |s: &str| u32::from_str_radix(s, 16).ok();
    let mut units: Vec<u16> = Vec::with_capacity(source.len());
    let b = source.as_bytes();
    let mut index = 0;
    while index < b.len() {
        if b[index] != b'%' {
            let c = source[index..].chars().next().unwrap_or('\u{FFFD}');
            let mut buf = [0u16; 2];
            units.extend_from_slice(c.encode_utf16(&mut buf));
            index += c.len_utf8();
            continue;
        }
        let byte = |i: usize| source.get(i + 1..i + 3).and_then(hex);
        // The parser only lets through well-formed `%XX` escapes.
        let mut ch = byte(index).unwrap_or(0);
        index += 3;
        if ch >= 0xC0 {
            let mut remaining = if ch < 0xE0 {
                1
            } else if ch < 0xF0 {
                2
            } else {
                3
            };
            ch &= (1 << (6 - remaining)) - 1;
            while remaining > 0 {
                ch = (ch << 6) | (byte(index).unwrap_or(0) & 0x3F);
                index += 3;
                remaining -= 1;
            }
        }
        if ch < 0x10000 {
            units.push(ch as u16);
        } else {
            let c = ch - 0x10000;
            units.push((0xD800 | (c >> 10)) as u16);
            units.push((0xDC00 | (c & 0x3FF)) as u16);
        }
    }
    String::from_utf16_lossy(&units)
}

/// `tagNameFull`: expands a raw tag (`!!str`, `!e!foo`, `!<tag:x>`) to its full name.
pub(crate) fn tag_name_full(raw_tag: &str, tag_handlers: &HashMap<String, String>) -> String {
    if raw_tag.len() >= 3 && raw_tag.starts_with("!<") && raw_tag.ends_with('>') {
        return tag_percent_decode(&raw_tag[2..raw_tag.len() - 1]);
    }
    let handle = match raw_tag.get(1..).and_then(|rest| rest.find('!')) {
        Some(i) => &raw_tag[..i + 2],
        None => "!",
    };
    let prefix = match tag_handlers.get(handle) {
        Some(prefix) => prefix.as_str(),
        None => match handle {
            "!!" => "tag:yaml.org,2002:",
            _ => handle,
        },
    };
    let mut full = tag_percent_decode(prefix);
    full.push_str(&tag_percent_decode(
        raw_tag.get(handle.len()..).unwrap_or(""),
    ));
    full
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_handles() {
        let mut handlers = HashMap::new();
        handlers.insert("!e!".to_owned(), "tag:example.com,2000:".to_owned());
        assert_eq!(tag_name_full("!!str", &handlers), "tag:yaml.org,2002:str");
        assert_eq!(tag_name_full("!local", &handlers), "!local");
        assert_eq!(
            tag_name_full("!e!foo%21", &handlers),
            "tag:example.com,2000:foo!"
        );
        assert_eq!(tag_name_full("!<tag:x%C3%A9>", &handlers), "tag:xé");
    }
}
