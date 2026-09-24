//! JSONC tolerance: strip `//` / `/* */` comments and trailing commas so a
//! JSONC file (opencode, VS Code) can be parsed read-only. String literals
//! are preserved exactly.
//!
//! Split out of [`crate::format`].

/// Strip `//`, `/* */` comments and trailing commas so a JSONC file can be
/// parsed read-only. String literals are preserved exactly.
pub(crate) fn jsonc_strip(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = next {
                    out.push(n);
                    i += 2;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
            }
            ',' => {
                // Drop the comma if only whitespace separates it from `}` or `]`.
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if matches!(chars.get(j), Some('}' | ']')) {
                    i += 1; // skip comma, whitespace is copied naturally below
                } else {
                    out.push(c);
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}
