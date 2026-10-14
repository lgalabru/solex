use std::path::PathBuf;

use clap::Args;
use solex_core::Result;

use crate::{commands, style};

#[derive(Args, Debug, Default)]
pub struct ChannelsCommand {
    /// Emit JSON instead of a table
    #[arg(long)]
    pub json: bool,

    /// Path to Solex.yaml
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl ChannelsCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let m = project.manifest();
        let active = m.resolve_channel(None).ok();

        if self.json {
            let mut out = serde_json::Map::new();
            for (name, ch) in &m.channels {
                let tags: Vec<_> = m.tags(name)?.into_iter().collect();
                out.insert(
                    name.clone(),
                    serde_json::json!({
                        "program_id": ch.program_id,
                        "cluster": ch.cluster,
                        "includes": ch.includes,
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
            style::bold(&m.name),
            style::dim(&project.manifest_path().display().to_string())
        );
        println!();
        for (name, ch) in &m.channels {
            let marker = if active.as_deref() == Some(name.as_str()) {
                style::green("●")
            } else {
                style::dim("○")
            };
            println!("  {marker} {}", style::accent(name));
            println!(
                "      {:<10} {}",
                style::dim("program"),
                ch.program_id.as_deref().unwrap_or("(unset)")
            );
            println!(
                "      {:<10} {}",
                style::dim("cluster"),
                ch.cluster.as_deref().unwrap_or("(solana CLI default)")
            );
            let tags: Vec<_> = m.tags(name)?.into_iter().collect();
            println!("      {:<10} {}", style::dim("ships"), tags.join(", "));
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
