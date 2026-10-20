//! Port of `src/parser/parser.ts`: YAML text → [`Event`]s.
//!
//! The functions keep upstream's names (in snake case) and order, so new upstream releases
//! can be diffed and applied. Differences from upstream are deliberate and marked:
//!
//! - Positions are byte offsets. Upstream reads UTF-16 code units; scanning bytes is
//!   equivalent because every delimiter YAML looks at is ASCII, and continuation bytes
//!   never equal an ASCII byte. Columns (`position - lineStart`), which upstream compares
//!   with indentation, are counted in UTF-16 units (`State::column`).
//! - Upstream appends a `\0` sentinel to the input; `State::at` returns `0` past the end.
//! - `throwError` becomes `return Err(...)` with the same message and position.
//! - Recursion is capped at [`HARD_DEPTH_LIMIT`] whatever `max_depth` says: a stack
//!   overflow would abort the Node process, where upstream throws a `RangeError`.
//! - `findBlockMappingColon` is unused upstream and not ported.
//! - Scalars whose escapes decode to lone surrogates get U+FFFD from
//!   [`get_scalar_value`]; the constructor reports them as unsupported instead.

mod scalar;

use std::collections::HashMap;

pub use self::scalar::get_scalar_value;
pub(crate) use self::scalar::scalar_value_exact;

use crate::error::Error;
use crate::event::{
    Chomping, CollectionEvent, CollectionStyle, Directive, DocumentEvent, Event, ScalarEvent,
    ScalarStyle, Span,
};

/// Nesting depth at which parsing fails even if `max_depth` allows more.
///
/// Measured with `cargo run --release --example stack_probe`: at most ~870 bytes of stack
/// per level in release builds (block mappings; ~6.8 KB in debug), so 1000 levels fit in
/// well under 1 MiB, inside Node's 4 MiB worker-thread stacks with room for V8's frames.
pub const HARD_DEPTH_LIMIT: usize = 1000;

/// `ParserOptions` (the filename lives in the bindings, which format messages).
#[derive(Debug, Clone)]
pub struct ParserOptions {
    /// Maximum nesting depth of collections; aliases don't count.
    pub max_depth: usize,
}

impl Default for ParserOptions {
    fn default() -> Self {
        ParserOptions { max_depth: 100 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Context {
    FlowIn,
    FlowOut,
    BlockIn,
    BlockOut,
}

/// `NodeProperties`
#[derive(Debug, Clone, Copy, Default)]
struct Props {
    anchor: Option<Span>,
    tag: Option<Span>,
}

impl Props {
    fn is_empty(self) -> bool {
        self.anchor.is_none() && self.tag.is_none()
    }
}

/// `ParserSnapshot`
#[derive(Debug, Clone, Copy)]
struct Snapshot {
    position: usize,
    line: usize,
    line_start: usize,
    line_indent: isize,
    first_tab_in_line: Option<usize>,
    events_length: usize,
}

type Result<T> = std::result::Result<T, Error>;

/// `ParserState`
struct State<'a> {
    text: &'a str,
    input: &'a [u8],
    length: usize,
    max_depth: usize,
    position: usize,
    line: usize,
    line_start: usize,
    line_indent: isize,
    first_tab_in_line: Option<usize>,
    depth: usize,
    directives: Vec<Directive>,
    tag_handlers: HashMap<String, String>,
    events: Vec<Event>,
}

const BOM: &[u8] = "\u{FEFF}".as_bytes();

impl State<'_> {
    /// `state.input.charCodeAt(position)`, with `0` for the sentinel and beyond.
    fn at(&self, position: usize) -> u8 {
        self.input.get(position).copied().unwrap_or(0)
    }

    fn ch(&self) -> u8 {
        self.at(self.position)
    }

    fn slice(&self, start: usize, end: usize) -> &str {
        self.text.get(start..end).unwrap_or("")
    }

    /// `position - lineStart` in UTF-16 units, as upstream computes columns.
    fn column(&self, line_start: usize, position: usize) -> isize {
        let bytes = &self.input[line_start.min(position)..position];
        let units = if bytes.is_ascii() {
            bytes.len()
        } else {
            crate::utf16::utf16_len(bytes)
        };
        isize::try_from(units).unwrap_or(isize::MAX)
    }

    fn bom_at(&self, position: usize) -> bool {
        self.input.get(position..position + BOM.len()) == Some(BOM)
    }

    fn error(&self, message: impl Into<String>) -> Error {
        Error::new(message, self.position)
    }

    fn fail<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(self.error(message))
    }

    fn add_document_event(&mut self, explicit_start: bool, explicit_end: bool) {
        let directives = std::mem::take(&mut self.directives);
        self.events.push(Event::Document(DocumentEvent {
            explicit_start,
            explicit_end,
            directives,
        }));
    }

    fn add_collection_event(
        &mut self,
        mapping: bool,
        start: usize,
        props: Props,
        style: CollectionStyle,
    ) {
        let event = CollectionEvent {
            start,
            anchor: props.anchor,
            tag: props.tag,
            style,
        };
        self.events.push(if mapping {
            Event::Mapping(event)
        } else {
            Event::Sequence(event)
        });
    }

    fn insert_flow_pair_mapping_event(&mut self, snapshot: Snapshot) {
        let event = CollectionEvent {
            start: snapshot.position,
            anchor: None,
            tag: None,
            style: CollectionStyle::Flow,
        };
        self.events
            .insert(snapshot.events_length, Event::Mapping(event));
    }

    #[allow(clippy::too_many_arguments)]
    fn add_scalar_event(
        &mut self,
        value: Option<Span>,
        props: Props,
        style: ScalarStyle,
        chomping: Chomping,
        indent: i32,
        fast: bool,
    ) {
        self.events.push(Event::Scalar(ScalarEvent {
            value,
            anchor: props.anchor,
            tag: props.tag,
            style,
            chomping,
            indent,
            fast,
        }));
    }

    fn add_empty_scalar_event(&mut self) {
        self.add_scalar_event(
            None,
            Props::default(),
            ScalarStyle::Plain,
            Chomping::Clip,
            -1,
            false,
        );
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            position: self.position,
            line: self.line,
            line_start: self.line_start,
            line_indent: self.line_indent,
            first_tab_in_line: self.first_tab_in_line,
            events_length: self.events.len(),
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.position = snapshot.position;
        self.line = snapshot.line;
        self.line_start = snapshot.line_start;
        self.line_indent = snapshot.line_indent;
        self.first_tab_in_line = snapshot.first_tab_in_line;
        self.events.truncate(snapshot.events_length);
    }

    fn is_mapping_at(&self, index: usize) -> bool {
        matches!(self.events.get(index), Some(Event::Mapping(_)))
    }
}

fn is_eol(c: u8) -> bool {
    c == b'\n' || c == b'\r'
}

fn is_white_space(c: u8) -> bool {
    c == b'\t' || c == b' '
}

fn is_ws_or_eol(c: u8) -> bool {
    is_white_space(c) || is_eol(c)
}

fn is_ws_or_eol_or_end(c: u8) -> bool {
    c == 0 || is_ws_or_eol(c)
}

fn is_flow_indicator(c: u8) -> bool {
    matches!(c, b',' | b'[' | b']' | b'{' | b'}')
}

fn from_decimal_code(c: u8) -> Option<u8> {
    c.is_ascii_digit().then(|| c - b'0')
}

fn escaped_hex_len(c: u8) -> usize {
    match c {
        b'x' => 2,
        b'u' => 4,
        b'U' => 8,
        _ => 0,
    }
}

fn is_simple_escape(c: u8) -> bool {
    matches!(
        c,
        b'0' | b'a'
            | b'b'
            | b't'
            | b'\t'
            | b'n'
            | b'v'
            | b'f'
            | b'r'
            | b'e'
            | b' '
            | b'"'
            | b'/'
            | b'\\'
            | b'N'
            | b'_'
            | b'L'
            | b'P'
    )
}

// --- the regular expressions of upstream, as predicates ---

/// YAML 1.2.2 [91] c-tag-handle: `!`, `!!` or `!word!`.
fn is_tag_handle(s: &str) -> bool {
    let b = s.as_bytes();
    match b {
        [b'!'] | [b'!', b'!'] => true,
        [b'!', middle @ .., b'!'] => {
            !middle.is_empty()
                && middle
                    .iter()
                    .all(|&c| c.is_ascii_alphanumeric() || c == b'-')
        }
        _ => false,
    }
}

/// Length of one `%XX` escape or one char of the class at `b[i..]`, if it matches.
fn uri_char_len(b: &[u8], i: usize, tag_char: bool) -> Option<usize> {
    let c = *b.get(i)?;
    if c == b'%' {
        let hex = |j: usize| b.get(j).is_some_and(u8::is_ascii_hexdigit);
        return (hex(i + 1) && hex(i + 2)).then_some(3);
    }
    // [39] ns-uri-char; [40] ns-tag-char additionally excludes `!`, `,`, `[`, `]`.
    let ok = c.is_ascii_alphanumeric()
        || matches!(
            c,
            b'-' | b'#'
                | b';'
                | b'/'
                | b'?'
                | b':'
                | b'@'
                | b'&'
                | b'='
                | b'+'
                | b'$'
                | b'_'
                | b'.'
                | b'~'
                | b'*'
                | b'\''
                | b'('
                | b')'
        )
        || (!tag_char && matches!(c, b',' | b'!' | b'[' | b']'));
    ok.then_some(1)
}

/// `^(?:class)*$` from byte `from`.
fn all_uri_chars(b: &[u8], mut from: usize, tag_char: bool) -> bool {
    while from < b.len() {
        match uri_char_len(b, from, tag_char) {
            Some(n) => from += n,
            None => return false,
        }
    }
    true
}

/// `PATTERN_TAG_URI`: (ns-uri-char)*
fn is_tag_uri(s: &str) -> bool {
    all_uri_chars(s.as_bytes(), 0, false)
}

/// `PATTERN_TAG_SUFFIX`: (ns-tag-char)+
fn is_tag_suffix(s: &str) -> bool {
    !s.is_empty() && all_uri_chars(s.as_bytes(), 0, true)
}

/// `PATTERN_TAG_PREFIX`: `!` (ns-uri-char)* | ns-tag-char (ns-uri-char)*
fn is_tag_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    if b.first() == Some(&b'!') {
        return all_uri_chars(b, 1, false);
    }
    match uri_char_len(b, 0, true) {
        Some(n) => all_uri_chars(b, n, false),
        None => false,
    }
}

/// `PATTERN_NON_PRINTABLE`: the byte offset of the first non-printable char of `b`.
/// (Its lone-surrogate alternatives can't occur in UTF-8.)
fn find_non_printable(b: &[u8]) -> Option<usize> {
    // Fast path: printable ASCII and tabs only (vectorises well).
    if b.iter().all(|&c| (0x20..0x7F).contains(&c) || c == b'\t') {
        return None;
    }
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let bad = match c {
            b'\t' | b'\n' | b'\r' => false,
            0x00..=0x1F | 0x7F => true,
            // U+0080..=U+009F except U+0085 (NEL)
            0xC2 => b
                .get(i + 1)
                .is_some_and(|&n| (0x80..=0x9F).contains(&n) && n != 0x85),
            // U+FFFE, U+FFFF
            0xEF => b.get(i + 1) == Some(&0xBF) && matches!(b.get(i + 2), Some(0xBE | 0xBF)),
            _ => false,
        };
        if bad {
            return Some(i);
        }
        i += 1;
    }
    None
}

// --- the parser ---

impl State<'_> {
    /// Precondition: the position is at LF or CR.
    fn consume_line_break(&mut self) {
        let ch = self.ch();
        self.position += 1;
        if ch == b'\r' && self.ch() == b'\n' {
            self.position += 1;
        }
        self.line += 1;
        self.line_start = self.position;
        self.line_indent = 0;
        self.first_tab_in_line = None;
    }

    fn skip_separation_space(&mut self, allow_comments: bool) -> usize {
        let mut line_breaks = 0;
        let mut ch = self.ch();
        let mut has_separation = self.position == self.line_start
            || (self.position > 0 && is_ws_or_eol(self.at(self.position - 1)));

        while ch != 0 {
            while is_white_space(ch) {
                has_separation = true;
                if ch == b'\t' && self.first_tab_in_line.is_none() {
                    self.first_tab_in_line = Some(self.position);
                }
                self.position += 1;
                ch = self.ch();
            }

            if allow_comments && has_separation && ch == b'#' {
                loop {
                    self.position += 1;
                    ch = self.ch();
                    if is_eol(ch) || ch == 0 {
                        break;
                    }
                }
            }

            if !is_eol(ch) {
                break;
            }

            self.consume_line_break();
            line_breaks += 1;
            has_separation = true;
            ch = self.ch();

            while ch == b' ' {
                self.line_indent += 1;
                self.position += 1;
                ch = self.ch();
            }
        }

        line_breaks
    }

    fn test_document_separator(&self, position: usize) -> bool {
        let ch = self.at(position);
        if (ch == b'-' || ch == b'.') && ch == self.at(position + 1) && ch == self.at(position + 2)
        {
            let following = self.at(position + 3);
            return following == 0 || is_ws_or_eol(following);
        }
        false
    }

    fn skip_byte_order_mark(&mut self) {
        if self.position == self.line_start && self.bom_at(self.position) {
            self.position += BOM.len();
            self.line_start = self.position;
        }
    }

    fn test_document_boundary(&mut self) -> bool {
        if self.position != self.line_start {
            return false;
        }

        // Fast path: most document boundaries do not have a BOM.
        if self.test_document_separator(self.position) {
            return true;
        }
        if !self.bom_at(self.position) {
            return false;
        }

        let snapshot = self.snapshot();
        self.skip_byte_order_mark();
        self.skip_separation_space(true);

        let ch = self.ch();
        let result = self.position == self.line_start
            && (ch == b'%' || (ch == b'-' && self.test_document_separator(self.position)));

        self.restore(snapshot);
        result
    }

    fn skip_until_line_end(&mut self) {
        let mut ch = self.ch();
        while ch != 0 && !is_eol(ch) {
            self.position += 1;
            ch = self.ch();
        }
    }

    fn check_printable(&mut self, start: usize, end: usize) -> Result<()> {
        if let Some(offset) = find_non_printable(&self.input[start..end]) {
            self.position = start + offset;
            return self.fail("the stream contains non-printable characters");
        }
        Ok(())
    }

    fn read_tag_property(&mut self, props: &mut Props, in_flow: bool) -> Result<bool> {
        if self.ch() != b'!' {
            return Ok(false);
        }
        if props.tag.is_some() {
            return self.fail("duplication of a tag property");
        }

        let start = self.position;
        let mut is_verbatim = false;
        let mut is_named = false;
        let mut tag_handle = (start, start + 1); // "!"
        self.position += 1;
        let mut ch = self.ch();

        if ch == b'<' {
            is_verbatim = true;
            self.position += 1;
            ch = self.ch();
        } else if ch == b'!' {
            is_named = true;
            tag_handle = (start, start + 2); // "!!"
            self.position += 1;
            ch = self.ch();
        }

        let mut suffix_start = self.position;
        let tag_name;

        if is_verbatim {
            while ch != 0 && ch != b'>' {
                self.position += 1;
                ch = self.ch();
            }
            if ch != b'>' {
                return self.fail("unexpected end of the stream within a verbatim tag");
            }
            tag_name = (suffix_start, self.position);
            self.position += 1;
        } else {
            while ch != 0 && !is_ws_or_eol(ch) && !(in_flow && is_flow_indicator(ch)) {
                if ch == b'!' {
                    if is_named {
                        return self.fail("tag suffix cannot contain exclamation marks");
                    }
                    tag_handle = (suffix_start - 1, self.position + 1);
                    if !is_tag_handle(self.slice(tag_handle.0, tag_handle.1)) {
                        return self.fail("named tag handle cannot contain such characters");
                    }
                    is_named = true;
                    suffix_start = self.position + 1;
                }
                self.position += 1;
                ch = self.ch();
            }

            tag_name = (suffix_start, self.position);
            if self.input[tag_name.0..tag_name.1]
                .iter()
                .any(|&c| is_flow_indicator(c))
            {
                return self.fail("tag suffix cannot contain flow indicator characters");
            }
        }

        let name = self.slice(tag_name.0, tag_name.1);
        if !name.is_empty()
            && !(if is_verbatim {
                is_tag_uri(name)
            } else {
                is_tag_suffix(name)
            })
        {
            let message = format!("tag name cannot contain such characters: {name}");
            return self.fail(message);
        }

        let handle = self.slice(tag_handle.0, tag_handle.1);
        if !is_verbatim
            && handle != "!"
            && handle != "!!"
            && !self.tag_handlers.contains_key(handle)
        {
            let message = format!("undeclared tag handle \"{handle}\"");
            return self.fail(message);
        }

        props.tag = Some(Span {
            start,
            end: self.position,
        });
        Ok(true)
    }

    /// The name after `&` or `*`: up to whitespace, a line break, a flow indicator or the end.
    fn read_node_name(&mut self) -> Span {
        self.position += 1;
        let start = self.position;
        loop {
            let ch = self.ch();
            if ch == 0 || is_ws_or_eol(ch) || is_flow_indicator(ch) {
                break;
            }
            self.position += 1;
        }
        Span {
            start,
            end: self.position,
        }
    }

    fn read_anchor_property(&mut self, props: &mut Props) -> Result<bool> {
        if self.ch() != b'&' {
            return Ok(false);
        }
        if props.anchor.is_some() {
            return self.fail("duplication of an anchor property");
        }

        let span = self.read_node_name();
        if span.start == span.end {
            return self.fail("name of an anchor node must contain at least one character");
        }

        props.anchor = Some(span);
        Ok(true)
    }

    fn read_alias(&mut self, props: Props) -> Result<bool> {
        if self.ch() != b'*' {
            return Ok(false);
        }
        if !props.is_empty() {
            return self.fail("alias node should not have any properties");
        }

        let span = self.read_node_name();
        if span.start == span.end {
            return self.fail("name of an alias node must contain at least one character");
        }

        self.events.push(Event::Alias(span));
        Ok(true)
    }

    fn read_flow_scalar_break(&mut self, node_indent: isize) -> Result<()> {
        self.skip_separation_space(false);
        if self.line_indent < node_indent {
            return self.fail("deficient indentation");
        }
        Ok(())
    }

    fn read_single_quoted_scalar(&mut self, node_indent: isize, props: Props) -> Result<bool> {
        if self.ch() != b'\'' {
            return Ok(false);
        }

        self.position += 1;
        let start = self.position;
        // Sliceable verbatim when there are no '' pairs and no folded line breaks.
        let mut simple = true;

        while self.ch() != 0 {
            let ch = self.ch();

            if ch == b'\'' {
                if self.at(self.position + 1) == b'\'' {
                    simple = false;
                    self.position += 2;
                    continue;
                }

                let end = self.position;
                self.position += 1;
                let value = Some(Span { start, end });
                self.add_scalar_event(
                    value,
                    props,
                    ScalarStyle::SingleQuoted,
                    Chomping::Clip,
                    -1,
                    simple,
                );
                return Ok(true);
            }

            if is_eol(ch) {
                simple = false;
                self.read_flow_scalar_break(node_indent)?;
            } else if self.position == self.line_start
                && self.test_document_separator(self.position)
            {
                return self.fail("unexpected end of the document within a single quoted scalar");
            } else if ch != b'\t' && ch < 0x20 {
                return self.fail("expected valid JSON character");
            } else {
                self.position += 1;
            }
        }

        self.fail("unexpected end of the stream within a single quoted scalar")
    }

    fn read_double_quoted_scalar(&mut self, node_indent: isize, props: Props) -> Result<bool> {
        if self.ch() != b'"' {
            return Ok(false);
        }

        self.position += 1;
        let start = self.position;
        // Sliceable verbatim when there are no \ escapes and no folded line breaks.
        let mut simple = true;

        while self.ch() != 0 {
            let ch = self.ch();

            if ch == b'"' {
                let end = self.position;
                self.position += 1;
                let value = Some(Span { start, end });
                self.add_scalar_event(
                    value,
                    props,
                    ScalarStyle::DoubleQuoted,
                    Chomping::Clip,
                    -1,
                    simple,
                );
                return Ok(true);
            }

            if ch == b'\\' {
                simple = false;
                self.position += 1;
                let escaped = self.ch();

                if is_eol(escaped) {
                    self.read_flow_scalar_break(node_indent)?;
                } else if is_simple_escape(escaped) {
                    self.position += 1;
                } else {
                    let hex_length = escaped_hex_len(escaped);
                    if hex_length == 0 {
                        return self.fail("unknown escape sequence");
                    }
                    for _ in 0..hex_length {
                        self.position += 1;
                        if !self.ch().is_ascii_hexdigit() {
                            return self.fail("expected hexadecimal character");
                        }
                    }
                    self.position += 1;
                }
            } else if is_eol(ch) {
                simple = false;
                self.read_flow_scalar_break(node_indent)?;
            } else if self.position == self.line_start
                && self.test_document_separator(self.position)
            {
                return self.fail("unexpected end of the document within a double quoted scalar");
            } else if ch != b'\t' && ch < 0x20 {
                return self.fail("expected valid JSON character");
            } else {
                self.position += 1;
            }
        }

        self.fail("unexpected end of the stream within a double quoted scalar")
    }

    fn read_block_scalar(&mut self, parent_indent: isize, props: Props) -> Result<bool> {
        let ch = self.ch();
        let mut chomping = Chomping::Clip;
        let mut indent: isize = -1;
        let mut detected_indent = false;

        if ch != b'|' && ch != b'>' {
            return Ok(false);
        }

        let style = if ch == b'|' {
            ScalarStyle::LiteralBlock
        } else {
            ScalarStyle::FoldedBlock
        };
        self.position += 1;

        while self.ch() != 0 {
            let current = self.ch();

            if current == b'+' || current == b'-' {
                if chomping != Chomping::Clip {
                    return self.fail("repeat of a chomping mode identifier");
                }
                chomping = if current == b'+' {
                    Chomping::Keep
                } else {
                    Chomping::Strip
                };
                self.position += 1;
            } else if let Some(digit) = from_decimal_code(current) {
                if digit == 0 {
                    return self.fail(
                        "bad explicit indentation width of a block scalar; it cannot be less than one",
                    );
                }
                if detected_indent {
                    return self.fail("repeat of an indentation width identifier");
                }
                indent = parent_indent + isize::from(digit) - 1;
                detected_indent = true;
                self.position += 1;
            } else {
                break;
            }
        }

        let mut had_whitespace = false;
        while is_white_space(self.ch()) {
            had_whitespace = true;
            self.position += 1;
        }
        if had_whitespace && self.ch() == b'#' {
            self.skip_until_line_end();
        }

        if is_eol(self.ch()) {
            self.consume_line_break();
        } else if self.ch() != 0 {
            return self.fail("a line break is expected");
        }

        let mut content_indent: isize = if detected_indent { indent } else { -1 };
        let mut max_leading_indent: isize = 0;
        let value_start = self.position;
        let mut value_end = self.position;

        while self.ch() != 0 {
            let line_position = self.position;
            let mut spaces = 0;
            while self.at(line_position + spaces) == b' ' {
                spaces += 1;
            }
            let column = isize::try_from(spaces).unwrap_or(isize::MAX);

            let first = self.at(line_position + spaces);
            if first == 0 {
                // End of input acts as a line terminator without a line break to include.
                // A final all-spaces line still counts: beyond a content indent the spaces
                // are content; in a wholly blank block they form a blank line that chomping
                // must see. Otherwise the block ends at the start of this empty line.
                if content_indent >= 0 {
                    if column > content_indent {
                        value_end = line_position + spaces;
                    }
                } else if column > 0 {
                    value_end = line_position + spaces;
                }
                break;
            }
            if self.test_document_boundary() {
                break;
            }

            if !detected_indent && content_indent == -1 && is_eol(first) {
                max_leading_indent = max_leading_indent.max(column);
            }

            if !detected_indent && content_indent == -1 && !is_eol(first) {
                if first == b'\t' && column < parent_indent {
                    self.position = line_position + spaces;
                    return self.fail("tab characters must not be used in indentation");
                }
                if column >= parent_indent && column < max_leading_indent {
                    self.position = line_position + spaces;
                    return self.fail("bad indentation of a mapping entry");
                }
            }

            if content_indent == -1 && !is_eol(first) && column < parent_indent {
                self.line_indent = column;
                self.position = line_position + spaces;
                break;
            }

            if !detected_indent && !is_eol(first) && content_indent == -1 {
                content_indent = column;
            }

            let required_indent = if content_indent == -1 {
                parent_indent + 1
            } else {
                content_indent
            };
            if !is_eol(first) && column < required_indent {
                self.line_indent = column;
                self.position = line_position + spaces;
                break;
            }

            self.skip_until_line_end();
            value_end = self.position;
            if is_eol(self.ch()) {
                self.consume_line_break();
                // Include the line break so trailing blank lines are preserved; this tells
                // an empty `|+` (range "") from one with a blank line (range "\n").
                // De-indentation and chomping happen in get_scalar_value.
                value_end = self.position;
            }
        }

        self.check_printable(value_start, value_end)?;
        let value = Some(Span {
            start: value_start,
            end: value_end,
        });
        let indent = i32::try_from(content_indent).unwrap_or(i32::MAX);
        self.add_scalar_event(value, props, style, chomping, indent, false);
        Ok(true)
    }

    fn can_start_plain_scalar(&self, context: Context) -> bool {
        let ch = self.ch();
        let in_flow = context == Context::FlowIn;

        if ch == 0
            || is_ws_or_eol(ch)
            || matches!(
                ch,
                b'#' | b'&' | b'*' | b'!' | b'|' | b'>' | b'\'' | b'"' | b'%' | b'@' | b'`'
            )
            || is_flow_indicator(ch)
        {
            return false;
        }

        if ch == b'?' || ch == b'-' {
            let following = self.at(self.position + 1);
            if is_ws_or_eol_or_end(following) || (in_flow && is_flow_indicator(following)) {
                return false;
            }
        }

        true
    }

    fn read_plain_scalar(
        &mut self,
        node_indent: isize,
        context: Context,
        props: Props,
    ) -> Result<bool> {
        if !self.can_start_plain_scalar(context) {
            return Ok(false);
        }

        let start = self.position;
        let mut end = self.position;
        let mut ch = self.ch();
        let in_flow = context == Context::FlowIn;
        // A single-line plain scalar is sliceable verbatim: the range already excludes
        // trailing whitespace. Folded line breaks make it non-simple.
        let mut multiline = false;

        while ch != 0 {
            if self.test_document_boundary() {
                break;
            }

            if ch == b':' {
                let following = self.at(self.position + 1);
                if is_ws_or_eol_or_end(following) || (in_flow && is_flow_indicator(following)) {
                    break;
                }
            } else if ch == b'#' {
                if is_ws_or_eol(self.at(self.position - 1)) {
                    break;
                }
            } else if in_flow && is_flow_indicator(ch) {
                break;
            } else if is_eol(ch) {
                let saved = (self.position, self.line, self.line_start, self.line_indent);

                self.skip_separation_space(false);

                if self.line_indent >= node_indent {
                    multiline = true;
                    ch = self.ch();
                    continue;
                }

                (self.position, self.line, self.line_start, self.line_indent) = saved;
                break;
            }

            if !is_white_space(ch) {
                end = self.position + 1;
            }
            self.position += 1;
            ch = self.ch();
        }

        if end == start {
            return Ok(false);
        }

        self.check_printable(start, end)?;
        let value = Some(Span { start, end });
        self.add_scalar_event(
            value,
            props,
            ScalarStyle::Plain,
            Chomping::Clip,
            -1,
            !multiline,
        );
        Ok(true)
    }

    fn skip_flow_separation_space(&mut self, node_indent: isize) -> Result<()> {
        let start_line = self.line;
        self.skip_separation_space(true);

        if self.line_indent < node_indent
            && (self.line > start_line || self.first_tab_in_line.is_some())
        {
            return self.fail("deficient indentation");
        }
        Ok(())
    }

    fn read_flow_collection(&mut self, node_indent: isize, props: Props) -> Result<bool> {
        let ch = self.ch();
        let is_mapping = ch == b'{';
        let start = self.position;
        let mut read_next = true;

        if ch != b'[' && ch != b'{' {
            return Ok(false);
        }

        let terminator = if is_mapping { b'}' } else { b']' };
        self.add_collection_event(is_mapping, start, props, CollectionStyle::Flow);
        self.position += 1;

        while self.ch() != 0 {
            self.skip_flow_separation_space(node_indent)?;

            let mut ch = self.ch();

            if ch == terminator {
                self.position += 1;
                self.events.push(Event::Pop);
                return Ok(true);
            } else if !read_next {
                return self.fail("missed comma between flow collection entries");
            } else if ch == b',' {
                return self.fail("expected the node content, but found ','");
            }

            let mut is_pair = false;
            let mut is_explicit_pair = false;

            if ch == b'?' && is_ws_or_eol(self.at(self.position + 1)) {
                is_pair = true;
                is_explicit_pair = true;
                self.position += 1;
                self.skip_flow_separation_space(node_indent)?;
            }

            let entry_line = self.line;
            let entry_start = self.snapshot();

            let key_was_read = self.parse_node(node_indent, Context::FlowIn, false, true, true)?;
            self.skip_flow_separation_space(node_indent)?;

            ch = self.ch();

            if (is_mapping || is_explicit_pair || self.line == entry_line) && ch == b':' {
                self.position += 1;
                self.skip_flow_separation_space(node_indent)?;
                if !is_mapping {
                    self.insert_flow_pair_mapping_event(entry_start);
                }
                if !key_was_read {
                    self.add_empty_scalar_event();
                }
                if !self.parse_node(node_indent, Context::FlowIn, false, true, true)? {
                    self.add_empty_scalar_event();
                }
                self.skip_flow_separation_space(node_indent)?;
                if !is_mapping {
                    self.events.push(Event::Pop);
                }
            } else if is_mapping && is_pair {
                if !key_was_read {
                    self.add_empty_scalar_event();
                }
                self.add_empty_scalar_event();
            } else if is_mapping {
                self.add_empty_scalar_event();
            } else if is_pair {
                self.insert_flow_pair_mapping_event(entry_start);
                if !key_was_read {
                    self.add_empty_scalar_event();
                }
                self.add_empty_scalar_event();
                self.events.push(Event::Pop);
            }

            ch = self.ch();

            if ch == b',' {
                read_next = true;
                self.position += 1;
            } else {
                read_next = false;
            }
        }

        self.fail("unexpected end of the stream within a flow collection")
    }

    fn at_block_sequence_entry(&self) -> bool {
        self.ch() == b'-' && is_ws_or_eol_or_end(self.at(self.position + 1))
    }

    fn read_block_sequence(&mut self, node_indent: isize, props: Props) -> Result<bool> {
        if self.first_tab_in_line.is_some() || !self.at_block_sequence_entry() {
            return Ok(false);
        }

        self.add_collection_event(false, self.position, props, CollectionStyle::Block);

        while self.at_block_sequence_entry() {
            if let Some(tab) = self.first_tab_in_line {
                self.position = tab;
                return self.fail("tab characters must not be used in indentation");
            }

            let entry_line = self.line;
            self.position += 1;

            let had_break = self.skip_separation_space(true) > 0;
            if self.first_tab_in_line.is_some() && self.at_block_sequence_entry() {
                return self.fail("bad indentation of a sequence entry");
            }

            if had_break && self.line_indent <= node_indent {
                self.add_empty_scalar_event();
            } else {
                self.parse_node(node_indent, Context::BlockIn, false, true, true)?;
            }

            self.skip_separation_space(true);

            if self.line_indent < node_indent || self.position >= self.length {
                break;
            }
            if self.line_indent > node_indent {
                return self.fail("bad indentation of a sequence entry");
            }
            if self.line == entry_line && self.at_block_sequence_entry() {
                return self.fail("bad indentation of a sequence entry");
            }
        }

        self.events.push(Event::Pop);
        Ok(true)
    }

    // Upstream assigns `pendingExplicitKey` in a few places where it is never read again;
    // they are kept so the code lines up with upstream.
    #[allow(unused_assignments)]
    fn read_block_mapping(
        &mut self,
        node_indent: isize,
        flow_indent: isize,
        props: Props,
    ) -> Result<bool> {
        let mut at_explicit_key = false;
        let mut detected = false;
        let mut mapping_opened = false;
        let mut pending_explicit_key = false;

        if self.first_tab_in_line.is_some() {
            return Ok(false);
        }

        let mut ch = self.ch();

        while ch != 0 {
            if !at_explicit_key {
                if let Some(tab) = self.first_tab_in_line {
                    self.position = tab;
                    return self.fail("tab characters must not be used in indentation");
                }
            }

            let following = self.at(self.position + 1);
            let entry_line = self.line;

            if (ch == b'?' || ch == b':') && is_ws_or_eol_or_end(following) {
                if !mapping_opened {
                    self.add_collection_event(true, self.position, props, CollectionStyle::Block);
                    mapping_opened = true;
                }

                if ch == b'?' {
                    if at_explicit_key {
                        self.add_empty_scalar_event();
                    }
                    detected = true;
                    at_explicit_key = true;
                } else if at_explicit_key {
                    at_explicit_key = false;
                } else {
                    self.add_empty_scalar_event();
                    detected = true;
                    at_explicit_key = false;
                }

                self.position += 1;
                pending_explicit_key = true;
            } else {
                // An explicit key awaiting its value, followed by an implicit key, means the
                // explicit key's value is empty. Emit it now so it precedes the implicit key.
                if at_explicit_key {
                    self.add_empty_scalar_event();
                    at_explicit_key = false;
                }

                let before_key = self.snapshot();

                if !self.parse_node(flow_indent, Context::FlowOut, false, true, true)? {
                    break;
                }

                if self.line == entry_line {
                    ch = self.ch();
                    while is_white_space(ch) {
                        self.position += 1;
                        ch = self.ch();
                    }

                    if ch == b':' {
                        self.position += 1;
                        ch = self.ch();

                        if !is_ws_or_eol_or_end(ch) {
                            return self.fail(
                                "a whitespace character is expected after the key-value separator within a block mapping",
                            );
                        }

                        if !mapping_opened {
                            // The key is already parsed: insert the MAPPING event before its
                            // events instead of rolling back and re-reading the key.
                            let event = CollectionEvent {
                                start: before_key.position,
                                anchor: props.anchor,
                                tag: props.tag,
                                style: CollectionStyle::Block,
                            };
                            self.events
                                .insert(before_key.events_length, Event::Mapping(event));
                            mapping_opened = true;
                        }

                        detected = true;
                        at_explicit_key = false;
                        pending_explicit_key = false;
                    } else if detected {
                        return self.fail("expected ':' after a mapping key");
                    } else {
                        // Not a mapping. If outer properties are pending, roll back so the
                        // caller re-reads this node with them attached.
                        if !props.is_empty() {
                            self.restore(before_key);
                            return Ok(false);
                        }
                        return Ok(true);
                    }
                } else if detected {
                    return self.fail(
                        "can not read a block mapping entry; a multiline key may not be an implicit key",
                    );
                } else {
                    if !props.is_empty() {
                        self.restore(before_key);
                        return Ok(false);
                    }
                    return Ok(true);
                }
            }

            if self.parse_node(
                node_indent,
                Context::BlockOut,
                true,
                pending_explicit_key,
                true,
            )? {
                pending_explicit_key = false;
            }

            if !at_explicit_key && pending_explicit_key {
                self.add_empty_scalar_event();
                pending_explicit_key = false;
            }

            self.skip_separation_space(true);
            ch = self.ch();

            if (self.line == entry_line || self.line_indent > node_indent) && ch != 0 {
                return self.fail("bad indentation of a mapping entry");
            } else if self.line_indent < node_indent {
                break;
            }
        }

        if !detected {
            return Ok(false);
        }
        if at_explicit_key {
            self.add_empty_scalar_event();
        }
        if mapping_opened {
            self.events.push(Event::Pop);
        }
        Ok(true)
    }

    fn indent_status(&self, parent_indent: isize) -> i8 {
        match self.line_indent.cmp(&parent_indent) {
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Less => -1,
        }
    }

    fn parse_node(
        &mut self,
        parent_indent: isize,
        context: Context,
        allow_to_seek: bool,
        allow_compact: bool,
        allow_property_mapping: bool,
    ) -> Result<bool> {
        if self.depth >= self.max_depth {
            let message = format!("nesting exceeded maxDepth ({})", self.max_depth);
            return self.fail(message);
        }
        if self.depth >= HARD_DEPTH_LIMIT {
            let message = format!("nesting exceeded the parser's hard limit ({HARD_DEPTH_LIMIT})");
            return self.fail(message);
        }

        self.depth += 1;

        let mut indent_status = 1;
        let mut at_new_line = false;
        let mut has_content = false;
        let mut property_start: Option<Snapshot> = None;
        let mut props = Props::default();

        let mut allow_block_scalars = matches!(context, Context::BlockOut | Context::BlockIn);
        let mut allow_block_collections = allow_block_scalars;
        let allow_block_styles = allow_block_scalars;

        if allow_to_seek && self.skip_separation_space(true) > 0 {
            at_new_line = true;
            indent_status = self.indent_status(parent_indent);
        }

        if indent_status == 1 {
            loop {
                let ch = self.ch();
                let property_state = self.snapshot();

                if at_new_line && indent_status != 1 && (ch == b'!' || ch == b'&') {
                    break;
                }

                if at_new_line
                    && allow_block_styles
                    && !props.is_empty()
                    && (ch == b'!' || ch == b'&')
                {
                    let fallback_state = self.snapshot();
                    let flow_indent = parent_indent + 1;
                    let mapping_indent = self.column(self.line_start, self.position);

                    if self.read_block_mapping(mapping_indent, flow_indent, props)?
                        && self.is_mapping_at(fallback_state.events_length)
                    {
                        self.depth -= 1;
                        return Ok(true);
                    }

                    self.restore(fallback_state);
                }

                if at_new_line
                    && ((ch == b'!' && props.tag.is_some())
                        || (ch == b'&' && props.anchor.is_some()))
                {
                    break;
                }

                if !self.read_tag_property(&mut props, context == Context::FlowIn)?
                    && !self.read_anchor_property(&mut props)?
                {
                    break;
                }

                if property_start.is_none() {
                    property_start = Some(property_state);
                }

                if self.skip_separation_space(true) > 0 {
                    at_new_line = true;
                    allow_block_collections = allow_block_styles;
                    indent_status = self.indent_status(parent_indent);
                } else {
                    allow_block_collections = false;
                }
            }
        }

        if allow_block_collections {
            allow_block_collections = at_new_line || allow_compact;
        }

        if indent_status == 1 || context == Context::BlockOut {
            let flow_indent = if matches!(context, Context::FlowIn | Context::FlowOut) {
                parent_indent
            } else {
                parent_indent + 1
            };
            let block_indent = self.column(self.line_start, self.position);

            if indent_status == 1 {
                if (allow_block_collections
                    && (self.read_block_sequence(block_indent, props)?
                        || self.read_block_mapping(block_indent, flow_indent, props)?))
                    || self.read_flow_collection(flow_indent, props)?
                {
                    has_content = true;
                } else {
                    let ch = self.ch();

                    let property_mapping = allow_property_mapping
                        && allow_block_styles
                        && !allow_block_collections
                        && ch != b'|'
                        && ch != b'>';
                    if let (Some(property_start), true) = (property_start, property_mapping) {
                        let fallback_state = self.snapshot();
                        let property_indent =
                            self.column(property_start.line_start, property_start.position);

                        self.restore(property_start);

                        if self.read_block_mapping(
                            property_indent,
                            flow_indent,
                            Props::default(),
                        )? && self.is_mapping_at(fallback_state.events_length)
                        {
                            has_content = true;
                        } else {
                            self.restore(fallback_state);
                        }
                    }

                    if !has_content
                        && ((allow_block_scalars && self.read_block_scalar(flow_indent, props)?)
                            || self.read_single_quoted_scalar(flow_indent, props)?
                            || self.read_double_quoted_scalar(flow_indent, props)?
                            || self.read_alias(props)?
                            || self.read_plain_scalar(flow_indent, context, props)?)
                    {
                        has_content = true;
                    }
                }
            } else if indent_status == 0 {
                has_content =
                    allow_block_collections && self.read_block_sequence(block_indent, props)?;
            }
        }

        allow_block_scalars = allow_block_scalars && !has_content;

        if !has_content && (!props.is_empty() || allow_block_scalars) {
            self.add_scalar_event(None, props, ScalarStyle::Plain, Chomping::Clip, -1, false);
            has_content = true;
        }

        self.depth -= 1;
        Ok(has_content || !props.is_empty())
    }

    fn read_directive(&mut self) -> Result<bool> {
        if self.line_indent > 0 || self.ch() != b'%' {
            return Ok(false);
        }

        self.position += 1;
        let name_start = self.position;

        while self.ch() != 0 && !is_ws_or_eol(self.ch()) {
            self.position += 1;
        }

        let name = (name_start, self.position);
        let mut args: Vec<(usize, usize)> = Vec::new();

        if name.0 == name.1 {
            return self.fail("directive name must not be less than one character in length");
        }

        while self.ch() != 0 && !is_eol(self.ch()) {
            while is_white_space(self.ch()) {
                self.position += 1;
            }
            let ch = self.ch();
            if ch == b'#' || is_eol(ch) || ch == 0 {
                break;
            }

            let start = self.position;
            while self.ch() != 0 && !is_ws_or_eol(self.ch()) {
                self.position += 1;
            }
            args.push((start, self.position));
        }

        if is_eol(self.ch()) {
            self.consume_line_break();
        }

        let arg = |i: usize| -> &str { self.slice(args[i].0, args[i].1) };
        match self.slice(name.0, name.1) {
            "YAML" => {
                if self
                    .directives
                    .iter()
                    .any(|d| matches!(d, Directive::Yaml { .. }))
                {
                    return self.fail("duplication of %YAML directive");
                }
                if args.len() != 1 {
                    return self.fail("YAML directive accepts exactly one argument");
                }
                let version = arg(0).to_owned();
                let Some((major, minor)) = version.split_once('.') else {
                    return self.fail("ill-formed argument of the YAML directive");
                };
                let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
                if !digits(major) || !digits(minor) {
                    return self.fail("ill-formed argument of the YAML directive");
                }
                // parseInt(major, 10) !== 1
                if major.trim_start_matches('0') != "1" {
                    return self.fail("unacceptable YAML version of the document");
                }
                self.directives.push(Directive::Yaml { version });
            }
            "TAG" => {
                if args.len() != 2 {
                    return self.fail("TAG directive accepts exactly two arguments");
                }
                let (handle, prefix) = (arg(0).to_owned(), arg(1).to_owned());
                if !is_tag_handle(&handle) {
                    return self
                        .fail("ill-formed tag handle (first argument) of the TAG directive");
                }
                if self.tag_handlers.contains_key(&handle) {
                    let message = format!(
                        "there is a previously declared suffix for \"{handle}\" tag handle"
                    );
                    return self.fail(message);
                }
                if !is_tag_prefix(&prefix) {
                    return self
                        .fail("ill-formed tag prefix (second argument) of the TAG directive");
                }
                self.tag_handlers.insert(handle.clone(), prefix.clone());
                self.directives.push(Directive::Tag { handle, prefix });
            }
            _ => {}
        }

        Ok(true)
    }

    fn read_document(&mut self) -> Result<()> {
        self.directives = Vec::new();
        self.tag_handlers.clear();
        let mut has_directives = false;

        self.skip_separation_space(true);

        while self.read_directive()? {
            has_directives = true;
            self.skip_separation_space(true);
        }

        let mut explicit_start = false;
        let mut explicit_end = false;
        let mut allow_compact = true;

        if self.line_indent == 0
            && self.ch() == b'-'
            && self.at(self.position + 1) == b'-'
            && self.at(self.position + 2) == b'-'
            && is_ws_or_eol_or_end(self.at(self.position + 3))
        {
            explicit_start = true;
            let marker_line = self.line;
            self.position += 3;
            self.skip_separation_space(true);
            allow_compact = self.line > marker_line;
        } else if has_directives {
            return self.fail("directives end mark is expected");
        }

        let document_event_index = self.events.len();
        if !explicit_start
            && self.position == self.line_start
            && self.ch() == b'.'
            && self.test_document_separator(self.position)
        {
            self.position += 3;
            self.skip_separation_space(true);
            return Ok(());
        }

        self.add_document_event(explicit_start, false);
        if !self.parse_node(
            self.line_indent - 1,
            Context::BlockOut,
            false,
            allow_compact,
            allow_compact,
        )? {
            self.add_empty_scalar_event();
        }
        self.skip_separation_space(true);

        if self.position == self.line_start && self.test_document_separator(self.position) {
            explicit_end = self.ch() == b'.';
            if explicit_end {
                let marker_line = self.line;
                self.position += 3;
                self.skip_separation_space(true);
                if self.line == marker_line && self.position < self.length {
                    return self.fail("end of the stream or a document separator is expected");
                }
            }
        }

        if let Some(Event::Document(document)) = self.events.get_mut(document_event_index) {
            document.explicit_end = explicit_end;
        }

        self.events.push(Event::Pop);

        if !explicit_end && self.position < self.length && !self.test_document_boundary() {
            return self.fail("end of the stream or a document separator is expected");
        }
        Ok(())
    }
}

/// Port of `parseEvents`: the event stream of every document in `input`.
pub fn parse_events(
    input: &str,
    options: &ParserOptions,
) -> std::result::Result<Vec<Event>, Error> {
    if let Some(nullpos) = memchr::memchr(0, input.as_bytes()) {
        return Err(Error::new("null byte is not allowed in input", nullpos));
    }

    let mut state = State {
        text: input,
        input: input.as_bytes(),
        length: input.len(),
        max_depth: options.max_depth,
        position: 0,
        line: 0,
        line_start: 0,
        line_indent: 0,
        first_tab_in_line: None,
        depth: 0,
        directives: Vec::new(),
        tag_handlers: HashMap::new(),
        // Typical YAML yields roughly one event per 6–10 bytes.
        events: Vec::with_capacity(input.len() / 8),
    };

    while state.position < state.length {
        state.skip_byte_order_mark();
        state.skip_separation_space(true);
        if state.position >= state.length {
            break;
        }
        let document_start = state.position;
        state.read_document()?;
        if state.position == document_start {
            // Internal progress guard: stop instead of looping forever.
            return state.fail("can not read a document");
        }
    }

    Ok(state.events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_patterns() {
        for (s, handle) in [
            ("!", true),
            ("!!", true),
            ("!a-1!", true),
            ("!!!", false),
            ("!a", false),
        ] {
            assert_eq!(is_tag_handle(s), handle, "{s}");
        }
        assert!(is_tag_uri("tag:yaml.org,2002:str"));
        assert!(is_tag_uri("a%20b![]"));
        assert!(!is_tag_uri("a%2"));
        assert!(!is_tag_suffix("a,b"));
        assert!(is_tag_suffix("foo%21"));
        assert!(is_tag_prefix("!local-"));
        assert!(is_tag_prefix("tag:example.com,2000:"));
        assert!(!is_tag_prefix(",bad"));
    }

    #[test]
    fn non_printable() {
        assert_eq!(find_non_printable(b"ok\t\n\r"), None);
        assert_eq!(find_non_printable("a\u{7}".as_bytes()), Some(1));
        assert_eq!(find_non_printable("é\u{85}x\u{9F}".as_bytes()), Some(5));
        assert_eq!(find_non_printable("\u{FFFE}".as_bytes()), Some(0));
    }

    #[test]
    fn deep_nesting_hits_the_hard_limit_without_overflowing() {
        let input = "[".repeat(HARD_DEPTH_LIMIT + 10);
        let options = ParserOptions {
            max_depth: usize::MAX,
        };
        // Debug builds use ~6.8 KB per level (see HARD_DEPTH_LIMIT), so give it room.
        let error = std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(move || parse_events(&input, &options).unwrap_err())
            .unwrap()
            .join()
            .unwrap();
        assert!(error.reason().contains("hard limit"), "{error}");
    }
}
