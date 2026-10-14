//! MCP server for solex. A thin dispatch layer over `solex-core`: every tool
//! loads the project and serializes a core result.
//!
//! Tools never deploy. `deploy_plan` returns the exact command for the user
//! to run, which keeps the irreversible step in human hands.

use std::path::{Path, PathBuf};

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, Implementation, ProtocolVersion, ServerCapabilities,
        ServerConfig,
    },
    schemars::{self, JsonSchema},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use solex_core::{
    Project,
    build::{self, BuildOptions},
    deploy,
    scan::{self, Cell},
};

const INSTRUCTIONS: &str = "solex manages release channels for Solana programs. Start with \
                            `channels` to learn the channel names, use `features` to see what \
                            each channel's binary contains, `build` to compile one channel, and \
                            `deploy_plan` to get the deploy command for the user to run.";

/// Serve over stdio until the client disconnects.
pub async fn run_server() -> Result<(), String> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let service = SolexMcp::new()
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await
        .map_err(|e| e.to_string())?;
    service
        .waiting()
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub struct SolexMcp {
    tool_router: ToolRouter<Self>,
}

impl Default for SolexMcp {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectParams {
    /// Path to Solex.toml / Solex.yaml / Anchor.toml with a [solex] section.
    /// Defaults to searching upward from the current directory.
    #[serde(default)]
    pub manifest: Option<PathBuf>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FeaturesParams {
    #[serde(default)]
    pub manifest: Option<PathBuf>,
    /// Only this channel's column. Defaults to all channels.
    #[serde(default)]
    pub channel: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BuildParams {
    #[serde(default)]
    pub manifest: Option<PathBuf>,
    /// Channel to build. Defaults to SOLEX_CHANNEL or the manifest default.
    #[serde(default)]
    pub channel: Option<String>,
    /// Return the cargo build-sbf command instead of running it.
    #[serde(default)]
    pub dry_run: bool,
    /// Cluster within the channel, for channels with per-cluster overrides.
    #[serde(default)]
    pub cluster: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeployPlanParams {
    #[serde(default)]
    pub manifest: Option<PathBuf>,
    /// Channel to plan a deploy for.
    #[serde(default)]
    pub channel: Option<String>,
    /// One of the channel's clusters; defaults to all of them.
    #[serde(default)]
    pub cluster: Option<String>,
}

type ToolResult = Result<CallToolResult, ErrorData>;

fn load(manifest: Option<&Path>) -> Result<Project, String> {
    match manifest {
        Some(p) => Project::load(p),
        None => Project::discover_cwd(),
    }
    .map_err(|e| e.to_string())
}

fn json(value: serde_json::Value) -> ToolResult {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        serde_json::to_string_pretty(&value).unwrap_or_default(),
    )]))
}

fn fail(message: impl Into<String>) -> ToolResult {
    Ok(CallToolResult::error(vec![ContentBlock::text(
        message.into(),
    )]))
}

#[tool_router]
impl SolexMcp {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List the project's release channels: program ids, clusters, vars, which other channels each one includes, and the resolved tag set it ships. Call this first to learn the channel names before gating or building."
    )]
    async fn channels(&self, Parameters(p): Parameters<ProjectParams>) -> ToolResult {
        let project = match load(p.manifest.as_deref()) {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        let channels = project.channels();
        let default = channels.resolve(None).ok();
        let mut out = serde_json::Map::new();
        for (name, ch) in &channels.channels {
            let tags: Vec<_> = channels
                .tags(name)
                .map(|t| t.into_iter().collect())
                .unwrap_or_default();
            out.insert(
                name.clone(),
                serde_json::json!({
                    "program_id": ch.program_id,
                    "deployable": ch.is_deployable(),
                    "cluster": ch.cluster,
                    "includes": ch.includes,
                    "vars": ch.vars,
                    "ships": tags,
                    "default": default.as_deref() == Some(name.as_str()),
                }),
            );
        }
        json(serde_json::json!({
            "name": project.name(),
            "manifest": project.manifest_path(),
            "channels": out,
        }))
    }

    #[tool(
        description = "Static matrix of every #[channel(...)] marker in the program sources against each channel: ships, ships from an epoch, or not compiled. Also reports tags that no channel defines. Use it to review what a channel's binary will contain before building."
    )]
    async fn features(&self, Parameters(p): Parameters<FeaturesParams>) -> ToolResult {
        let project = match load(p.manifest.as_deref()) {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        let channels = project.channels();
        let columns = match &p.channel {
            Some(c) => match channels.channel(c) {
                Ok(_) => vec![c.clone()],
                Err(e) => return fail(e.to_string()),
            },
            None => channels.names(),
        };
        let items = match scan::scan_program(&project.program_dir()) {
            Ok(i) => i,
            Err(e) => return fail(e.to_string()),
        };
        let unknown: Vec<_> = scan::unknown_tags(channels, &items)
            .into_iter()
            .map(|(tag, item)| {
                serde_json::json!({
                    "tag": tag, "item": item.name, "file": item.file, "line": item.line
                })
            })
            .collect();
        let matrix = match scan::matrix(channels, items, &columns) {
            Ok(m) => m,
            Err(e) => return fail(e.to_string()),
        };
        let rows: Vec<_> = matrix
            .rows
            .iter()
            .map(|(item, cells)| {
                let per: serde_json::Map<String, serde_json::Value> = columns
                    .iter()
                    .zip(cells)
                    .map(|(ch, cell)| {
                        let v = match cell {
                            Cell::Excluded => serde_json::json!({ "ships": false }),
                            Cell::Included => serde_json::json!({ "ships": true }),
                            Cell::IncludedFrom(e) => {
                                serde_json::json!({ "ships": true, "from_epoch": e })
                            }
                        };
                        (ch.clone(), v)
                    })
                    .collect();
                serde_json::json!({
                    "file": item.file, "line": item.line, "kind": item.kind,
                    "name": item.name, "channel_attr": item.spec_text, "channels": per,
                })
            })
            .collect();
        json(serde_json::json!({ "items": rows, "unknown_tags": unknown }))
    }

    #[tool(
        description = "Build one channel with cargo build-sbf into target/solex/<channel>/. Builds take minutes; set dry_run to get the exact command instead. Returns the artifact path, size and program id."
    )]
    async fn build(&self, Parameters(p): Parameters<BuildParams>) -> ToolResult {
        let project = match load(p.manifest.as_deref()) {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        let channel = match project.channels().resolve(p.channel.as_deref()) {
            Ok(c) => c,
            Err(e) => return fail(e.to_string()),
        };
        let opts = BuildOptions {
            cluster: p.cluster.clone(),
            ..Default::default()
        };
        if p.dry_run {
            let cmd = build::build_command(&project, &channel, &opts);
            return json(serde_json::json!({
                "channel": channel,
                "command": build::display_command(&cmd),
            }));
        }
        let result = tokio::task::spawn_blocking(move || build::build(&project, &channel, &opts))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
        match result {
            Ok(out) => json(serde_json::json!({
                "channel": out.channel,
                "cluster": out.cluster,
                "program_id": out.program_id,
                "artifact": out.so_path,
                "size_bytes": out.size,
                "seconds": out.elapsed.as_secs_f64(),
            })),
            Err(e) => fail(e),
        }
    }

    #[tool(
        description = "Plan a deploy for one channel: validates the program id, keypair and artifact, and returns the exact `solana program deploy` command with cluster and upgrade authority. This tool never deploys; show the command to the user."
    )]
    async fn deploy_plan(&self, Parameters(p): Parameters<DeployPlanParams>) -> ToolResult {
        let project = match load(p.manifest.as_deref()) {
            Ok(p) => p,
            Err(e) => return fail(e),
        };
        let channel = match project.channels().resolve(p.channel.as_deref()) {
            Ok(c) => c,
            Err(e) => return fail(e.to_string()),
        };
        let opts = deploy::DeployOptions {
            url: p.cluster.clone(),
            ..Default::default()
        };
        match deploy::plans(&project, &channel, &opts) {
            Ok(plans) => json(serde_json::json!(
                plans
                    .iter()
                    .map(|plan| {
                        serde_json::json!({
                            "channel": plan.channel,
                            "program_id": plan.program_id,
                            "cluster": plan.cluster,
                            "artifact": plan.so_path,
                            "program_keypair": plan.program_keypair,
                            "upgrade_authority": plan.upgrade_authority,
                            "command": build::display_command(&plan.command()),
                        })
                    })
                    .collect::<Vec<_>>()
            )),
            Err(e) => fail(e.to_string()),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SolexMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
            .with_server_info(
                Implementation::new("solex", env!("CARGO_PKG_VERSION")).with_title("Solex"),
            )
            .with_instructions(INSTRUCTIONS)
    }
}
