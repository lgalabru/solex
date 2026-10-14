use std::path::PathBuf;

use clap::{Args, ValueEnum};
use solex_core::{Result, build};

use crate::commands;

/// Print the environment a build needs so another pipeline can select the
/// channel: `eval "$(solex env -c nightly)"` then `cargo +nightly build-bpf`.
#[derive(Args, Debug, Default)]
pub struct EnvCommand {
    /// Channel (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Cluster within the channel (needed when it has per-cluster overrides)
    #[arg(long)]
    pub cluster: Option<String>,

    /// Output format
    #[arg(long, value_enum, default_value_t = EnvFormat::Shell)]
    pub format: EnvFormat,

    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

#[derive(ValueEnum, Clone, Copy, Debug, Default)]
pub enum EnvFormat {
    /// `export KEY=value` lines
    #[default]
    Shell,
    /// `KEY=value` lines for .env files, Docker `--env-file`, CI
    Dotenv,
    /// JSON object
    Json,
}

impl EnvCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channel = project.channels().resolve(self.channel.as_deref())?;
        let cluster = project
            .channels()
            .channel(&channel)?
            .resolve_cluster(&channel, self.cluster.as_deref())?;
        let env = build::target_env(&project, &channel, cluster.as_deref());
        match self.format {
            EnvFormat::Shell => {
                for (k, v) in env {
                    println!("export {k}={}", shell_quote(&v));
                }
            }
            EnvFormat::Dotenv => {
                for (k, v) in env {
                    println!("{k}={v}");
                }
            }
            EnvFormat::Json => {
                let map: serde_json::Map<String, serde_json::Value> = env
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&map).unwrap_or_default());
            }
        }
        Ok(())
    }
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}
