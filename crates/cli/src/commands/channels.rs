use std::path::PathBuf;

use clap::Args;
use solex_core::Result;

use crate::{commands, components, components::boxed, style};

#[derive(Args, Debug, Default)]
pub struct ChannelsCommand {
    /// Emit JSON instead of a table
    #[arg(long)]
    pub json: bool,

    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl ChannelsCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let m = project.channels();
        let active = m.resolve(None).ok();
        let features_on =
            |channel: &str| -> Vec<String> { m.features_for(channel).unwrap_or_default() };

        if self.json {
            let mut out = serde_json::Map::new();
            for (name, ch) in &m.channels {
                let tags: Vec<_> = m.tags(name)?.into_iter().collect();
                out.insert(
                    name.clone(),
                    serde_json::json!({
                        "program_id": ch.program_id,
                        "deployable": ch.is_deployable(),
                        "cluster": ch.cluster,
                        "includes": ch.includes,
                        "vars": ch.vars,
                        "overrides": ch.overrides,
                        "ships": tags,
                        "features": features_on(name),
                        "default": active.as_deref() == Some(name.as_str()),
                    }),
                );
            }
            println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
            return Ok(());
        }

        let mut sections = Vec::new();
        for (name, ch) in &m.channels {
            let mut lines: Vec<boxed::Line> = Vec::new();
            let mut head = vec![(
                style::bold(&style::channel_color(name, name)),
                name.chars().count(),
            )];
            if !ch.cluster.is_empty() {
                head.push(boxed::plain(ch.cluster.0.join(", ")));
            }
            if !ch.is_deployable() {
                head.push((style::dim("tag only"), 8));
            }
            let right = if active.as_deref() == Some(name.as_str()) {
                (style::accent("default"), 7)
            } else {
                boxed::plain("")
            };
            lines.push(boxed::split(boxed::join(&head), right));
            if let Some(d) = &ch.description {
                for seg in boxed::wrap(d, boxed::INNER) {
                    let w = seg.chars().count();
                    lines.push((style::dim(&seg), w));
                }
            }
            if let Some(id) = &ch.program_id {
                let mut parts = vec![boxed::plain(components::short(id))];
                for (cluster, o) in &ch.overrides {
                    if let Some(oid) = &o.program_id {
                        parts.push(boxed::plain(format!(
                            "{cluster}: {}",
                            components::short(oid)
                        )));
                    }
                }
                lines.push(boxed::field("program", boxed::join(&parts)));
            }
            if !ch.includes.is_empty() {
                lines.push(boxed::field(
                    "includes",
                    boxed::plain(ch.includes.join(", ")),
                ));
            }
            let feats = features_on(name);
            if !feats.is_empty() {
                let parts: Vec<boxed::Line> = feats
                    .iter()
                    .map(|f| match m.activation_epoch(f, name).ok().flatten() {
                        Some(e) => boxed::plain(format!("{f} (from epoch {e})")),
                        None => boxed::plain(f.clone()),
                    })
                    .collect();
                lines.push(boxed::field("features", boxed::join(&parts)));
            }
            if !ch.vars.is_empty() || ch.overrides.values().any(|o| !o.vars.is_empty()) {
                let mut parts: Vec<boxed::Line> = ch
                    .vars
                    .iter()
                    .map(|(k, v)| boxed::plain(format!("{k} = {}", components::short(v))))
                    .collect();
                for (cluster, o) in &ch.overrides {
                    for (k, v) in &o.vars {
                        parts.push(boxed::plain(format!(
                            "{k} = {} ({cluster})",
                            components::short(v)
                        )));
                    }
                }
                lines.push(boxed::field("vars", boxed::join(&parts)));
            }
            sections.push(lines);
        }
        println!("{}", boxed::frame(&sections));
        Ok(())
    }
}
