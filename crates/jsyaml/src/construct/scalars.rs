//! The `resolve` functions of the built-in scalar tags (`src/tag/scalar/*.ts`), plus the
//! JS number semantics they rely on (`parseInt`, `parseFloat`, `Number`, `String(n)`).
//! Upstream's regular expressions are written out as hand-rolled matchers.

use crate::schema::Tag;

/// What a scalar tag's `resolve` returned.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Resolved {
    Null,
    Bool(bool),
    Number(f64),
    /// `strTag` and `mergeTag` (whose value is the string `<<`).
    Str,
    Merge,
    Binary(Vec<u8>),
    Date(f64),
}

/// `implicitFirstChars`: `None` means any first char. The empty string stands for an
/// empty scalar, like upstream's `''` entry.
pub(crate) fn implicit_first_chars(tag: Tag) -> Option<&'static [&'static str]> {
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const SIGN_DIGITS: [&str; 11] = ["-", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const SIGNS_DIGITS: [&str; 12] = ["-", "+", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const FLOAT: [&str; 13] = [
        "-", "+", ".", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    ];
    Some(match tag {
        Tag::NullCore | Tag::NullYaml11 => &["", "~", "n", "N"],
        Tag::NullJson => &["n"],
        Tag::BoolCore => &["t", "T", "f", "F"],
        Tag::BoolJson => &["t", "f"],
        Tag::BoolYaml11 => &["y", "Y", "n", "N", "t", "T", "f", "F", "o", "O"],
        Tag::IntCore | Tag::IntYaml11 => &SIGNS_DIGITS,
        Tag::IntJson | Tag::FloatJson => &SIGN_DIGITS,
        Tag::FloatCore | Tag::FloatYaml11 => &FLOAT,
        Tag::Merge => &["<"],
        Tag::Timestamp => &DIGITS,
        _ => return None,
    })
}

/// Whether the tag takes part in implicit resolution (`implicit: true`).
pub(crate) fn is_implicit(tag: Tag) -> bool {
    !matches!(tag, Tag::Str | Tag::Binary) && tag.kind() == crate::schema::TagKind::Scalar
}

/// `tag.resolve(source, isExplicit)`; `None` is `NOT_RESOLVED`.
pub(crate) fn resolve(tag: Tag, source: &str, explicit: bool) -> Option<Resolved> {
    match tag {
        Tag::Str => Some(Resolved::Str),
        Tag::NullCore | Tag::NullYaml11 => {
            matches!(source, "" | "~" | "null" | "Null" | "NULL").then_some(Resolved::Null)
        }
        Tag::NullJson => {
            (source == "null" || (explicit && source.is_empty())).then_some(Resolved::Null)
        }
        Tag::BoolCore => bool_from(
            source,
            &["true", "True", "TRUE"],
            &["false", "False", "FALSE"],
        ),
        Tag::BoolJson => bool_from(source, &["true"], &["false"]),
        Tag::BoolYaml11 => bool_from(
            source,
            &[
                "true", "True", "TRUE", "y", "Y", "yes", "Yes", "YES", "on", "On", "ON",
            ],
            &[
                "false", "False", "FALSE", "n", "N", "no", "No", "NO", "off", "Off", "OFF",
            ],
        ),
        Tag::IntCore => int_core(source, explicit),
        Tag::IntJson => int_json(source, explicit),
        Tag::IntYaml11 => int_yaml11(source),
        Tag::FloatCore => float_core(source),
        Tag::FloatJson => float_json(source, explicit),
        Tag::FloatYaml11 => float_yaml11(source),
        Tag::Merge => {
            (source == "<<" || (explicit && source.is_empty())).then_some(Resolved::Merge)
        }
        Tag::Binary => binary(source),
        Tag::Timestamp => timestamp(source),
        _ => None,
    }
}

fn bool_from(source: &str, truthy: &[&str], falsy: &[&str]) -> Option<Resolved> {
    if truthy.contains(&source) {
        Some(Resolved::Bool(true))
    } else if falsy.contains(&source) {
        Some(Resolved::Bool(false))
    } else {
        None
    }
}

fn finite(n: f64) -> Option<Resolved> {
    n.is_finite().then_some(Resolved::Number(n))
}

// --- small matchers for upstream's patterns ---

fn all(s: &str, f: impl Fn(u8) -> bool) -> bool {
    s.bytes().all(f)
}

fn nonempty_all(s: &str, f: impl Fn(u8) -> bool) -> bool {
    !s.is_empty() && all(s, f)
}

fn is_octal(c: u8) -> bool {
    (b'0'..=b'7').contains(&c)
}

fn is_binary(c: u8) -> bool {
    c == b'0' || c == b'1'
}

fn strip_sign(s: &str) -> (f64, &str) {
    match s.as_bytes().first() {
        Some(b'-') => (-1.0, &s[1..]),
        Some(b'+') => (1.0, &s[1..]),
        _ => (1.0, s),
    }
}

/// `sign * parseInt(...)` of upstream's core/json `parseYamlInteger`.
fn parse_yaml_integer(source: &str) -> f64 {
    let (sign, value) = strip_sign(source);
    let magnitude = if let Some(d) = value.strip_prefix("0b") {
        parse_int_pow2(d, 1)
    } else if let Some(d) = value.strip_prefix("0o") {
        parse_int_pow2(d, 3)
    } else if let Some(d) = value.strip_prefix("0x") {
        parse_int_pow2(d, 4)
    } else {
        parse_decimal(value)
    };
    sign * magnitude
}

/// `[-+]?0b[01]+ | [-+]?0o[0-7]+ | [-+]?0x[0-9a-fA-F]+ | [-+]?[0-9]+`
fn is_explicit_integer(source: &str) -> bool {
    let (_, v) = strip_sign(source);
    if let Some(d) = v.strip_prefix("0b") {
        if nonempty_all(d, is_binary) {
            return true;
        }
    }
    if let Some(d) = v.strip_prefix("0o") {
        if nonempty_all(d, is_octal) {
            return true;
        }
    }
    if let Some(d) = v.strip_prefix("0x") {
        if nonempty_all(d, |c| c.is_ascii_hexdigit()) {
            return true;
        }
    }
    nonempty_all(v, |c| c.is_ascii_digit())
}

fn int_core(source: &str, explicit: bool) -> Option<Resolved> {
    let ok = if explicit {
        is_explicit_integer(source)
    } else {
        // 0o[0-7]+ | 0x[0-9a-fA-F]+ | [-+]?[0-9]+
        source
            .strip_prefix("0o")
            .is_some_and(|d| nonempty_all(d, is_octal))
            || source
                .strip_prefix("0x")
                .is_some_and(|d| nonempty_all(d, |c| c.is_ascii_hexdigit()))
            || nonempty_all(strip_sign(source).1, |c| c.is_ascii_digit())
    };
    if ok {
        finite(parse_yaml_integer(source))
    } else {
        None
    }
}

fn int_json(source: &str, explicit: bool) -> Option<Resolved> {
    let ok = if explicit {
        is_explicit_integer(source)
    } else {
        // -?(0|[1-9][0-9]*)
        let v = source.strip_prefix('-').unwrap_or(source);
        v == "0"
            || (v
                .as_bytes()
                .first()
                .is_some_and(|c| (b'1'..=b'9').contains(c))
                && all(v, |c| c.is_ascii_digit()))
    };
    if ok {
        finite(parse_yaml_integer(source))
    } else {
        None
    }
}

/// `[-+]?[0-9][0-9_]*(?::[0-5]?[0-9])+` after an optional sign has been removed.
fn is_sexagesimal_int(v: &str) -> bool {
    let mut parts = v.split(':');
    let Some(head) = parts.next() else {
        return false;
    };
    let mut count = 0;
    for part in parts {
        let b = part.as_bytes();
        let ok = match b {
            [d] => d.is_ascii_digit(),
            [a, d] => (b'0'..=b'5').contains(a) && d.is_ascii_digit(),
            _ => false,
        };
        if !ok {
            return false;
        }
        count += 1;
    }
    count > 0
        && head.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && all(head, |c| c.is_ascii_digit() || c == b'_')
}

fn int_yaml11(source: &str) -> Option<Resolved> {
    let (_, v) = strip_sign(source);
    let digits_ = |s: &str, f: fn(u8) -> bool| nonempty_all(s, |c| f(c) || c == b'_');
    let ok = v.strip_prefix("0b").is_some_and(|d| digits_(d, is_binary))
        || (v.len() > 1 && v.starts_with('0') && digits_(&v[1..], is_octal))
        || v.strip_prefix("0x")
            .is_some_and(|d| digits_(d, |c| c.is_ascii_hexdigit()))
        || is_sexagesimal_int(v)
        || v == "0"
        || (v
            .as_bytes()
            .first()
            .is_some_and(|c| (b'1'..=b'9').contains(c))
            && all(v, |c| c.is_ascii_digit() || c == b'_'));
    if !ok {
        return None;
    }

    let cleaned = source.replace('_', "");
    let (sign, value) = strip_sign(&cleaned);
    let magnitude = if let Some(d) = value.strip_prefix("0b") {
        parse_int_pow2(d, 1)
    } else if let Some(d) = value.strip_prefix("0x") {
        parse_int_pow2(d, 4)
    } else if value.contains(':') {
        value
            .split(':')
            .fold(0.0, |acc, part| acc * 60.0 + js_number(part))
    } else if value != "0" && value.starts_with('0') {
        parse_int_pow2(&value[1..], 3)
    } else {
        parse_decimal(value)
    };
    finite(sign * magnitude)
}

/// Core float pattern (also json's explicit one):
/// `[-+]?[0-9]+(\.[0-9]*)?([eE][-+]?[0-9]+)? | [-+]?\.[0-9]+([eE][-+]?[0-9]+)? | [-+]?\.(inf|Inf|INF) | \.(nan|NaN|NAN)`
fn is_core_float(source: &str) -> bool {
    if matches!(source, ".nan" | ".NaN" | ".NAN") {
        return true;
    }
    let (_, v) = strip_sign(source);
    if matches!(v, ".inf" | ".Inf" | ".INF") {
        return true;
    }
    let (mantissa, exponent) = match v.find(['e', 'E']) {
        Some(i) => (&v[..i], Some(&v[i + 1..])),
        None => (v, None),
    };
    if let Some(e) = exponent {
        let e = e.strip_prefix(['-', '+']).unwrap_or(e);
        if !nonempty_all(e, |c| c.is_ascii_digit()) {
            return false;
        }
    }
    match mantissa.split_once('.') {
        Some(("", frac)) => nonempty_all(frac, |c| c.is_ascii_digit()),
        Some((int, frac)) => {
            nonempty_all(int, |c| c.is_ascii_digit()) && all(frac, |c| c.is_ascii_digit())
        }
        None => nonempty_all(mantissa, |c| c.is_ascii_digit()),
    }
}

fn is_float_special(source: &str) -> bool {
    matches!(source, ".nan" | ".NaN" | ".NAN")
        || matches!(strip_sign(source).1, ".inf" | ".Inf" | ".INF")
}

/// The shared tail of core float and json explicit float: sign, `.inf`/`.nan`, parseFloat.
fn float_value(source: &str) -> Result<f64, ()> {
    let lower = source.to_ascii_lowercase();
    let (sign, value) = match lower.as_bytes().first() {
        Some(b'-') => (-1.0, &lower[1..]),
        Some(b'+') => (1.0, &lower[1..]),
        _ => (1.0, &lower[..]),
    };
    match value {
        ".inf" => Ok(sign * f64::INFINITY),
        ".nan" => Ok(f64::NAN),
        _ => parse_float(value).map(|n| sign * n).ok_or(()),
    }
}

fn float_core(source: &str) -> Option<Resolved> {
    if !is_core_float(source) {
        return None;
    }
    let result = float_value(source).unwrap_or(f64::NAN);
    (result.is_finite() || is_float_special(source)).then_some(Resolved::Number(result))
}

fn float_json(source: &str, explicit: bool) -> Option<Resolved> {
    if explicit {
        if !is_core_float(source) {
            return None;
        }
        // `.inf` / `.nan` are returned as such; anything else must be finite.
        return match float_value(source) {
            Ok(n) if is_float_special(source) => Some(Resolved::Number(n)),
            Ok(n) => finite(n),
            Err(()) => None,
        };
    }
    // -?(0|[1-9][0-9]*)(\.[0-9]*)?([eE][-+]?[0-9]+)?
    let v = source.strip_prefix('-').unwrap_or(source);
    let (mantissa, exponent) = match v.find(['e', 'E']) {
        Some(i) => (&v[..i], Some(&v[i + 1..])),
        None => (v, None),
    };
    if let Some(e) = exponent {
        let e = e.strip_prefix(['-', '+']).unwrap_or(e);
        if !nonempty_all(e, |c| c.is_ascii_digit()) {
            return None;
        }
    }
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let int_ok = int == "0"
        || (int
            .as_bytes()
            .first()
            .is_some_and(|c| (b'1'..=b'9').contains(c))
            && all(int, |c| c.is_ascii_digit()));
    if !int_ok || !all(frac, |c| c.is_ascii_digit()) {
        return None;
    }
    finite(js_number(source))
}

/// YAML 1.1 float pattern:
/// `[-+]?([0-9][0-9_]*)?\.[0-9_]*([eE][-+][0-9]+)? | [-+]?[0-9][0-9_]*(:[0-5]?[0-9])+\.[0-9_]* | [-+]?\.(inf|Inf|INF) | \.(nan|NaN|NAN)`
fn is_yaml11_float(source: &str) -> bool {
    if is_float_special(source) {
        return true;
    }
    let (_, v) = strip_sign(source);
    let digits_ = |s: &str| all(s, |c| c.is_ascii_digit() || c == b'_');
    // Sexagesimal: base60 int part, then `.` and fraction.
    if let Some((head, frac)) = v.rsplit_once('.') {
        if head.contains(':') && is_sexagesimal_int(head) && digits_(frac) {
            return true;
        }
    }
    let (mantissa, exponent) = match v.find(['e', 'E']) {
        Some(i) => (&v[..i], Some(&v[i + 1..])),
        None => (v, None),
    };
    if let Some(e) = exponent {
        let Some(e) = e.strip_prefix(['-', '+']) else {
            return false;
        };
        if !nonempty_all(e, |c| c.is_ascii_digit()) {
            return false;
        }
    }
    let Some((int, frac)) = mantissa.split_once('.') else {
        return false;
    };
    let int_ok = int.is_empty() || (int.as_bytes()[0].is_ascii_digit() && digits_(int));
    int_ok && digits_(frac)
}

fn float_yaml11(source: &str) -> Option<Resolved> {
    if !is_yaml11_float(source) {
        return None;
    }
    let value = source.to_ascii_lowercase().replace('_', "");
    let (sign, value) = match value.as_bytes().first() {
        Some(b'-') => (-1.0, value[1..].to_owned()),
        Some(b'+') => (1.0, value[1..].to_owned()),
        _ => (1.0, value),
    };
    if value == ".inf" {
        return Some(Resolved::Number(sign * f64::INFINITY));
    }
    if value == ".nan" {
        return Some(Resolved::Number(f64::NAN));
    }
    let result = if value.contains(':') {
        sign * value
            .split(':')
            .fold(0.0, |acc, part| acc * 60.0 + js_number(part))
    } else {
        sign * parse_float(&value).unwrap_or(f64::NAN)
    };
    (result.is_finite() || is_float_special(source)).then_some(Resolved::Number(result))
}

/// JS `\s` (whitespace and line terminators, including Unicode spaces and the BOM).
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

fn binary(source: &str) -> Option<Resolved> {
    let input: String = source.chars().filter(|&c| !is_js_whitespace(c)).collect();
    let b = input.as_bytes();
    let body = input.trim_end_matches('=');
    let padding = input.len() - body.len();
    let alphabet = |c: u8| c.is_ascii_alphanumeric() || c == b'+' || c == b'/';
    if b.len() % 4 != 0 || padding > 2 || !all(body, alphabet) {
        return None;
    }
    // atob (forgiving-base64): extra bits in the last char are discarded.
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let (mut acc, mut nbits) = (0u32, 0u32);
    for &c in body.as_bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        acc = (acc << 6) | u32::from(v);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((acc >> nbits) as u8);
            acc &= (1 << nbits) - 1;
        }
    }
    Some(Resolved::Binary(out))
}

// --- timestamps ---

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    // Howard Hinnant's algorithm, proleptic Gregorian (as JS Date).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

fn take_digits<'a>(s: &mut &'a str, min: usize, max: usize) -> Option<&'a str> {
    let n = s.bytes().take(max).take_while(u8::is_ascii_digit).count();
    if n < min {
        return None;
    }
    let (digits, rest) = s.split_at(n);
    *s = rest;
    Some(digits)
}

fn eat(s: &mut &str, c: char) -> bool {
    match s.strip_prefix(c) {
        Some(rest) => {
            *s = rest;
            true
        }
        None => false,
    }
}

fn num(digits: &str) -> i64 {
    digits.parse().unwrap_or(0)
}

fn timestamp(source: &str) -> Option<Resolved> {
    let mut s = source;
    let year = num(take_digits(&mut s, 4, 4)?);
    if !eat(&mut s, '-') {
        return None;
    }
    let month_digits = take_digits(&mut s, 1, 2)?;
    if !eat(&mut s, '-') {
        return None;
    }
    let day_digits = take_digits(&mut s, 1, 2)?;
    let (month, day) = (num(month_digits), num(day_digits));
    let valid_date =
        |y: i64, m: i64, d: i64| (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m);

    if s.is_empty() {
        // YAML_DATE_REGEXP needs two-digit month and day.
        if month_digits.len() != 2 || day_digits.len() != 2 || !valid_date(year, month, day) {
            return None;
        }
        return Some(Resolved::Date(
            days_from_civil(year, month, day) as f64 * 86_400_000.0,
        ));
    }

    // (?:[Tt]|[ \t]+)
    if !eat(&mut s, 'T') && !eat(&mut s, 't') {
        let n = s.bytes().take_while(|&c| c == b' ' || c == b'\t').count();
        if n == 0 {
            return None;
        }
        s = &s[n..];
    }
    let hour = num(take_digits(&mut s, 1, 2)?);
    if !eat(&mut s, ':') {
        return None;
    }
    let minute = num(take_digits(&mut s, 2, 2)?);
    if !eat(&mut s, ':') {
        return None;
    }
    let second = num(take_digits(&mut s, 2, 2)?);
    let mut fraction = 0;
    if eat(&mut s, '.') {
        let digits = take_digits(&mut s, 0, usize::MAX).unwrap_or("");
        let mut value: String = digits.chars().take(3).collect();
        while value.len() < 3 {
            value.push('0');
        }
        fraction = num(&value);
    }
    // (?:[ \t]*(Z|([-+])([0-9][0-9]?)(?::([0-9][0-9]))?))?$
    let mut offset_ms = 0;
    if !s.is_empty() {
        let n = s.bytes().take_while(|&c| c == b' ' || c == b'\t').count();
        let mut t = &s[n..];
        if eat(&mut t, 'Z') {
            // UTC
        } else {
            let sign = match t.as_bytes().first() {
                Some(b'-') => -1,
                Some(b'+') => 1,
                _ => return None,
            };
            t = &t[1..];
            let offset_hour = num(take_digits(&mut t, 1, 2)?);
            let mut offset_minute = 0;
            if eat(&mut t, ':') {
                offset_minute = num(take_digits(&mut t, 2, 2)?);
            }
            if offset_hour > 23 || offset_minute > 59 {
                return None;
            }
            offset_ms = sign * (offset_hour * 60 + offset_minute) * 60_000;
        }
        if !t.is_empty() {
            return None;
        }
    }

    if hour > 23 || minute > 59 || second > 59 || !valid_date(year, month, day) {
        return None;
    }
    let ms = days_from_civil(year, month, day) * 86_400_000
        + hour * 3_600_000
        + minute * 60_000
        + second * 1000
        + fraction
        - offset_ms;
    Some(Resolved::Date(ms as f64))
}

// --- JS number semantics ---

/// `parseInt(digits, 10)` for an all-digit string: correctly rounded, like V8.
fn parse_decimal(digits: &str) -> f64 {
    digits.parse().unwrap_or(f64::NAN)
}

/// `parseInt(digits, 2 ** bits)` for valid digits, rounded to nearest-even as V8 does.
fn parse_int_pow2(digits: &str, bits: u32) -> f64 {
    if digits.is_empty() {
        return f64::NAN; // parseInt('', radix)
    }
    let mut mantissa: u64 = 0;
    let mut width = 0u32; // significant bits held in `mantissa`
    let mut dropped = 0i32; // bits shifted out beyond 64
    let mut sticky = false;
    for c in digits.bytes() {
        let Some(d) = (c as char).to_digit(16) else {
            return f64::NAN;
        };
        for i in (0..bits).rev() {
            let bit = u64::from((d >> i) & 1);
            if width == 0 && bit == 0 {
                continue;
            }
            if width < 64 {
                mantissa = (mantissa << 1) | bit;
                width += 1;
            } else {
                dropped += 1;
                sticky |= bit == 1;
            }
        }
    }
    if width <= 53 {
        return mantissa as f64 * 2f64.powi(dropped);
    }
    let shift = width - 53;
    let rest = mantissa & ((1u64 << shift) - 1);
    let half = 1u64 << (shift - 1);
    let mut top = mantissa >> shift;
    if rest > half || (rest == half && (sticky || top & 1 == 1)) {
        top += 1;
    }
    let exponent = i32::try_from(shift)
        .unwrap_or(i32::MAX)
        .saturating_add(dropped);
    top as f64 * 2f64.powi(exponent)
}

/// `parseFloat` of a string already validated as a decimal literal (`1.`, `.5`, `1.e5`).
fn parse_float(s: &str) -> Option<f64> {
    if s.is_empty() || s == "." {
        return None;
    }
    s.parse().ok()
}

/// `Number(s)` for the digit strings the patterns let through.
fn js_number(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    parse_float(s).unwrap_or(f64::NAN)
}

/// `String(n)` (ECMAScript `Number::toString`).
pub(crate) fn js_number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n == 0.0 {
        return "0".into();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    let sign = if n < 0.0 { "-" } else { "" };
    // Shortest round-trip digits, as `d.ddde±x`.
    let sci = format!("{:e}", n.abs());
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let k = i64::try_from(digits.len()).unwrap_or(0);
    let point = exponent.parse::<i64>().unwrap_or(0) + 1; // `n` in the spec
    let zeros = |count: i64| "0".repeat(usize::try_from(count).unwrap_or(0));
    let body = if k <= point && point <= 21 {
        format!("{digits}{}", zeros(point - k))
    } else if 0 < point && point <= 21 {
        let (int, frac) = digits.split_at(usize::try_from(point).unwrap_or(0));
        format!("{int}.{frac}")
    } else if -6 < point && point <= 0 {
        format!("0.{}{digits}", zeros(-point))
    } else {
        let e = point - 1;
        let exp = if e >= 0 {
            format!("+{e}")
        } else {
            e.to_string()
        };
        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            format!("{first}e{exp}")
        } else {
            format!("{first}.{rest}e{exp}")
        }
    };
    format!("{sign}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_to_string_matches_js() {
        for (n, s) in [
            (1.0, "1"),
            (1.5, "1.5"),
            (-0.000_001, "-0.000001"),
            (1e-7, "1e-7"),
            (1e21, "1e+21"),
            (123_456_789_012_345_680_000.0, "123456789012345680000"),
            (0.1 + 0.2, "0.30000000000000004"),
            (5e-324, "5e-324"),
            (1.797_693_134_862_315_7e308, "1.7976931348623157e+308"),
        ] {
            assert_eq!(js_number_to_string(n), s);
        }
    }

    #[test]
    fn pow2_parse_rounds_like_v8() {
        assert_eq!(parse_int_pow2("1F", 4), 31.0);
        assert_eq!(parse_int_pow2("20000000000001", 4), 9_007_199_254_740_992.0); // 2^53 + 1 → even
        assert_eq!(parse_int_pow2("20000000000003", 4), 9_007_199_254_740_996.0); // ties up to even
        assert_eq!(parse_int_pow2("17", 3), 15.0);
    }

    #[test]
    fn timestamps() {
        assert_eq!(
            timestamp("2001-12-14t21:59:43.10-05:00"),
            Some(Resolved::Date(1_008_385_183_100.0))
        );
        assert_eq!(
            timestamp("2002-12-14"),
            Some(Resolved::Date(1_039_824_000_000.0))
        );
        assert_eq!(timestamp("2023-02-29"), None);
        assert_eq!(timestamp("2001-1-1"), None);
        assert_eq!(
            timestamp("0001-02-28"),
            Some(Resolved::Date(-62_130_585_600_000.0)) // as Node computes
        );
    }
}
