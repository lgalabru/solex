//! Railed notice: a colored bar, a bold title, a dimmed body.
//!
//! ```text
//! │ Deployed canary
//! │ program  CHNL…sWkJ
//! │ cluster  devnet
//! ```

use std::sync::atomic::{AtomicBool, Ordering};

use crate::style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl NoticeLevel {
    fn rail(self) -> String {
        let code = match self {
            Self::Info => 75,
            Self::Success => 114,
            Self::Warning => 215,
            Self::Error => 167,
        };
        style::bold(&style::color(code, "│"))
    }
}

/// Render a notice. Body lines are sanitized and dimmed; an empty body gives
/// a single title line.
pub fn notice(level: NoticeLevel, title: &str, body: &str) -> String {
    let rail = level.rail();
    let mut out = format!(
        "{rail} {}\n",
        style::bold(&super::terminal::sanitize(title))
    );
    for line in body.lines() {
        out.push_str(&format!(
            "{rail} {}\n",
            style::dim(&super::terminal::sanitize(line))
        ));
    }
    out
}

static PRINTED: AtomicBool = AtomicBool::new(false);

/// Print a notice to stderr, so stdout stays usable for `--json` and pipes.
/// A blank line separates it from any notice printed earlier in the run, so
/// two notices never read as one block.
pub fn print_notice(level: NoticeLevel, title: &str, body: &str) {
    if PRINTED.swap(true, Ordering::Relaxed) {
        eprintln!();
    }
    eprint!("{}", notice(level, title, body));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_and_body_share_the_rail() {
        crate::style::init(false);
        let out = notice(NoticeLevel::Warning, "Nothing sent", "re-run with --yes");
        let plain: String = console::strip_ansi_codes(&out).into();
        assert_eq!(plain, "│ Nothing sent\n│ re-run with --yes\n");
    }
}
