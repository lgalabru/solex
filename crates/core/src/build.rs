//! Drive `cargo build-sbf` once per channel. The channel reaches the proc
//! macros through `SOLEX_CHANNEL`; artifacts land in `target/solex/<channel>/`
//! so several channels can coexist.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use solex_types::{CHANNEL_ENV, CLUSTER_ENV, FEATURES_ENV, MANIFEST_ENV};

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
    /// Arguments appended to `cargo build-sbf` after the manifest's
    /// `build_args` (for example `--arch v3`).
    pub extra_args: Vec<String>,
    /// Cluster within the channel; required when the channel has per-cluster
    /// overrides and several clusters.
    pub cluster: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub channel: String,
    pub cluster: Option<String>,
    pub program_id: String,
    pub so_path: PathBuf,
    pub size: u64,
    pub elapsed: Duration,
}

/// Environment a build needs for the proc macros to resolve `channel`.
///
/// Framework CLIs with their own build pipeline apply these to their
/// `cargo build-sbf` invocation instead of calling [`build`].
pub fn channel_env(project: &Project, channel: &str) -> Vec<(&'static str, String)> {
    target_env(project, channel, None)
}

/// [`channel_env`] plus `SOLEX_CLUSTER` when a cluster is pinned.
pub fn target_env(
    project: &Project,
    channel: &str,
    cluster: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut env = vec![
        (CHANNEL_ENV, channel.to_string()),
        (
            MANIFEST_ENV,
            project.manifest_path().to_string_lossy().into_owned(),
        ),
    ];
    if let Some(c) = cluster {
        env.push((CLUSTER_ENV, c.to_string()));
    }
    // Not read by the macros; for pipelines that need to pass `--features`.
    if let Ok(features) = channel_features(project, channel, &[]) {
        if !features.is_empty() {
            env.push((FEATURES_ENV, features.join(",")));
        }
    }
    env
}

/// Cargo features for a channel: the manifest's channel-managed features
/// enabled on it, plus any extra ones requested.
pub fn channel_features(project: &Project, channel: &str, extra: &[String]) -> Result<Vec<String>> {
    let mut features = project.channels().features_for(channel)?;
    for f in extra {
        if !features.contains(f) {
            features.push(f.clone());
        }
    }
    Ok(features)
}

/// The cluster a build targets, resolved from options, `SOLEX_CLUSTER` and the
/// manifest.
pub fn resolve_cluster(
    project: &Project,
    channel: &str,
    opts: &BuildOptions,
) -> Result<Option<String>> {
    Ok(project
        .channels()
        .channel(channel)?
        .resolve_cluster(channel, opts.cluster.as_deref())?)
}

/// The command that would run, for `--dry-run` and verbose output.
///
/// With the stock pipeline this is `cargo build-sbf` pointed at the program
/// crate and the channel's output directory. With a custom `[build] command`
/// it is that command verbatim, run in the program directory; solex adds only
/// the environment and the manifest's `args`.
pub fn build_command(project: &Project, channel: &str, opts: &BuildOptions) -> Command {
    let cfg = &project.manifest().build;
    let cluster = resolve_cluster(project, channel, opts).ok().flatten();
    let out_dir = project.target_out_dir(channel, cluster.as_deref());
    let features = channel_features(project, channel, &opts.features).unwrap_or_default();
    let mut cmd = if cfg.is_build_sbf() {
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
        if !features.is_empty() {
            cmd.args(["--features", &features.join(",")]);
        }
        cmd
    } else {
        let mut cmd = Command::new(&cfg.command[0]);
        cmd.args(&cfg.command[1..]);
        if !features.is_empty() {
            cmd.args(["--features", &features.join(",")]);
        }
        cmd
    };
    cmd.args(&cfg.args);
    cmd.args(&opts.extra_args);
    cmd.envs(target_env(project, channel, cluster.as_deref()))
        .current_dir(project.program_dir());
    cmd
}

/// Resolve `[build] artifact` for a channel: `{name}`, `{lib}`, `{channel}`.
pub fn artifact_path(project: &Project, channel: &str) -> Option<PathBuf> {
    let pattern = project.manifest().build.artifact.as_ref()?;
    let name = project.name();
    let resolved = pattern
        .replace("{name}", &name)
        .replace("{lib}", &name.replace('-', "_"))
        .replace("{channel}", channel);
    Some(project.root().join(resolved))
}

pub fn build(project: &Project, channel: &str, opts: &BuildOptions) -> Result<BuildOutput> {
    let cluster = resolve_cluster(project, channel, opts)?;
    let ch = project.channels().channel(channel)?;
    let program_id = ch
        .resolved(channel, cluster.as_deref())
        .program_id
        .ok_or_else(|| CoreError::message(format!("channel `{channel}` has no program_id")))?
        .to_string();
    let out_dir = project.target_out_dir(channel, cluster.as_deref());
    std::fs::create_dir_all(&out_dir).map_err(|e| CoreError::io("create", &out_dir, e))?;

    let cfg = &project.manifest().build;
    if !cfg.is_build_sbf() && cfg.artifact.is_none() {
        return Err(CoreError::message(
            "[build] command is set but [build] artifact is not; add the path the command \
             writes, e.g. artifact = \"target/bpfel-unknown-none/release/lib{lib}.so\"",
        ));
    }

    let start = Instant::now();
    let mut cmd = build_command(project, channel, opts);
    let label = if cfg.is_build_sbf() {
        "cargo build-sbf".to_string()
    } else {
        cfg.command.join(" ")
    };
    run(&mut cmd, &label, opts.verbose)?;

    remove_stray_keypairs(&out_dir);
    // A custom pipeline leaves the artifact wherever it likes; copy it into
    // the channel directory so every downstream step sees one layout.
    if let Some(src) = artifact_path(project, channel) {
        let dest = out_dir.join(src.file_name().unwrap_or_default());
        std::fs::copy(&src, &dest).map_err(|e| CoreError::io("copy", &src, e))?;
    }
    let so_path = find_so(&out_dir)?;
    let size = std::fs::metadata(&so_path)
        .map(|m| m.len())
        .map_err(|e| CoreError::io("stat", &so_path, e))?;
    Ok(BuildOutput {
        channel: channel.to_string(),
        cluster,
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

#[cfg(test)]
mod tests {
    use solex_types::{BuildConfig, Channel, Channels};

    use super::*;

    fn project(build: BuildConfig) -> Project {
        let dir = std::env::temp_dir().join(format!("solex-build-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"my-prog\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let mut channels = Channels::default();
        channels.channels.insert(
            "stable".into(),
            Channel {
                program_id: Some("11111111111111111111111111111111".into()),
                ..Default::default()
            },
        );
        let mut p = Project::from_parts(channels, &dir.join("Solex.toml"), None, None).unwrap();
        p.loaded.manifest.build = build;
        p
    }

    #[test]
    fn default_pipeline_is_build_sbf_with_channel_env() {
        let p = project(BuildConfig::default());
        let cmd = build_command(&p, "stable", &BuildOptions::default());
        let shown = display_command(&cmd);
        assert!(shown.contains("SOLEX_CHANNEL=stable"), "{shown}");
        assert!(shown.contains("cargo build-sbf --manifest-path"), "{shown}");
        assert!(shown.contains("target/solex/stable"), "{shown}");
        assert!(artifact_path(&p, "stable").is_none());
    }

    #[test]
    fn custom_pipeline_runs_verbatim_and_resolves_artifact() {
        let p = project(BuildConfig {
            command: vec!["cargo".into(), "+nightly".into(), "build-bpf".into()],
            args: vec!["--release".into()],
            artifact: Some("target/bpfel-unknown-none/release/lib{lib}.so".into()),
        });
        let cmd = build_command(&p, "stable", &BuildOptions::default());
        let shown = display_command(&cmd);
        assert!(
            shown.ends_with("cargo +nightly build-bpf --release"),
            "{shown}"
        );
        assert!(!shown.contains("--sbf-out-dir"), "{shown}");
        let art = artifact_path(&p, "stable").unwrap();
        assert!(
            art.ends_with("target/bpfel-unknown-none/release/libmy_prog.so"),
            "{art:?}"
        );
    }
}
