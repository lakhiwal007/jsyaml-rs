//! The event tape: one fixed-size `i32` record per event, in upstream's field order, so
//! `js/decode.cjs` can rebuild upstream's event objects for `constructFromEvents`.

use jsyaml::utf16::Utf16Cursor;
use jsyaml::{Directive, Event, Span};

/// `i32`s per event record.
pub const EVENT_STRIDE: usize = 11;

/// See the module docs. Records (`-1` = absent range, offsets in UTF-16 units):
///
/// | type | fields after `type` |
/// |---|---|
/// | 1 document | explicitStart, explicitEnd, index into `directives` |
/// | 2 sequence, 3 mapping | start, anchorStart, anchorEnd, tagStart, tagEnd, style |
/// | 4 scalar | valueStart, valueEnd, anchorStart, anchorEnd, tagStart, tagEnd, style, chomping, indent, fast |
/// | 5 alias | anchorStart, anchorEnd |
/// | 6 pop | — |
#[derive(Debug, Default)]
pub struct EventTape {
    /// `EVENT_STRIDE` × event count.
    pub records: Vec<i32>,
    /// A JSON array with one array of directive objects per document event.
    pub directives: String,
}

impl EventTape {
    pub(crate) fn encode(input: &str, events: &[Event]) -> Self {
        let mut utf16 = Utf16Cursor::new(input);
        let mut records = Vec::with_capacity(events.len() * EVENT_STRIDE);
        let mut directives = String::from("[");
        let mut documents = 0;

        for event in events {
            let mut r = [0i32; EVENT_STRIDE];
            r[0] = i32::from(event.id());
            let mut range = |at: usize, span: Option<Span>| {
                let (start, end) = span.map_or((-1, -1), |s| {
                    (
                        offset(utf16.to_utf16(s.start)),
                        offset(utf16.to_utf16(s.end)),
                    )
                });
                r[at] = start;
                r[at + 1] = end;
            };
            match event {
                Event::Document(d) => {
                    if documents > 0 {
                        directives.push(',');
                    }
                    directives_json(&mut directives, &d.directives);
                    r[1] = i32::from(d.explicit_start);
                    r[2] = i32::from(d.explicit_end);
                    r[3] = documents;
                    documents += 1;
                }
                Event::Sequence(c) | Event::Mapping(c) => {
                    range(2, c.anchor);
                    range(4, c.tag);
                    r[1] = offset(utf16.to_utf16(c.start));
                    r[6] = i32::from(c.style as u8);
                }
                Event::Scalar(s) => {
                    range(1, s.value);
                    range(3, s.anchor);
                    range(5, s.tag);
                    r[7] = i32::from(s.style as u8);
                    r[8] = i32::from(s.chomping as u8);
                    r[9] = s.indent;
                    r[10] = i32::from(s.fast);
                }
                Event::Alias(span) => range(1, Some(*span)),
                Event::Pop => {}
            }
            records.extend_from_slice(&r);
        }
        directives.push(']');
        EventTape {
            records,
            directives,
        }
    }
}

fn offset(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

fn directives_json(out: &mut String, directives: &[Directive]) {
    out.push('[');
    for (i, d) in directives.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match d {
            Directive::Yaml { version } => {
                out.push_str("{\"kind\":\"yaml\",\"version\":");
                json_string(out, version);
            }
            Directive::Tag { handle, prefix } => {
                out.push_str("{\"kind\":\"tag\",\"handle\":");
                json_string(out, handle);
                out.push_str(",\"prefix\":");
                json_string(out, prefix);
            }
        }
        out.push('}');
    }
    out.push(']');
}

fn json_string(out: &mut String, s: &str) {
    // Directive text is printable YAML; only quotes and backslashes need escaping.
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use jsyaml::{CollectionEvent, CollectionStyle, DocumentEvent};

    use super::*;

    #[test]
    fn records_use_utf16_offsets() {
        let input = "é: &a [x]";
        let events = [
            Event::Document(DocumentEvent {
                explicit_start: false,
                explicit_end: false,
                directives: vec![Directive::Tag {
                    handle: "!e!".into(),
                    prefix: "tag:\"q\"".into(),
                }],
            }),
            Event::Sequence(CollectionEvent {
                start: 7,
                anchor: Some(Span { start: 5, end: 6 }),
                tag: None,
                style: CollectionStyle::Flow,
            }),
            Event::Pop,
        ];
        let tape = EventTape::encode(input, &events);
        assert_eq!(
            &tape.records[EVENT_STRIDE..2 * EVENT_STRIDE],
            &[2, 6, 4, 5, -1, -1, 2, 0, 0, 0, 0]
        );
        assert_eq!(tape.records[2 * EVENT_STRIDE], 6);
        let directives: serde_json::Value = serde_json::from_str(&tape.directives).unwrap();
        assert_eq!(directives[0][0]["prefix"], "tag:\"q\"");
    }
}
