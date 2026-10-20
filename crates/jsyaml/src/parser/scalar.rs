//! Port of `src/parser/parser_scalar.ts`: decoding a scalar event's raw text.
//!
//! Upstream builds JS strings, which may hold lone surrogates (from `"\uD800"` escapes);
//! Rust strings can't, so escapes that don't form a valid code point decode to U+FFFD.
//! Everything else matches upstream exactly.

use std::borrow::Cow;

use crate::event::{Chomping, ScalarEvent, ScalarStyle};

// --- character helpers ---

fn simple_escape_sequence(c: u8) -> Option<char> {
    Some(match c {
        b'0' => '\x00',
        b'a' => '\x07',
        b'b' => '\x08',
        b't' | b'\t' => '\x09',
        b'n' => '\x0A',
        b'v' => '\x0B',
        b'f' => '\x0C',
        b'r' => '\x0D',
        b'e' => '\x1B',
        b' ' => ' ',
        b'"' => '"',
        b'/' => '/',
        b'\\' => '\\',
        b'N' => '\u{85}',
        b'_' => '\u{A0}',
        b'L' => '\u{2028}',
        b'P' => '\u{2029}',
        _ => return None,
    })
}

fn from_hex_code(c: u8) -> u32 {
    // Double-quoted scalar ranges are validated by the parser before cooking.
    match c {
        b'0'..=b'9' => u32::from(c - b'0'),
        _ => u32::from((c | 0x20).wrapping_sub(b'a')) + 10,
    }
}

fn escaped_hex_len(c: u8) -> usize {
    match c {
        b'x' => 2,
        b'u' => 4,
        _ => 8,
    }
}

fn at(input: &[u8], position: usize) -> u8 {
    input.get(position).copied().unwrap_or(0)
}

// --- line folding helpers ---

/// Skips a run of line breaks plus the leading whitespace of the following lines;
/// returns the new position and the number of line breaks consumed.
fn skip_folded_breaks(input: &[u8], mut position: usize, end: usize) -> (usize, usize) {
    let mut breaks = 0;
    while position < end {
        match input[position] {
            b'\n' => {
                breaks += 1;
                position += 1;
            }
            b'\r' => {
                breaks += 1;
                position += 1;
                if at(input, position) == b'\n' {
                    position += 1;
                }
            }
            b' ' | b'\t' => position += 1,
            _ => break,
        }
    }
    (position, breaks)
}

/// A single break becomes a space, several breaks become (count - 1) newlines.
fn push_folded_breaks(result: &mut String, count: usize) {
    if count == 1 {
        result.push(' ');
    } else {
        // Called only after skip_folded_breaks() consumed at least one line break.
        result.extend(std::iter::repeat_n('\n', count - 1));
    }
}

// --- per-style extractors ---

fn get_plain_value(text: &str, start: usize, end: usize) -> String {
    let input = text.as_bytes();
    let mut result = String::new();
    let mut position = start;
    let mut capture_start = start;
    let mut capture_end = start;

    while position < end {
        let ch = input[position];
        if ch == b'\n' || ch == b'\r' {
            result.push_str(&text[capture_start..capture_end]);
            let (next, breaks) = skip_folded_breaks(input, position, end);
            push_folded_breaks(&mut result, breaks);
            position = next;
            capture_start = next;
            capture_end = next;
        } else {
            position += 1;
            if ch != b' ' && ch != b'\t' {
                capture_end = position;
            }
        }
    }

    result.push_str(&text[capture_start..capture_end]);
    result
}

fn get_single_quoted_value(text: &str, start: usize, end: usize) -> String {
    let input = text.as_bytes();
    let mut result = String::new();
    let mut position = start;
    let mut capture_start = start;
    let mut capture_end = start;

    while position < end {
        let ch = input[position];
        if ch == b'\'' {
            // Within the stored range every quote is part of an escaped '' pair.
            result.push_str(&text[capture_start..position]);
            result.push('\'');
            position += 2;
            capture_start = position;
            capture_end = position;
        } else if ch == b'\n' || ch == b'\r' {
            result.push_str(&text[capture_start..capture_end]);
            let (next, breaks) = skip_folded_breaks(input, position, end);
            push_folded_breaks(&mut result, breaks);
            position = next;
            capture_start = next;
            capture_end = next;
        } else {
            position += 1;
            if ch != b' ' && ch != b'\t' {
                capture_end = position;
            }
        }
    }

    // Whitespace right before the closing quote is significant (it is only stripped
    // when followed by a line break).
    result.push_str(&text[capture_start..end]);
    result
}

/// Collects UTF-16 code units from `\x`, `\u` and `\U` escapes, so that two `\u`
/// escapes forming a surrogate pair decode to one character, as in JS.
#[derive(Default)]
struct Utf16Escapes {
    high: Option<u16>,
    /// A lone surrogate was replaced with U+FFFD.
    lossy: bool,
}

impl Utf16Escapes {
    fn push(&mut self, result: &mut String, code_point: u32) {
        match code_point {
            0xD800..=0xDBFF => {
                self.flush(result);
                self.high = Some(code_point as u16);
            }
            0xDC00..=0xDFFF => {
                if let Some(high) = self.high.take() {
                    let c = 0x10000 + ((u32::from(high) - 0xD800) << 10) + (code_point - 0xDC00);
                    result.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                } else {
                    self.lossy = true;
                    result.push('\u{FFFD}');
                }
            }
            _ => {
                self.flush(result);
                result.push(char::from_u32(code_point).unwrap_or_else(|| {
                    self.lossy = true;
                    '\u{FFFD}'
                }));
            }
        }
    }

    /// A high surrogate not followed by a low one.
    fn flush(&mut self, result: &mut String) {
        if self.high.take().is_some() {
            self.lossy = true;
            result.push('\u{FFFD}');
        }
    }
}

fn get_double_quoted_value(text: &str, start: usize, end: usize) -> (String, bool) {
    let input = text.as_bytes();
    let mut result = String::new();
    let mut pending = Utf16Escapes::default();
    let mut position = start;
    let mut capture_start = start;
    let mut capture_end = start;

    while position < end {
        let ch = input[position];
        if ch == b'\\' {
            // A pending high surrogate pairs only with an immediately following escape.
            if capture_start < position {
                pending.flush(&mut result);
            }
            result.push_str(&text[capture_start..position]);
            position += 1;
            let escaped = at(input, position);

            if escaped == b'\n' || escaped == b'\r' {
                // Escaped line break: a line continuation that joins with nothing.
                pending.flush(&mut result);
                position = skip_folded_breaks(input, position, end).0;
            } else if let Some(c) = simple_escape_sequence(escaped) {
                pending.flush(&mut result);
                result.push(c);
                position += 1;
            } else {
                // The parser has already rejected unknown escapes and invalid hex digits.
                let mut code_point: u32 = 0;
                for _ in 0..escaped_hex_len(escaped) {
                    position += 1;
                    code_point = (code_point << 4).wrapping_add(from_hex_code(at(input, position)));
                }
                pending.push(&mut result, code_point);
                position += 1;
            }

            capture_start = position;
            capture_end = position;
        } else if ch == b'\n' || ch == b'\r' {
            pending.flush(&mut result);
            result.push_str(&text[capture_start..capture_end]);
            let (next, breaks) = skip_folded_breaks(input, position, end);
            push_folded_breaks(&mut result, breaks);
            position = next;
            capture_start = next;
            capture_end = next;
        } else {
            position += 1;
            if ch != b' ' && ch != b'\t' {
                capture_end = position;
            }
        }
    }

    pending.flush(&mut result);
    result.push_str(&text[capture_start..end]);
    (result, pending.lossy)
}

fn get_block_value(
    text: &str,
    start: usize,
    end: usize,
    indent: i32,
    chomping: Chomping,
    folded: bool,
) -> String {
    let text_indent = usize::try_from(indent).unwrap_or(0);
    // The range starts at column 0 of the first line and includes every line break,
    // including those of trailing blank lines.
    let region = normalize_line_breaks(&text[start..end]);
    // An empty range is a block with no lines at all (e.g. an empty `|+`) and must stay
    // empty. A trailing line break only terminates the last line, so drop it before
    // splitting. Interior blank lines are kept.
    let body = region.strip_suffix('\n').unwrap_or(&region);
    let lines: Vec<&str> = if region.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    };

    let mut result = String::new();
    let mut did_read_content = false;
    let mut empty_lines = 0;
    let mut at_more_indented = false;
    let newlines = |result: &mut String, n: usize| result.extend(std::iter::repeat_n('\n', n));

    for line in lines {
        // Whitespace beyond the content indentation is content, so the indentation scan
        // stops at text_indent. indent < 0 means no content line was detected (a wholly
        // blank block), so every line is an empty line.
        let bytes = line.as_bytes();
        let mut column = 0;
        while column < text_indent && bytes.get(column) == Some(&b' ') {
            column += 1;
        }

        if indent < 0 || column >= bytes.len() {
            empty_lines += 1;
            continue;
        }

        let content = line.get(text_indent..).unwrap_or("");
        let first = content.as_bytes().first().copied().unwrap_or(0);

        if folded {
            if first == b' ' || first == b'\t' {
                // More-indented lines are not folded.
                at_more_indented = true;
                newlines(
                    &mut result,
                    if did_read_content {
                        1 + empty_lines
                    } else {
                        empty_lines
                    },
                );
            } else if at_more_indented {
                at_more_indented = false;
                newlines(&mut result, empty_lines + 1);
            } else if empty_lines == 0 {
                if did_read_content {
                    result.push(' ');
                }
            } else {
                newlines(&mut result, empty_lines);
            }
        } else {
            newlines(
                &mut result,
                if did_read_content {
                    1 + empty_lines
                } else {
                    empty_lines
                },
            );
        }

        result.push_str(content);
        did_read_content = true;
        empty_lines = 0;
    }

    match chomping {
        Chomping::Keep => {
            newlines(
                &mut result,
                if did_read_content {
                    1 + empty_lines
                } else {
                    empty_lines
                },
            );
        }
        Chomping::Clip if did_read_content => result.push('\n'),
        Chomping::Clip | Chomping::Strip => {}
    }
    result
}

/// `\r\n` and lone `\r` → `\n`.
fn normalize_line_breaks(s: &str) -> Cow<'_, str> {
    if !s.contains('\r') {
        return Cow::Borrowed(s);
    }
    Cow::Owned(s.replace("\r\n", "\n").replace('\r', "\n"))
}

/// Port of `getScalarValue`: the decoded text of `scalar`.
pub fn get_scalar_value<'a>(input: &'a str, scalar: &ScalarEvent) -> Cow<'a, str> {
    decode(input, scalar).0
}

/// [`get_scalar_value`], or `None` when the JS string would hold a lone surrogate (which
/// Rust strings can't represent).
pub(crate) fn scalar_value_exact<'a>(input: &'a str, scalar: &ScalarEvent) -> Option<Cow<'a, str>> {
    match decode(input, scalar) {
        (value, false) => Some(value),
        (_, true) => None,
    }
}

fn decode<'a>(input: &'a str, scalar: &ScalarEvent) -> (Cow<'a, str>, bool) {
    let Some(span) = scalar.value else {
        return (Cow::Borrowed(""), false);
    };
    // The parser marks verbatim slices (single-line, no escapes or folded breaks).
    if scalar.fast {
        return (Cow::Borrowed(span.slice(input)), false);
    }
    let (start, end) = (span.start, span.end);
    let value = match scalar.style {
        ScalarStyle::SingleQuoted => get_single_quoted_value(input, start, end),
        ScalarStyle::DoubleQuoted => {
            let (value, lossy) = get_double_quoted_value(input, start, end);
            return (Cow::Owned(value), lossy);
        }
        ScalarStyle::LiteralBlock => {
            get_block_value(input, start, end, scalar.indent, scalar.chomping, false)
        }
        ScalarStyle::FoldedBlock => {
            get_block_value(input, start, end, scalar.indent, scalar.chomping, true)
        }
        ScalarStyle::Plain => get_plain_value(input, start, end),
    };
    (Cow::Owned(value), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Span;

    fn scalar(input: &str, style: ScalarStyle) -> String {
        let event = ScalarEvent {
            value: Some(Span {
                start: 0,
                end: input.len(),
            }),
            anchor: None,
            tag: None,
            style,
            chomping: Chomping::Clip,
            indent: -1,
            fast: false,
        };
        get_scalar_value(input, &event).into_owned()
    }

    #[test]
    fn double_quoted_escapes() {
        let decoded = scalar(r"é\U0001F600\x41\t\n😀", ScalarStyle::DoubleQuoted);
        assert_eq!(decoded, "é😀A\t\n😀");
        assert_eq!(scalar(r"a\uD800b", ScalarStyle::DoubleQuoted), "a\u{FFFD}b");
        assert_eq!(scalar(r"\uDC00", ScalarStyle::DoubleQuoted), "\u{FFFD}");
    }

    #[test]
    fn folding() {
        assert_eq!(scalar("a\n  b\n\n  c", ScalarStyle::Plain), "a b\nc");
        assert_eq!(scalar("it''s \n x ", ScalarStyle::SingleQuoted), "it's x ");
    }
}
