use std::path::PathBuf;

use clap::Args;
use solex_core::Result;

use crate::{commands, style};

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

        let mut rows = Vec::new();
        for (name, ch) in &m.channels {
            let label = if active.as_deref() == Some(name.as_str()) {
                format!("{name} *")
            } else {
                name.clone()
            };
            let program = if ch.is_deployable() {
                let mut ids: Vec<String> = Vec::new();
                if let Some(id) = &ch.program_id {
                    ids.push(id.clone());
                }
                for (cluster, o) in &ch.overrides {
                    if let Some(id) = &o.program_id {
                        ids.push(format!("{id} ({cluster})"));
                    }
                }
                ids.join(", ")
            } else {
                "(tag only)".into()
            };
            let mut vars: Vec<String> = ch.vars.iter().map(|(k, v)| format!("{k}={v}")).collect();
            for (cluster, o) in &ch.overrides {
                vars.extend(o.vars.iter().map(|(k, v)| format!("{k}={v} ({cluster})")));
            }
            let feats = features_on(name)
                .iter()
                .map(|f| match m.activation_epoch(f, name).ok().flatten() {
                    Some(e) => format!("{f}@{e}"),
                    None => f.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            rows.push(vec![
                label,
                program,
                ch.cluster.0.join(", "),
                ch.includes.join(", "),
                feats,
                vars.join(", "),
            ]);
        }
        print!(
            "{}",
            style::table(
                &[
                    "channel", "program", "cluster", "includes", "features", "vars"
                ],
                &rows,
                &[style::Align::Left; 6],
            )
        );
        Ok(())
    }
}
