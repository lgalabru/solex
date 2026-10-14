pub mod build;
pub mod channel;
pub mod channels;
pub mod completions;
pub mod deploy;
pub mod features;
pub mod init;
pub mod keys;
pub mod test;

use solex_core::{CoreError, Project, Result};

/// Load the project from the current directory, or from `--manifest` when given.
pub fn load_project(manifest: Option<&std::path::Path>) -> Result<Project> {
    match manifest {
        Some(p) => Ok(Project {
            loaded: solex_core::manifest::load(p)?,
        }),
        None => Project::discover_cwd(),
    }
}

/// `--all` wins; otherwise one explicit or resolved channel.
pub fn select_channels(project: &Project, channel: Option<&str>, all: bool) -> Result<Vec<String>> {
    if all {
        return Ok(project.manifest().channel_names());
    }
    Ok(vec![project.manifest().resolve_channel(channel)?])
}

pub fn ensure_nonempty_channels(v: &[String]) -> Result<()> {
    if v.is_empty() {
        return Err(CoreError::message("no channels defined in Solex.yaml"));
    }
    Ok(())
}
