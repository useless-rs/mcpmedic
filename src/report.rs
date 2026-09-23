//! Terminal output helpers. Colors auto-disable for pipes and `NO_COLOR`.

use std::fmt::Write as _;
use std::io::IsTerminal;

use colored::Colorize;

/// Disable colors when `NO_COLOR` is set or stdout is not a terminal.
pub(crate) fn init_colors() {
    if std::env::var_os("NO_COLOR").is_some() || !std::io::stdout().is_terminal() {
        colored::control::set_override(false);
    }
}

/// Truncate a string with an ellipsis if it exceeds `max` characters.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Render fixed-width columns, padding each cell to the widest value in it.
pub(crate) fn render_table(rows: &[Vec<String>]) -> String {
    let mut widths = Vec::new();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if widths.len() <= i {
                widths.push(0);
            }
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            let _ = write!(line, "{cell:<width$}", width = widths[i]);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Green check glyph.
pub(crate) fn glyph_ok() -> String {
    "✓".green().to_string()
}

/// Red cross glyph.
pub(crate) fn glyph_crit() -> String {
    "✗".red().to_string()
}

/// Yellow warning glyph.
pub(crate) fn glyph_warn() -> String {
    "⚠".yellow().to_string()
}

/// Blue info glyph.
pub(crate) fn glyph_info() -> String {
    "ℹ".blue().to_string()
}

/// Neutral "not configured" glyph.
pub(crate) fn glyph_none() -> String {
    "○".normal().to_string()
}

/// Bold section header, e.g. `cursor — ~/.cursor/mcp.json (2 servers)`.
pub(crate) fn header(text: &str) -> String {
    text.bold().to_string()
}

/// Error line for stderr.
pub(crate) fn error_line(text: &str) -> String {
    format!("{} {text}", glyph_crit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_is_identity() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_long_gets_ellipsis() {
        assert_eq!(truncate("abcdefgh", 5), "abcd…");
    }

    #[test]
    fn table_aligns_columns() {
        let rows = vec![
            vec!["NAME".into(), "TRANSPORT".into()],
            vec!["context7".into(), "stdio".into()],
        ];
        let out = render_table(&rows);
        assert!(out.contains("NAME      TRANSPORT"));
        assert!(out.contains("context7  stdio"));
    }
}
