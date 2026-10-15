//! `solex setup`: install the agent skill and, on request, register the MCP
//! server with Claude Code.

use std::process::{Command, Stdio};

use clap::Args;
use solex_core::{CoreError, Result};

use crate::{components, style};

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
        if !self.no_skill {
            install_skill()?;
        }
        if self.mcp {
            register_mcp()?;
        } else if !mcp_registered() {
            components::print_notice(
                components::NoticeLevel::Warning,
                "MCP server not registered",
                "solex setup --mcp registers it with Claude Code, or add it yourself:\n\
                 claude mcp add --scope user solex -- solex mcp",
            );
        }
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
/// the skill where each detected agent (Claude Code, Cursor, Codex, ...) reads
/// it. `-g` installs user-level: `solex setup` is run from inside a program
/// repo and must not drop `.agents/` and `.claude/` directories into it.
fn install_skill() -> Result<()> {
    let manual = format!("npx -y skills add {SKILL_REPO} --skill {SKILL_NAME} -g -y");
    if !has(npx()) {
        return Err(CoreError::message(format!(
            "npx not found; install Node.js, then run:\n  {manual}"
        )));
    }
    let sp = style::spinner("Installing the solex agent skill...");
    let status = Command::new(npx())
        .args([
            "-y", "skills", "add", SKILL_REPO, "--skill", SKILL_NAME, "-g", "-y",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    sp.finish_and_clear();
    match status {
        Ok(out) if out.status.success() => {
            components::print_notice(
                components::NoticeLevel::Success,
                "Skill installed",
                "User-level, for every project; your coding agents (Claude Code, Cursor, Codex, ...)\n\
                 now know solex. Re-run `solex setup` after upgrading to refresh it.",
            );
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

/// Whether Claude Code already knows a `solex` MCP server. Without the
/// `claude` CLI there is nothing to check, so the hint is shown.
fn mcp_registered() -> bool {
    Command::new("claude")
        .args(["mcp", "get", "solex"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn register_mcp() -> Result<()> {
    let solex_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "solex".into());
    if !has("claude") {
        components::print_notice(
            components::NoticeLevel::Warning,
            "`claude` CLI not found; add this to your MCP client config",
            &serde_json::to_string_pretty(&serde_json::json!({
                "mcpServers": { "solex": { "command": solex_bin, "args": ["mcp"] } }
            }))
            .unwrap_or_default(),
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
            components::print_notice(
                components::NoticeLevel::Success,
                "MCP server registered with Claude Code",
                &format!(
                    "{solex_bin} mcp (user scope)\nclaude mcp list shows it; tools: channels, features, build, deploy_plan"
                ),
            );
            Ok(())
        }
        Ok(out) => {
            let msg = String::from_utf8_lossy(&out.stderr);
            if msg.contains("already exists") {
                components::print_notice(
                    components::NoticeLevel::Success,
                    "MCP server already registered",
                    "claude mcp remove solex, then solex setup --mcp, to re-register.",
                );
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
