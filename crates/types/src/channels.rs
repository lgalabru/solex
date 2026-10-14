//! The embeddable `solex` section and its resolution rules.

use std::{collections::BTreeSet, path::PathBuf};

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Error, Result};

/// One or more clusters a channel deploys to, written either as
/// `cluster = "devnet"` or `cluster = ["mainnet-beta", "testnet"]`.
///
/// Several clusters mean the same artifact and program id are deployed to
/// each of them; `solex deploy` iterates over them unless `--cluster` picks one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clusters(pub Vec<String>);

impl Clusters {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    /// The only cluster, when exactly one is declared.
    pub fn single(&self) -> Option<&str> {
        match self.0.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }

    /// Validate a `--cluster` choice against the declared list. Any value is
    /// accepted when nothing is declared (the manifest leaves it to the CLI).
    pub fn select(&self, requested: Option<&str>) -> Result<Vec<String>> {
        match requested {
            Some(r) => {
                if self.0.is_empty() || self.0.iter().any(|c| c == r) {
                    Ok(vec![r.to_string()])
                } else {
                    Err(Error::UnknownCluster {
                        cluster: r.to_string(),
                        declared: self.0.clone(),
                    })
                }
            }
            None => Ok(self.0.clone()),
        }
    }
}

impl From<&str> for Clusters {
    fn from(s: &str) -> Self {
        Clusters(vec![s.to_string()])
    }
}

impl Serialize for Clusters {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self.0.as_slice() {
            [one] => s.serialize_str(one),
            many => many.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for Clusters {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OneOrMany {
            One(String),
            Many(Vec<String>),
        }
        Ok(match OneOrMany::deserialize(d)? {
            OneOrMany::One(s) => Clusters(vec![s]),
            OneOrMany::Many(v) => Clusters(v),
        })
    }
}

/// One release channel: a deployment target plus the set of feature tags it
/// ships.
///
/// A channel with no `program_id` is *tag-only*: it names a set of features
/// for other channels to `include` and is never built or deployed. Use one
/// when the same surface ships to several deployments that differ in
/// `program_id` or `vars`:
///
/// ```toml
/// [channels.stable]            # tag only
/// [channels.mainnet]
/// includes = ["stable"]
/// program_id = "..."
/// vars = { treasury = "..." }
/// [channels.testnet]
/// includes = ["stable"]
/// program_id = "..."
/// vars = { treasury = "..." }
/// ```
/// Fields a cluster may override within a channel, written as
/// `[channels.<channel>.<cluster>]`. Anything not set here is inherited.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClusterOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keypair: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upgrade_authority: Option<PathBuf>,
    /// Merged over the channel's `vars`.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub vars: IndexMap<String, String>,
}

/// A channel's effective settings for one cluster: the channel's values with
/// that cluster's override applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<'a> {
    pub channel: &'a str,
    pub cluster: Option<&'a str>,
    pub program_id: Option<&'a str>,
    pub keypair: Option<&'a PathBuf>,
    pub upgrade_authority: Option<&'a PathBuf>,
    pub vars: IndexMap<&'a str, &'a str>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Channel {
    /// Address the program is deployed at on this channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_id: Option<String>,
    /// Keypair file for `program_id`; required for the first deploy only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keypair: Option<PathBuf>,
    /// Cluster moniker(s) (`mainnet-beta`, `devnet`, `testnet`, `localnet`) or
    /// RPC URL(s) passed to the Solana CLI. A string or a list.
    #[serde(default, skip_serializing_if = "Clusters::is_empty")]
    pub cluster: Clusters,
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
    /// Per-channel constants read by `solex::var!("name")` at compile time:
    /// treasury owners, fee accounts, admin keys, anything that differs per
    /// deployment. Missing a var on the active channel is a compile error, so
    /// placeholders cannot leak into a real deployment.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub vars: IndexMap<String, String>,
    /// Per-cluster overrides, keyed by a name from `cluster`:
    /// `[channels.stable.devnet] vars = { ... }`. Any other key here is a
    /// manifest error, which is what catches a misspelled field.
    #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
    pub overrides: IndexMap<String, ClusterOverride>,
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
            for key in channel.overrides.keys() {
                if !channel.cluster.0.iter().any(|c| c == key) {
                    return Err(Error::UnknownOverride {
                        channel: name.clone(),
                        key: key.clone(),
                        declared: channel.cluster.0.clone(),
                    });
                }
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

    /// Channels with a `program_id`, the ones `--all` builds and deploys.
    pub fn deployable_names(&self) -> Vec<String> {
        self.channels
            .iter()
            .filter(|(_, c)| c.is_deployable())
            .map(|(n, _)| n.clone())
            .collect()
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

    /// Every tag a `#[channel(...)]` attribute may use: the channel names.
    pub fn known_tags(&self) -> BTreeSet<String> {
        self.channels.keys().cloned().collect()
    }

    /// The full set of tags a channel ships: its name and the names of
    /// everything it includes, transitively.
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

impl Channel {
    /// Whether this channel is something to build and deploy, as opposed to a
    /// tag-only channel that exists for others to include.
    pub fn is_deployable(&self) -> bool {
        self.program_id.is_some() || self.overrides.values().any(|o| o.program_id.is_some())
    }

    /// Whether any cluster changes something the compiler sees, in which case
    /// each cluster is its own build.
    pub fn has_overrides(&self) -> bool {
        !self.overrides.is_empty()
    }

    /// Pick the cluster a build or deploy targets. Explicit choice, else
    /// `SOLEX_CLUSTER`, else the only cluster, else none when the channel has
    /// no overrides (every cluster gets the same bytes). Several clusters with
    /// overrides and no choice is an error: the artifact would be ambiguous.
    pub fn resolve_cluster(&self, name: &str, requested: Option<&str>) -> Result<Option<String>> {
        if let Some(r) = requested {
            return Ok(Some(self.cluster.select(Some(r))?.remove(0)));
        }
        if let Ok(env) = std::env::var(crate::CLUSTER_ENV) {
            let env = env.trim();
            if !env.is_empty() {
                return Ok(Some(self.cluster.select(Some(env))?.remove(0)));
            }
        }
        if let Some(one) = self.cluster.single() {
            return Ok(Some(one.to_string()));
        }
        if self.cluster.is_empty() || !self.has_overrides() {
            return Ok(None);
        }
        Err(Error::ClusterRequired {
            channel: name.to_string(),
            declared: self.cluster.0.clone(),
        })
    }

    /// Effective settings for `cluster` (or the channel's own when `None`).
    pub fn resolved<'a>(&'a self, name: &'a str, cluster: Option<&'a str>) -> Resolved<'a> {
        let over = cluster.and_then(|c| self.overrides.get(c));
        let mut vars: IndexMap<&str, &str> = self
            .vars
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if let Some(o) = over {
            for (k, v) in &o.vars {
                vars.insert(k.as_str(), v.as_str());
            }
        }
        Resolved {
            channel: name,
            cluster,
            program_id: over
                .and_then(|o| o.program_id.as_deref())
                .or(self.program_id.as_deref()),
            keypair: over
                .and_then(|o| o.keypair.as_ref())
                .or(self.keypair.as_ref()),
            upgrade_authority: over
                .and_then(|o| o.upgrade_authority.as_ref())
                .or(self.upgrade_authority.as_ref()),
            vars,
        }
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
    fn cluster_overrides_merge_and_validate() {
        let mut c = sample();
        let stable = c.channels.get_mut("stable").unwrap();
        stable.cluster = Clusters(vec!["mainnet-beta".into(), "devnet".into()]);
        stable.vars.insert("treasury".into(), "main".into());
        stable.vars.insert("fee".into(), "shared".into());
        stable.overrides.insert(
            "devnet".into(),
            ClusterOverride {
                program_id: Some("9".into()),
                vars: IndexMap::from([("treasury".to_string(), "dev".to_string())]),
                ..Default::default()
            },
        );
        c.validate().unwrap();
        let ch = c.channel("stable").unwrap();
        let main = ch.resolved("stable", Some("mainnet-beta"));
        assert_eq!(main.program_id, Some("1"));
        assert_eq!(main.vars["treasury"], "main");
        let dev = ch.resolved("stable", Some("devnet"));
        assert_eq!(dev.program_id, Some("9"));
        assert_eq!(dev.vars["treasury"], "dev");
        assert_eq!(dev.vars["fee"], "shared");
        // Two clusters with overrides and no pick is ambiguous.
        if std::env::var(crate::CLUSTER_ENV).is_err() {
            assert!(matches!(
                ch.resolve_cluster("stable", None),
                Err(Error::ClusterRequired { .. })
            ));
        }
        assert_eq!(
            ch.resolve_cluster("stable", Some("devnet"))
                .unwrap()
                .as_deref(),
            Some("devnet")
        );
        // An override for an undeclared cluster is a manifest error.
        c.channels
            .get_mut("stable")
            .unwrap()
            .overrides
            .insert("testnet".into(), ClusterOverride::default());
        assert!(matches!(c.validate(), Err(Error::UnknownOverride { .. })));
    }

    #[test]
    fn tag_only_channels_are_not_deployable() {
        let mut c = sample();
        c.channels.insert("tagonly".into(), Channel::default());
        c.channels
            .get_mut("canary")
            .unwrap()
            .includes
            .push("tagonly".into());
        c.validate().unwrap();
        assert!(!c.channel("tagonly").unwrap().is_deployable());
        assert_eq!(c.deployable_names(), ["stable", "nightly", "canary"]);
        assert!(c.ships("canary", "tagonly").unwrap());
        assert!(matches!(
            c.program_id("tagonly"),
            Err(Error::MissingProgramId(_))
        ));
    }

    #[test]
    fn clusters_select_and_validate() {
        let one = Clusters::from("devnet");
        assert_eq!(one.single(), Some("devnet"));
        assert_eq!(one.select(None).unwrap(), ["devnet"]);
        let many = Clusters(vec!["mainnet-beta".into(), "testnet".into()]);
        assert_eq!(many.single(), None);
        assert_eq!(many.select(None).unwrap(), ["mainnet-beta", "testnet"]);
        assert_eq!(many.select(Some("testnet")).unwrap(), ["testnet"]);
        assert!(matches!(
            many.select(Some("devnet")),
            Err(Error::UnknownCluster { .. })
        ));
        // Nothing declared: the CLI's choice stands.
        assert_eq!(
            Clusters::default()
                .select(Some("http://localhost:8899"))
                .unwrap(),
            ["http://localhost:8899"]
        );
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
