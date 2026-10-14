//! Reusable display components for the CLI, in the shape of pay's: a bordered
//! box for listings, a railed notice for info/warnings/errors, labelled
//! key-value blocks, step lines, hyperlinks, and channel labels. Commands
//! compose these instead of formatting output by hand.

pub mod boxed;
pub mod label;
pub mod link;
pub mod notice;
pub mod step;
pub(crate) mod terminal;

pub use label::{channel, short};
pub use link::explorer_address_link;
pub use notice::{NoticeLevel, print_notice};
pub use step::{fail, fields, ok, tip};
