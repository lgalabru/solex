//! Step lines and key-value blocks for command progress and summaries.
//!
//! ```text
//!   ✓ nightly    CHNL…sWkJ  98.9 KiB  1.8s
//!     features   hooks
//!     artifact   target/solex/nightly/payment_channels.so
//! ```

use crate::style;

/// A completed step.
pub fn ok(msg: &str) -> String {
    format!("  {} {msg}", style::ok())
}

/// A failed step.
pub fn fail(msg: &str) -> String {
    format!("  {} {msg}", style::fail())
}

/// A dimmed hint under a step.
pub fn tip(msg: &str) -> String {
    format!("    {}", style::dim(msg))
}

/// Aligned `label  value` lines, indented under a step. Labels are dimmed;
/// values are printed as given (callers style them).
pub fn fields(rows: &[(&str, String)]) -> String {
    let width = rows
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    rows.iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(l, v)| format!("    {}  {v}", style::dim(&format!("{l:<width$}"))))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn fields_align_labels_and_skip_empty_values() {
        crate::style::init(false);
        let out = super::fields(&[
            ("program", "x".into()),
            ("features", String::new()),
            ("cluster", "devnet".into()),
        ]);
        let plain: String = console::strip_ansi_codes(&out).into();
        assert_eq!(plain, "    program   x\n    cluster   devnet");
    }
}
