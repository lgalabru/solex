//! Step lines for command progress.
//!
//! ```text
//!   ✓ Created demo  ./demo
//!     keys/stable.json
//! ```

use crate::style;

/// A completed step.
pub fn ok(msg: &str) -> String {
    format!("  {} {msg}", style::ok())
}

/// A dimmed hint under a step.
pub fn tip(msg: &str) -> String {
    format!("    {}", style::dim(msg))
}
