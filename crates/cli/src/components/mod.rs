//! Reusable display components for the CLI, in the shape of pay's: a bordered
//! box for listings and results, a railed notice for outcomes, info, warnings and
//! errors, hyperlinks, and channel labels. Commands compose these instead
//! of formatting output by hand.

pub mod boxed;
pub mod card;
pub mod label;
pub mod link;
pub mod notice;
pub(crate) mod terminal;

pub use card::ChannelCard;
pub use label::{channel, rel, short};
pub use link::{click_hint, explorer_link};
pub use notice::{NoticeLevel, print_notice, print_notice_lines};
