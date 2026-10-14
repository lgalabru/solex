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
                        "ships": tags,
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
            for (k, v) in &ch.vars {
                println!("      {:<10} {k} = {v}", style::dim("var"));
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
