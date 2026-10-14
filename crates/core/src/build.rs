//! Drive `cargo build-sbf` once per channel. The channel reaches the proc
//! macros through `SOLEX_CHANNEL`; artifacts land in `target/solex/<channel>/`
//! so several channels can coexist.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use solex_manifest::{CHANNEL_ENV, MANIFEST_ENV};

use crate::{CoreError, Project, Result};

#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Extra cargo features for the program crate.
    pub features: Vec<String>,
    /// Stream compiler output instead of capturing it.
    pub verbose: bool,
    /// Pass `--debug` to build-sbf (keeps debug symbols).
    pub debug: bool,
    /// Pin platform-tools (`--tools-version vX.Y`).
    pub tools_version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub channel: String,
    pub program_id: String,
    pub so_path: PathBuf,
    pub size: u64,
    pub elapsed: Duration,
}

/// The command that would run, for `--dry-run` and verbose output.
pub fn build_command(project: &Project, channel: &str, opts: &BuildOptions) -> Command {
    let out_dir = project.channel_out_dir(channel);
    let mut cmd = Command::new("cargo");
    cmd.arg("build-sbf")
        .arg("--manifest-path")
        .arg(project.program_manifest())
        .arg("--sbf-out-dir")
        .arg(&out_dir);
    if let Some(v) = &opts.tools_version {
        cmd.args(["--tools-version", v]);
    }
    if opts.debug {
        cmd.arg("--debug");
    }
    if !opts.features.is_empty() {
        cmd.args(["--features", &opts.features.join(",")]);
    }
    cmd.env(CHANNEL_ENV, channel)
        .env(MANIFEST_ENV, project.manifest_path())
        .current_dir(project.root());
    cmd
}

pub fn build(project: &Project, channel: &str, opts: &BuildOptions) -> Result<BuildOutput> {
    let manifest = project.manifest();
    let program_id = manifest.program_id(channel)?.to_string();
    let out_dir = project.channel_out_dir(channel);
    std::fs::create_dir_all(&out_dir).map_err(|e| CoreError::io("create", &out_dir, e))?;

    let start = Instant::now();
    let mut cmd = build_command(project, channel, opts);
    run(&mut cmd, "cargo build-sbf", opts.verbose)?;

    remove_stray_keypairs(&out_dir);
    let so_path = find_so(&out_dir)?;
    let size = std::fs::metadata(&so_path)
        .map(|m| m.len())
        .map_err(|e| CoreError::io("stat", &so_path, e))?;
    Ok(BuildOutput {
        channel: channel.to_string(),
        program_id,
        so_path,
        size,
        elapsed: start.elapsed(),
    })
}

/// `cargo build-sbf` drops a freshly generated `<name>-keypair.json` next to
/// the binary. Solex owns program keys under `keys/`, and a random key sitting
/// beside the artifact invites deploying under the wrong address, so drop it.
fn remove_stray_keypairs(out_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(out_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_keypair = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("-keypair.json"));
        if is_keypair {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Run a child process; capture output unless `verbose`, and turn a non-zero
/// exit into `CommandFailed` carrying whatever was captured.
pub fn run(cmd: &mut Command, label: &str, verbose: bool) -> Result<String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    if verbose {
        let status = cmd
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|source| CoreError::Spawn {
                program: program.clone(),
                source,
            })?;
        if !status.success() {
            return Err(CoreError::CommandFailed {
                command: label.to_string(),
                code: status.code().unwrap_or(1),
                output: String::new(),
            });
        }
        return Ok(String::new());
    }
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|source| CoreError::Spawn { program, source })?;
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(stdout.trim_end());
    }
    if !output.status.success() {
        return Err(CoreError::CommandFailed {
            command: label.to_string(),
            code: output.status.code().unwrap_or(1),
            output: text,
        });
    }
    Ok(text)
}

/// Newest `.so` in the channel output directory.
pub fn find_so(out_dir: &Path) -> Result<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    let channel = out_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let not_built = || {
        CoreError::message(format!(
            "no build found for channel `{channel}` ({})\n  Run `solex build --channel \
             {channel}` first, or pass --build to deploy.",
            out_dir.display()
        ))
    };
    let entries = std::fs::read_dir(out_dir).map_err(|_| not_built())?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "so") {
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                best = Some((modified, path));
            }
        }
    }
    best.map(|(_, p)| p).ok_or_else(not_built)
}

/// Render a command the way a user could paste it into a shell.
pub fn display_command(cmd: &Command) -> String {
    let mut parts: Vec<String> = cmd
        .get_envs()
        .filter_map(|(k, v)| v.map(|v| format!("{}={}", k.to_string_lossy(), v.to_string_lossy())))
        .collect();
    parts.push(cmd.get_program().to_string_lossy().into_owned());
    parts.extend(cmd.get_args().map(|a| {
        let a = a.to_string_lossy();
        if a.contains(' ') {
            format!("\"{a}\"")
        } else {
            a.into_owned()
        }
    }));
    parts.join(" ")
}
