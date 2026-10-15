//! The channel card: the one section `build`, `deploy`, `terminate` and
//! `init` all print for a target, so a channel looks the same everywhere.
//!
//! ```text
//!   ┌──────────────────────────────────────────────────────────────┐
//!   │ canary  devnet                              98.9 KiB  2.3s │
//!   │ program   CHNL…sWkJ                                        │
//!   │ features  hooks                                            │
//!   │ artifact  target/solex/canary/payment_channels.so          │
//!   └──────────────────────────────────────────────────────────────┘
//! ```

use std::path::Path;

use super::boxed::{self, Line};
use crate::{components, style};

pub struct ChannelCard {
    head: Vec<Line>,
    right: Line,
    fields: Vec<(String, Line)>,
    channel: String,
}

impl ChannelCard {
    /// Header: the colored channel label, then the cluster when known.
    pub fn new(channel: &str, cluster: Option<&str>) -> Self {
        let mut head = vec![boxed::styled(
            style::bold(&components::channel(channel)),
            channel,
        )];
        if let Some(c) = cluster {
            head.push(boxed::plain(c));
        }
        Self {
            head,
            right: boxed::plain(""),
            fields: Vec::new(),
            channel: channel.to_string(),
        }
    }

    /// Dimmed text on the far right of the header (size, duration, "default").
    pub fn right(mut self, text: &str) -> Self {
        self.right = boxed::styled(style::dim(text), text);
        self
    }

    /// `program` as a middle-truncated pubkey.
    pub fn program(self, id: &str) -> Self {
        let short = components::short(id);
        self.field("program", boxed::plain(short))
    }

    /// `features`, colored like the channel; omitted when there are none.
    pub fn features(self, features: &[String]) -> Self {
        let styled = features
            .iter()
            .map(|f| style::channel_color(&self.channel, f))
            .collect::<Vec<_>>()
            .join("  ");
        let plain = features.join("  ");
        self.field("features", boxed::styled(styled, &plain))
    }

    /// `artifact` as a path relative to the project root.
    pub fn artifact(self, root: &Path, path: &Path) -> Self {
        let rel = components::rel(root, path);
        self.field("artifact", boxed::plain(rel))
    }

    /// Any other labelled field; empty values are skipped by the box.
    pub fn text(self, label: &str, value: impl Into<String>) -> Self {
        self.field(label, boxed::plain(value.into()))
    }

    fn field(mut self, label: &str, value: Line) -> Self {
        self.fields.push((label.to_string(), value));
        self
    }

    /// The card as one section, for stacking several in a frame.
    pub fn section(&self) -> Vec<Line> {
        let fields: Vec<(&str, Line)> = self
            .fields
            .iter()
            .map(|(l, v)| (l.as_str(), v.clone()))
            .collect();
        boxed::section(&self.head, self.right.clone(), &fields)
    }

    /// The card framed on its own.
    pub fn frame(&self) -> String {
        boxed::frame(&[self.section()])
    }
}
