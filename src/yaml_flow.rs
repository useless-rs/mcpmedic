//! YAML inline values: flow collections (`[..]`/`{..}`), quoted and
//! plain scalars with typing (bools, ints, floats, null), and `\`-escapes.
//!
//! Split out of [`crate::yaml`].

use serde_json::{Map, Value};

/// Parse an inline value: a flow collection or a scalar.
pub(crate) fn parse_inline(text: &str) -> Result<Value, String> {
    let t = text.trim();
    if t.starts_with('&') || t.starts_with('*') || t.starts_with('!') {
        return Err(format!(
            "yaml: anchors, aliases and tags are not supported: `{t}`"
        ));
    }
    if t.starts_with('|') || t.starts_with('>') {
        return Err("yaml: block scalars are not supported".into());
    }
    if t.starts_with('[') || t.starts_with('{') {
        let mut pos = 0;
        let v = parse_flow(t, &mut pos)?;
        skip_flow_ws(t, &mut pos);
        if pos != t.len() {
            return Err(format!(
                "yaml: trailing content after flow value: `{}`",
                t[pos..].trim()
            ));
        }
        return Ok(v);
    }
    if t.starts_with('"') || t.starts_with('\'') {
        let mut pos = 0;
        let scanned = scan_quoted(t, &mut pos)?;
        if pos != t.len() {
            return Err(format!(
                "yaml: trailing content after quoted scalar: `{}`",
                t[pos..].trim()
            ));
        }
        return Ok(parse_scalar(scanned));
    }
    Ok(parse_scalar(t))
}

fn skip_flow_ws(text: &str, pos: &mut usize) {
    while matches!(text.as_bytes().get(*pos), Some(b' ' | b'\t')) {
        *pos += 1;
    }
}

/// Parse a flow collection starting at `*pos`. In map values, plain
/// scalars must stop at `:` (the key separator) — `stop_at_colon`
/// selects that mode for keys.
fn parse_flow(text: &str, pos: &mut usize) -> Result<Value, String> {
    match text.as_bytes().get(*pos) {
        Some(b'[') => {
            *pos += 1;
            let mut arr = Vec::new();
            loop {
                skip_flow_ws(text, pos);
                if text.as_bytes().get(*pos) == Some(&b']') {
                    *pos += 1;
                    return Ok(Value::Array(arr));
                }
                if !arr.is_empty() {
                    if text.as_bytes().get(*pos) != Some(&b',') {
                        return Err("yaml: expected `,` or `]` in flow sequence".into());
                    }
                    *pos += 1;
                    skip_flow_ws(text, pos);
                    if text.as_bytes().get(*pos) == Some(&b']') {
                        *pos += 1;
                        return Ok(Value::Array(arr));
                    }
                }
                arr.push(parse_flow_node(text, pos, false)?);
            }
        }
        Some(b'{') => {
            *pos += 1;
            let mut map = Map::new();
            loop {
                skip_flow_ws(text, pos);
                if text.as_bytes().get(*pos) == Some(&b'}') {
                    *pos += 1;
                    return Ok(Value::Object(map));
                }
                if !map.is_empty() {
                    if text.as_bytes().get(*pos) != Some(&b',') {
                        return Err("yaml: expected `,` or `}` in flow mapping".into());
                    }
                    *pos += 1;
                    skip_flow_ws(text, pos);
                }
                if text.as_bytes().get(*pos) == Some(&b'}') {
                    *pos += 1;
                    return Ok(Value::Object(map));
                }
                let k_node = parse_flow_node(text, pos, true)?;
                let Some(key) = k_node.as_str().map(str::to_string) else {
                    return Err("yaml: flow mapping key is not a string".into());
                };
                skip_flow_ws(text, pos);
                if text.as_bytes().get(*pos) != Some(&b':') {
                    return Err("yaml: expected `:` in flow mapping".into());
                }
                *pos += 1;
                let val = parse_flow_node(text, pos, false)?;
                map.insert(key, val);
            }
        }
        _ => Err("yaml: expected `[` or `{`".into()),
    }
}

fn parse_flow_node(text: &str, pos: &mut usize, stop_at_colon: bool) -> Result<Value, String> {
    skip_flow_ws(text, pos);
    match text.as_bytes().get(*pos) {
        Some(b'"' | b'\'') => {
            let quoted = scan_quoted(text, pos)?;
            Ok(parse_scalar(quoted))
        }
        Some(b'[' | b'{') => parse_flow(text, pos),
        Some(_) => {
            let start = *pos;
            while *pos < text.len() {
                match text.as_bytes()[*pos] {
                    b',' | b']' | b'}' => break,
                    b':' if stop_at_colon => break,
                    _ => *pos += 1,
                }
            }
            Ok(parse_scalar(&text[start..*pos]))
        }
        None => Err("yaml: unexpected end of flow value".into()),
    }
}

/// Scan a quoted scalar (cursor at the opening quote) and return the
/// full quoted slice with the cursor past the closing quote.
fn scan_quoted<'a>(text: &'a str, pos: &mut usize) -> Result<&'a str, String> {
    let quote = text.as_bytes()[*pos];
    let start = *pos;
    *pos += 1;
    while *pos < text.len() {
        let b = text.as_bytes()[*pos];
        if b == b'\\' && quote == b'"' {
            *pos += 2;
            continue;
        }
        if b == quote {
            if quote == b'\'' && text.as_bytes().get(*pos + 1) == Some(&b'\'') {
                *pos += 2;
                continue;
            }
            *pos += 1;
            return Ok(&text[start..*pos]);
        }
        *pos += 1;
    }
    Err("yaml: unterminated quoted scalar".into())
}

/// Parse a scalar with YAML typing: quoted strings, booleans, null,
/// numbers — everything else is a JSON string.
pub(crate) fn parse_scalar(text: &str) -> Value {
    let t = text.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        return Value::String(unescape_double(&t[1..t.len() - 1]));
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return Value::String(t[1..t.len() - 1].replace("''", "'"));
    }
    match t {
        "true" | "True" => return Value::Bool(true),
        "false" | "False" => return Value::Bool(false),
        "null" | "~" | "" => return Value::Null,
        _ => {}
    }
    if let Ok(n) = t.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(f) = t.parse::<f64>() {
        if let Some(v) = serde_json::Number::from_f64(f) {
            return Value::Number(v);
        }
    }
    Value::String(t.to_string())
}

fn unescape_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') | None => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}
