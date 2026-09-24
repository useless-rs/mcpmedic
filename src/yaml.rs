//! A minimal YAML subset parser for MCP config files (Goose,
//! Continue): block mappings by indentation, block sequences —
//! including indentless ones under a mapping key — flow sequences
//! and maps, and plain/quoted scalars. Anything outside the subset
//! (anchors, aliases, tags, block scalars, tabs for indentation) is
//! a clean parse error: mcpmedic never guesses. Zero dependencies —
//! the output is a `serde_json::Value`, so YAML configs flow through
//! the same downstream machinery as JSON ones.

use serde_json::{Map, Value};

/// One content line: (indentation, content, line number).
type Line<'a> = (usize, &'a str, usize);

/// Parse a YAML document into a JSON value tree.
pub(crate) fn parse_yaml(source: &str) -> Result<Value, String> {
    let lines = prepare(source)?;
    if lines.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let mut idx = 0;
    let value = parse_node(&lines, &mut idx, lines[0].0, 0)?;
    if idx != lines.len() {
        let (_, content, ln) = lines[idx];
        return Err(format!(
            "yaml: unexpected content `{content}` at line {ln} (indentation mismatch?)"
        ));
    }
    Ok(value)
}

/// Split into comment-stripped content lines, rejecting out-of-subset
/// constructs up front so the recursive parser only sees clean input.
fn prepare(source: &str) -> Result<Vec<Line<'_>>, String> {
    let mut out = Vec::new();
    for (n, raw) in source.lines().enumerate() {
        let ln = n + 1;
        let ws_end = raw.len() - raw.trim_start_matches([' ', '\t']).len();
        let ws = &raw[..ws_end];
        if ws.contains('\t') {
            return Err(format!("yaml: tab in indentation at line {ln}"));
        }
        let content = strip_comment(&raw[ws_end..]).trim();
        if content.is_empty() || content == "---" || content == "..." {
            continue;
        }
        if content.starts_with('&') || content.starts_with('*') || content.starts_with('!') {
            return Err(format!(
                "yaml: anchors, aliases and tags are not supported (line {ln})"
            ));
        }
        if content == "|"
            || content == ">"
            || content.starts_with("| ")
            || content.starts_with("> ")
        {
            return Err(format!("yaml: block scalars are not supported (line {ln})"));
        }
        out.push((ws.len(), content, ln));
    }
    Ok(out)
}

/// Strip a trailing comment. `#` starts a comment when at the start of
/// the content or preceded by whitespace, and never inside quotes.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_double {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_double = false;
            }
        } else if in_single {
            if b == b'\'' {
                in_single = false;
            }
        } else if b == b'"' {
            in_double = true;
        } else if b == b'\'' {
            in_single = true;
        } else if b == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return line[..i].trim_end();
        }
        i += 1;
    }
    line.trim_end()
}

fn is_dash(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

/// Parse the node starting at `lines[*idx]`, whose first line sits at
/// column `indent`.
/// Block nesting beyond this is rejected instead of recursing: real MCP
/// configs nest 3-4 levels deep, so anything past the cap is either hostile
/// or broken — and unbounded recursion would overflow the stack.
const MAX_DEPTH: usize = 64;

fn parse_node(
    lines: &[Line<'_>],
    idx: &mut usize,
    indent: usize,
    depth: usize,
) -> Result<Value, String> {
    if depth >= MAX_DEPTH {
        return Err("yaml: nesting too deep (limit 64)".to_owned());
    }
    if is_dash(lines[*idx].1) {
        parse_sequence(lines, idx, indent, depth)
    } else {
        parse_mapping(lines, idx, indent, None, depth)
    }
}

/// Parse a block mapping whose keys sit at column `indent`. `seed`
/// carries a key/value pair already consumed from a `- key: value`
/// sequence item, so the loop can continue with the remaining keys.
fn parse_mapping(
    lines: &[Line<'_>],
    idx: &mut usize,
    indent: usize,
    seed: Option<(String, Value)>,
    depth: usize,
) -> Result<Value, String> {
    let mut map = Map::new();
    if let Some((k, v)) = seed {
        map.insert(k, v);
    }
    while *idx < lines.len() {
        let (li, content, ln) = lines[*idx];
        if li < indent || (li == indent && is_dash(content)) {
            break;
        }
        if li > indent {
            return Err(format!("yaml: unexpected indentation at line {ln}"));
        }
        let (k_text, rest) = split_key(content)
            .ok_or_else(|| format!("yaml: expected `key: value` at line {ln}"))?;
        let key = key_string(k_text, ln)?;
        *idx += 1;
        let val = parse_value_after_key(lines, idx, indent, rest, depth)?;
        if map.insert(key, val).is_some() {
            return Err(format!("yaml: duplicate key at line {ln}"));
        }
    }
    Ok(Value::Object(map))
}

/// Parse a block sequence whose `-` markers sit at column `indent`.
fn parse_sequence(
    lines: &[Line<'_>],
    idx: &mut usize,
    indent: usize,
    depth: usize,
) -> Result<Value, String> {
    let mut arr = Vec::new();
    while *idx < lines.len() {
        let (li, content, _) = lines[*idx];
        if li != indent || !is_dash(content) {
            break;
        }
        let item = content.strip_prefix("- ").map_or("", str::trim);
        *idx += 1;
        if item.is_empty() {
            if *idx < lines.len() && lines[*idx].0 > indent {
                arr.push(parse_node(lines, idx, lines[*idx].0, depth + 1)?);
            } else {
                arr.push(Value::Null);
            }
        } else if let Some((k_text, rest)) = split_key(item) {
            let item_indent = indent + 2;
            let ln = lines[*idx - 1].2;
            let key = key_string(k_text, ln)?;
            let val = parse_value_after_key(lines, idx, item_indent, rest, depth)?;
            arr.push(parse_mapping(
                lines,
                idx,
                item_indent,
                Some((key, val)),
                depth,
            )?);
        } else {
            arr.push(parse_inline(item)?);
        }
    }
    Ok(Value::Array(arr))
}

/// The value for `key:` — either inline, a nested block (deeper
/// indent), an indentless sequence (same indent, dash items), or null.
fn parse_value_after_key(
    lines: &[Line<'_>],
    idx: &mut usize,
    key_indent: usize,
    rest: &str,
    depth: usize,
) -> Result<Value, String> {
    if !rest.is_empty() {
        return parse_inline(rest);
    }
    if *idx < lines.len() && lines[*idx].0 > key_indent {
        return parse_node(lines, idx, lines[*idx].0, depth + 1);
    }
    if *idx < lines.len() && lines[*idx].0 == key_indent && is_dash(lines[*idx].1) {
        return parse_sequence(lines, idx, key_indent, depth);
    }
    Ok(Value::Null)
}

/// Split `key: value` (or `key:`) into its parts. Handles quoted keys.
fn split_key(content: &str) -> Option<(&str, &str)> {
    if content.starts_with('"') || content.starts_with('\'') {
        let quote = content.as_bytes()[0];
        let mut i = 1;
        while i < content.len() {
            let b = content.as_bytes()[i];
            if b == b'\\' && quote == b'"' {
                i += 2;
                continue;
            }
            if b == quote {
                if quote == b'\'' && content.as_bytes().get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                break;
            }
            i += 1;
        }
        if i >= content.len() {
            return None;
        }
        let rest = content.get(i + 1..)?.strip_prefix(':')?;
        return Some((&content[..=i], rest.trim_start()));
    }
    if let Some(pos) = content.find(": ") {
        let k = &content[..pos];
        if k.contains('#') {
            return None;
        }
        return Some((k, content[pos + 2..].trim()));
    }
    if let Some(k) = content.strip_suffix(':') {
        return Some((k, ""));
    }
    None
}

fn key_string(k_text: &str, ln: usize) -> Result<String, String> {
    match parse_scalar(k_text) {
        Value::String(s) => Ok(s),
        other => Err(format!(
            "yaml: key `{other}` at line {ln} is not a plain or quoted string"
        )),
    }
}

/// Parse an inline value: a flow collection or a scalar.
fn parse_inline(text: &str) -> Result<Value, String> {
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
fn parse_scalar(text: &str) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_goose_extensions_shape() {
        let src = r#"provider: anthropic
model: claude-4.5-sonnet

extensions:
  - name: filesystem
    enabled: true
    transport:
      type: stdio
      command: npx
      args: ["-y", "@modelcontextprotocol/server-filesystem", "./"]
  - name: postgres
    enabled: false
    transport:
      type: stdio
      command: uvx
      args: ["mcp-server-postgres", "postgresql://localhost/mydb"]
    env:
      DATABASE_URL: postgresql://localhost/mydb

preferences:  # trailing comment
  telemetry: true
"#;
        let v = parse_yaml(src).unwrap();
        assert_eq!(v["provider"], "anthropic");
        let ext = v["extensions"].as_array().unwrap();
        assert_eq!(ext.len(), 2);
        assert_eq!(ext[0]["name"], "filesystem");
        assert_eq!(ext[0]["enabled"], true);
        assert_eq!(ext[0]["transport"]["type"], "stdio");
        assert_eq!(ext[0]["transport"]["command"], "npx");
        assert_eq!(
            ext[0]["transport"]["args"][1],
            "@modelcontextprotocol/server-filesystem"
        );
        assert_eq!(ext[1]["enabled"], false);
        assert_eq!(ext[1]["env"]["DATABASE_URL"], "postgresql://localhost/mydb");
        assert_eq!(v["preferences"]["telemetry"], true);
    }

    #[test]
    fn parses_continue_mcp_servers_shape() {
        let src = r"name: My Config
version: 1.0.0
schema: v1
mcpServers:
  - name: My MCP Server
    command: uvx
    args:
      - mcp-server-sqlite
      - --db-path
      - ./test.db
    cwd: /Users/NAME/project
    env:
      NODE_ENV: production
";
        let v = parse_yaml(src).unwrap();
        assert_eq!(v["name"], "My Config");
        assert_eq!(v["version"], "1.0.0");
        let servers = v["mcpServers"].as_array().unwrap();
        assert_eq!(servers[0]["name"], "My MCP Server");
        assert_eq!(servers[0]["command"], "uvx");
        let args = servers[0]["args"].as_array().unwrap();
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "mcp-server-sqlite");
        assert_eq!(args[2], "./test.db");
        assert_eq!(servers[0]["env"]["NODE_ENV"], "production");
    }

    #[test]
    fn parses_indentless_sequence() {
        let v = parse_yaml("key:\n- a\n- b\n").unwrap();
        assert_eq!(v["key"][0], "a");
        assert_eq!(v["key"][1], "b");
    }

    #[test]
    fn parses_flow_collections_and_typed_scalars() {
        let v = parse_yaml(
            "args: [1, \"two\", 'three']\nenv: { A: b, C: \"d\" }\nflag: true\nnum: 42\nratio: 0.5\nnothing: null\nempty:\n",
        )
        .unwrap();
        assert_eq!(v["args"][0], 1);
        assert_eq!(v["args"][1], "two");
        assert_eq!(v["args"][2], "three");
        assert_eq!(v["env"]["A"], "b");
        assert_eq!(v["flag"], true);
        assert_eq!(v["num"], 42);
        assert!((v["ratio"].as_f64().unwrap() - 0.5).abs() < f64::EPSILON);
        assert!(v["nothing"].is_null());
        assert!(v["empty"].is_null());
    }

    #[test]
    fn ignores_comments_and_preserves_hash_in_scalars() {
        let v = parse_yaml("# leading\nkey: value  # trailing\nkey2: a#b\n").unwrap();
        assert_eq!(v["key"], "value");
        assert_eq!(v["key2"], "a#b");
    }

    #[test]
    fn parses_nested_maps_and_quoted_scalars() {
        let v = parse_yaml(
            "\"quoted key\": 'single'\nmap:\n  inner: \"double \\\"escaped\\\" and \\\\ backslash\"\n",
        )
        .unwrap();
        assert_eq!(v["quoted key"], "single");
        assert_eq!(v["map"]["inner"], "double \"escaped\" and \\ backslash");
    }

    #[test]
    fn parses_empty_and_document_markers() {
        assert_eq!(parse_yaml("").unwrap(), serde_json::json!({}));
        assert_eq!(
            parse_yaml("---\n# only comments\n...\n").unwrap(),
            serde_json::json!({})
        );
    }

    #[test]
    fn rejects_out_of_subset() {
        assert!(parse_yaml("key:\n\t- a\n").is_err(), "tab in indentation");
        assert!(parse_yaml("a: &anchor value\n").is_err(), "anchor in value");
        assert!(parse_yaml("a: |\n  text\n").is_err(), "block scalar");
        assert!(parse_yaml("a: [1, 2\n").is_err(), "unterminated flow");
        assert!(
            parse_yaml("a: \"unterminated\n").is_err(),
            "unterminated quote"
        );
        assert!(parse_yaml("a: 1\n  b: 2\n").is_err(), "unexpected indent");
        assert!(
            parse_yaml("just a scalar\n").is_err(),
            "scalar where mapping expected"
        );
        assert!(parse_yaml("a: 1\na: 2\n").is_err(), "duplicate key");
    }

    #[test]
    fn rejects_pathological_nesting() {
        let mut doc = String::new();
        for i in 0..200 {
            doc.push_str(&" ".repeat(i * 2));
            doc.push_str("k:\n");
        }
        doc.push_str(&" ".repeat(400));
        doc.push_str("v: 1\n");
        let err = parse_yaml(&doc).expect_err("200-deep nesting must fail");
        assert!(err.contains("too deep"), "{err}");
    }
}
