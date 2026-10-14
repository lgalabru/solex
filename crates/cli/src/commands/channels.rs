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
        // Named features per channel, from the source scan. A project without
        // sources yet (or with unparsable ones) simply shows none.
        let items = solex_core::scan::scan_program(&project.program_dir()).unwrap_or_default();
        let features_on = |channel: &str| -> Vec<String> {
            let mut out: Vec<String> = Vec::new();
            for item in &items {
                let Some(f) = &item.feature else { continue };
                if out.contains(f) {
                    continue;
                }
                if matches!(
                    item.decision(m, channel),
                    Ok(solex_core::types::spec::Decision::Include { .. })
                ) {
                    out.push(f.clone());
                }
            }
            out
        };

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

        println!();
        println!(
            "  {} {}",
            style::bold(&project.name()),
            style::dim(&project.manifest_path().display().to_string())
        );
        println!();
        for (name, ch) in &m.channels {
            let marker = if active.as_deref() == Some(name.as_str()) {
                style::green("●")
            } else {
                style::dim("○")
            };
            if !ch.is_deployable() {
                println!(
                    "  {marker} {} {}",
                    style::accent(name),
                    style::dim("(tag only: included by other channels, never built)")
                );
                if let Some(d) = &ch.description {
                    println!("      {:<10} {}", style::dim("about"), d);
                }
                continue;
            } else {
                println!("  {marker} {}", style::accent(name));
                println!(
                    "      {:<10} {}",
                    style::dim("program"),
                    ch.program_id.as_deref().unwrap_or("(unset)")
                );
            }
            println!(
                "      {:<10} {}",
                style::dim("cluster"),
                if ch.cluster.is_empty() {
                    "(solana CLI default)".to_string()
                } else {
                    ch.cluster.0.join(", ")
                }
            );
            let tags: Vec<_> = m.tags(name)?.into_iter().collect();
            println!("      {:<10} {}", style::dim("ships"), tags.join(", "));
            let feats = features_on(name);
            if !feats.is_empty() {
                println!(
                    "      {:<10} {}",
                    style::dim("features"),
                    feats
                        .iter()
                        .map(|f| style::bold(f))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            for (k, v) in &ch.vars {
                println!("      {:<10} {k} = {v}", style::dim("var"));
            }
            for (cluster, o) in &ch.overrides {
                if let Some(id) = &o.program_id {
                    println!(
                        "      {:<10} {cluster}: program {id}",
                        style::dim("override")
                    );
                }
                for (k, v) in &o.vars {
                    println!("      {:<10} {cluster}: {k} = {v}", style::dim("override"));
                }
            }
            if let Some(d) = &ch.description {
                println!("      {:<10} {}", style::dim("about"), d);
            }
        }
        println!();
        if let Some(a) = active {
            println!("  {} default channel: {a}", style::dim("●"));
            println!();
        }
        Ok(())
    }
}
