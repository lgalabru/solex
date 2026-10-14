//! The embeddable `solex` section and its resolution rules.

use std::{collections::BTreeSet, path::PathBuf};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// One release channel: a deployment target plus the set of feature tags it
/// ships.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    /// Address the program is deployed at on this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    /// Keypair file for `program_id`; required for the first deploy only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keypair: Option<PathBuf>,
    /// Cluster moniker (`mainnet-beta`, `devnet`, `testnet`, `localnet`) or
    /// RPC URL passed to the Solana CLI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,
    /// Upgrade authority keypair passed to `solana program deploy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_authority: Option<PathBuf>,
    /// Other channels whose tags this channel also ships. A `nightly` channel
    /// that `includes: [stable]` compiles every `#[channel(stable)]` item in
    /// addition to `#[channel(nightly)]` ones.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub includes: Vec<String>,
    /// Free-form description shown by `solex channels`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The `solex` configuration section: the only part a host manifest needs.
///
/// ```toml
/// # Anchor.toml
/// [solex]
/// default_channel = "stable"
///
/// [solex.channels.stable]
/// program_id = "..."
/// cluster = "mainnet-beta"
///
/// [solex.channels.nightly]
/// program_id = "..."
/// cluster = "devnet"
/// includes = ["stable"]
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Channels {
    /// Channel used when `SOLEX_CHANNEL` is unset and `--channel` is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_channel: Option<String>,
    /// Channels keyed by name, in declaration order (the order `solex channels`
    /// and `solex features` display).
    #[serde(default)]
    pub channels: IndexMap<String, Channel>,
}

impl Channels {
    /// Structural checks that do not need the environment: non-empty, valid
    /// names, includes resolve, no include cycles, default exists.
    pub fn validate(&self) -> Result<()> {
        if self.channels.is_empty() {
            return Err(Error::NoChannels);
        }
        for (name, channel) in &self.channels {
            if !is_valid_channel_name(name) {
                return Err(Error::InvalidChannelName(name.clone()));
            }
            for inc in &channel.includes {
                if !self.channels.contains_key(inc) {
                    return Err(Error::UnknownInclude {
                        channel: name.clone(),
                        included: inc.clone(),
                    });
                }
            }
        }
        if let Some(def) = &self.default_channel {
            self.channel(def)?;
        }
        for name in self.channels.keys() {
            // `tags` walks the include graph and reports cycles.
            self.tags(name)?;
        }
        Ok(())
    }

    pub fn names(&self) -> Vec<String> {
        self.channels.keys().cloned().collect()
    }

    pub fn channel(&self, name: &str) -> Result<&Channel> {
        self.channels
            .get(name)
            .ok_or_else(|| Error::UnknownChannel {
                name: name.to_string(),
                defined: self.names(),
            })
    }

    /// Program id for a channel, as a base58 string.
    pub fn program_id(&self, name: &str) -> Result<&str> {
        self.channel(name)?
            .program_id
            .as_deref()
            .ok_or_else(|| Error::MissingProgramId(name.to_string()))
    }

    /// The full set of tags a channel ships: itself plus everything it
    /// includes, transitively.
    pub fn tags(&self, name: &str) -> Result<BTreeSet<String>> {
        let mut tags = BTreeSet::new();
        let mut stack = vec![name.to_string()];
        self.collect_tags(name, &mut tags, &mut stack)?;
        Ok(tags)
    }

    fn collect_tags(
        &self,
        name: &str,
        tags: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
    ) -> Result<()> {
        let channel = self.channel(name)?;
        tags.insert(name.to_string());
        for inc in &channel.includes {
            if stack.iter().any(|s| s == inc) {
                let mut cycle = stack.clone();
                cycle.push(inc.clone());
                return Err(Error::IncludeCycle { path: cycle });
            }
            if tags.contains(inc) {
                continue;
            }
            stack.push(inc.clone());
            self.collect_tags(inc, tags, stack)?;
            stack.pop();
        }
        Ok(())
    }

    /// Whether a build of `channel` ships items tagged `tag`.
    pub fn ships(&self, channel: &str, tag: &str) -> Result<bool> {
        Ok(self.tags(channel)?.contains(tag))
    }

    /// Resolve the channel to build: an explicit request, else `SOLEX_CHANNEL`,
    /// else `default_channel`, else the only channel defined.
    pub fn resolve(&self, requested: Option<&str>) -> Result<String> {
        if let Some(name) = requested {
            self.channel(name)?;
            return Ok(name.to_string());
        }
        if let Ok(name) = std::env::var(crate::CHANNEL_ENV) {
            let name = name.trim();
            if !name.is_empty() {
                self.channel(name)?;
                return Ok(name.to_string());
            }
        }
        if let Some(def) = &self.default_channel {
            return Ok(def.clone());
        }
        if self.channels.len() == 1 {
            return Ok(self.channels.keys().next().cloned().unwrap_or_default());
        }
        Err(Error::NoDefault {
            defined: self.names(),
        })
    }
}

pub fn is_valid_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Channels {
        let mut channels = IndexMap::new();
        channels.insert(
            "stable".into(),
            Channel {
                program_id: Some("1".into()),
                ..Default::default()
            },
        );
        channels.insert(
            "nightly".into(),
            Channel {
                program_id: Some("2".into()),
                includes: vec!["stable".into()],
                ..Default::default()
            },
        );
        channels.insert(
            "canary".into(),
            Channel {
                program_id: Some("3".into()),
                includes: vec!["nightly".into()],
                ..Default::default()
            },
        );
        let c = Channels {
            default_channel: Some("stable".into()),
            channels,
        };
        c.validate().unwrap();
        c
    }

    #[test]
    fn tags_follow_includes_transitively() {
        let c = sample();
        let v = |n: &str| c.tags(n).unwrap().into_iter().collect::<Vec<_>>();
        assert_eq!(v("stable"), ["stable"]);
        assert_eq!(v("nightly"), ["nightly", "stable"]);
        assert_eq!(v("canary"), ["canary", "nightly", "stable"]);
    }

    #[test]
    fn ships_is_directional() {
        let c = sample();
        assert!(c.ships("nightly", "stable").unwrap());
        assert!(!c.ships("stable", "nightly").unwrap());
    }

    #[test]
    fn order_follows_declaration() {
        assert_eq!(sample().names(), ["stable", "nightly", "canary"]);
    }

    #[test]
    fn cycles_unknown_includes_and_bad_defaults_are_rejected() {
        let mut c = sample();
        c.channels.get_mut("stable").unwrap().includes = vec!["canary".into()];
        assert!(matches!(c.validate(), Err(Error::IncludeCycle { .. })));

        let mut c = sample();
        c.channels.get_mut("stable").unwrap().includes = vec!["ghost".into()];
        assert!(matches!(c.validate(), Err(Error::UnknownInclude { .. })));

        let mut c = sample();
        c.default_channel = Some("ghost".into());
        assert!(matches!(c.validate(), Err(Error::UnknownChannel { .. })));

        let mut c = sample();
        c.channels.insert("Bad".into(), Channel::default());
        assert!(matches!(c.validate(), Err(Error::InvalidChannelName(_))));
    }

    #[test]
    fn resolve_prefers_explicit_then_default() {
        let c = sample();
        assert_eq!(c.resolve(Some("nightly")).unwrap(), "nightly");
        assert!(c.resolve(Some("ghost")).is_err());
        if std::env::var(crate::CHANNEL_ENV).is_err() {
            assert_eq!(c.resolve(None).unwrap(), "stable");
            let mut only = sample();
            only.default_channel = None;
            only.channels.shift_remove("nightly");
            only.channels.shift_remove("canary");
            assert_eq!(only.resolve(None).unwrap(), "stable");
        }
    }
}
