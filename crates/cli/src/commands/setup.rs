//! `solex setup`: install the agent skill and, on request, register the MCP
//! server with Claude Code.

use std::process::{Command, Stdio};

use clap::Args;
use solex_core::{CoreError, Result};

use crate::style;

const SKILL_REPO: &str = "https://github.com/lgalabru/solex";
const SKILL_NAME: &str = "solex";

#[derive(Args, Debug, Default)]
pub struct SetupCommand {
    /// Skip installing the agent skill
    #[arg(long)]
    pub no_skill: bool,

    /// Register `solex mcp` with Claude Code (user scope)
    #[arg(long)]
    pub mcp: bool,
}

impl SetupCommand {
    pub fn execute(&self) -> Result<()> {
        println!();
        if !self.no_skill {
            install_skill()?;
        }
        if self.mcp {
            register_mcp()?;
        } else {
            println!(
                "  {} Register the MCP server with Claude Code via {} or:",
                style::dim("tip"),
                style::bold("solex setup --mcp")
            );
            println!(
                "      {}",
                style::dim("claude mcp add --scope user solex -- solex mcp")
            );
        }
        println!();
        Ok(())
    }
}

fn npx() -> &'static str {
    if cfg!(windows) { "npx.cmd" } else { "npx" }
}

fn has(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `npx skills add` is the installer the skills.sh ecosystem uses; it places
/// the skill where each detected agent (Claude Code, Cursor, Codex, ...) reads it.
fn install_skill() -> Result<()> {
    let manual = format!("npx -y skills add {SKILL_REPO} --skill {SKILL_NAME} -y");
    if !has(npx()) {
        return Err(CoreError::message(format!(
            "npx not found; install Node.js, then run:\n  {manual}"
        )));
    }
    let sp = style::spinner("Installing the solex agent skill...");
    let status = Command::new(npx())
        .args([
            "-y", "skills", "add", SKILL_REPO, "--skill", SKILL_NAME, "-y",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    sp.finish_and_clear();
    match status {
        Ok(out) if out.status.success() => {
            println!("  {} Skill installed for your coding agents", style::ok());
            Ok(())
        }
        Ok(out) => Err(CoreError::message(format!(
            "skill install failed:\n{}\n  Retry with:\n  {manual}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        Err(e) => Err(CoreError::message(format!(
            "failed to run npx: {e}\n  Retry with:\n  {manual}"
        ))),
    }
}

fn register_mcp() -> Result<()> {
    let solex_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "solex".into());
    if !has("claude") {
        println!(
            "  {} `claude` CLI not found. Add this to your MCP client config:",
            style::warn()
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "mcpServers": { "solex": { "command": solex_bin, "args": ["mcp"] } }
            }))
            .unwrap_or_default()
        );
        return Ok(());
    }
    let status = Command::new("claude")
        .args([
            "mcp", "add", "--scope", "user", "solex", "--", &solex_bin, "mcp",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match status {
        Ok(out) if out.status.success() => {
            println!("  {} MCP server registered with Claude Code", style::ok());
            Ok(())
        }
        Ok(out) => {
            let msg = String::from_utf8_lossy(&out.stderr);
            if msg.contains("already exists") {
                println!("  {} MCP server already registered", style::ok());
                Ok(())
            } else {
                Err(CoreError::message(format!(
                    "claude mcp add failed:\n{}",
                    msg.trim()
                )))
            }
        }
        Err(e) => Err(CoreError::message(format!("failed to run claude: {e}"))),
    }
}
