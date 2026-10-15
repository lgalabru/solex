//! `solex setup`: install the agent skill and register the MCP server with
//! Claude Code and Codex; `--no-skill` / `--no-mcp` skip either half.

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

    /// Skip registering `solex mcp` with Claude Code and Codex (user scope)
    #[arg(long)]
    pub no_mcp: bool,
}

impl SetupCommand {
    pub fn execute(&self) -> Result<()> {
        if !self.no_skill {
            install_skill()?;
        }
        if !self.no_mcp {
            register_mcp()?;
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

/// An agent CLI that manages MCP servers with `<cli> mcp add|get`.
struct Agent {
    label: &'static str,
    bin: &'static str,
    /// Arguments placed between `mcp add` and the server name.
    add_flags: &'static [&'static str],
}

const AGENTS: &[Agent] = &[
    Agent {
        label: "Claude Code",
        bin: "claude",
        add_flags: &["--scope", "user"],
    },
    Agent {
        label: "Codex",
        bin: "codex",
        add_flags: &[],
    },
];

fn registered_with(agent: &Agent) -> bool {
    Command::new(agent.bin)
        .args(["mcp", "get", "solex"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Register `solex mcp` with every agent CLI on PATH (Claude Code, Codex),
/// skipping the ones that already have it.
fn register_mcp() -> Result<()> {
    let solex_bin = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "solex".into());
    let tools = format!("tools    {}", solex_mcp::tool_names().join(", "));

    let found: Vec<&Agent> = AGENTS.iter().filter(|a| has(a.bin)).collect();
    if found.is_empty() {
        components::print_notice(
            components::NoticeLevel::Warning,
            "No agent CLI found (claude, codex); add the MCP server to your client yourself",
            &format!(
                "{}\n{tools}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": { "solex": { "command": solex_bin, "args": ["mcp"] } }
                }))
                .unwrap_or_default()
            ),
        );
        return Ok(());
    }

    let mut done = Vec::new();
    for agent in found {
        if registered_with(agent) {
            done.push(agent.label);
            continue;
        }
        let out = Command::new(agent.bin)
            .args(["mcp", "add"])
            .args(agent.add_flags)
            .args(["solex", "--", &solex_bin, "mcp"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| CoreError::message(format!("failed to run {}: {e}", agent.bin)))?;
        let msg = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() && !msg.contains("already exists") {
            return Err(CoreError::message(format!(
                "{} mcp add failed:\n{}",
                agent.bin,
                msg.trim()
            )));
        }
        done.push(agent.label);
    }
    components::print_notice(
        components::NoticeLevel::Success,
        "MCP server registered",
        &format!("harness  {}\n{tools}", done.join(", ")),
    );
    Ok(())
}
