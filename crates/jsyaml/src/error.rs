//! `YAMLException` without the snippet: the JS bindings rebuild the full exception with
//! upstream's own `YAMLException.throwAt`, so messages and snippets match byte for byte.

use std::fmt;

/// A load error: upstream's `reason` plus the byte offset it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    reason: String,
    position: Option<usize>,
    unsupported: bool,
}

impl Error {
    /// An error at byte offset `position` of the input (upstream `throwAt`).
    pub fn new(reason: impl Into<String>, position: usize) -> Self {
        Error {
            reason: reason.into(),
            position: Some(position),
            unsupported: false,
        }
    }

    /// An error without a source mark (upstream `new YAMLException(reason)`).
    pub fn without_position(reason: impl Into<String>) -> Self {
        Error {
            reason: reason.into(),
            position: None,
            unsupported: false,
        }
    }

    /// The input is valid but this port can't reproduce js-yaml's result exactly (see
    /// [`Error::is_unsupported`]).
    pub(crate) fn unsupported(why: &str) -> Self {
        Error {
            reason: format!("jsyaml: unsupported: {why}"),
            position: None,
            unsupported: true,
        }
    }

    /// The load can't be reproduced exactly in Rust (lone surrogates, time-zone dependent
    /// key strings). Not a YAML error: the JS bindings rerun the load with js-yaml's own
    /// constructor; Rust callers get this error.
    pub fn is_unsupported(&self) -> bool {
        self.unsupported
    }

    /// The message without location, e.g. `duplicated mapping key`.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Byte offset into the input, if the error has a mark.
    pub fn position(&self) -> Option<usize> {
        self.position
    }

    /// Zero-based `(line, column)` of the mark, counted like upstream `throwAt`: `\n`,
    /// `\r\n` and lone `\r` each end a line, and the column is in UTF-16 code units.
    pub fn line_column(&self, input: &str) -> Option<(usize, usize)> {
        let position = self.position?.min(input.len());
        let bytes = input.as_bytes();
        let (mut line, mut line_start, mut i) = (0, 0, 0);
        while i < position {
            match bytes[i] {
                b'\n' => {
                    line += 1;
                    line_start = i + 1;
                }
                b'\r' => {
                    line += 1;
                    if bytes.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    line_start = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
        let line_start = line_start.min(position);
        let column = crate::utf16::utf16_len(&bytes[line_start..position]);
        Some((line, column))
    }

    /// Upstream's compact message: `reason in "file" (line:column)`, 1-based.
    pub fn format(&self, input: &str, filename: &str) -> String {
        let Some((line, column)) = self.line_column(input) else {
            return self.reason.clone();
        };
        let name = if filename.is_empty() {
            String::new()
        } else {
            format!("in \"{filename}\" ")
        };
        format!("{} {name}({}:{})", self.reason, line + 1, column + 1)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.position {
            Some(position) => write!(f, "{} (at byte {position})", self.reason),
            None => f.write_str(&self.reason),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_column_counts_all_line_breaks() {
        let input = "a\nb\r\nc\rdé€x";
        let at = |s: &str| Error::new("e", input.find(s).unwrap()).line_column(input);
        assert_eq!(at("a"), Some((0, 0)));
        assert_eq!(at("b"), Some((1, 0)));
        assert_eq!(at("c"), Some((2, 0)));
        assert_eq!(at("x"), Some((3, 3)));
    }

    #[test]
    fn format_matches_upstream_compact_form() {
        let e = Error::new("bad indentation", 4);
        assert_eq!(e.format("a:\n  b", ""), "bad indentation (2:2)");
        assert_eq!(
            e.format("a:\n  b", "x.yml"),
            "bad indentation in \"x.yml\" (2:2)"
        );
        assert_eq!(Error::without_position("empty").format("", "x"), "empty");
    }
}
