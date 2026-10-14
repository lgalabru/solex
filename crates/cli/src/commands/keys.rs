use std::path::PathBuf;

use clap::{Args, Subcommand};
use solex_core::{
    CoreError, Result,
    keys::{self, Keypair},
};

use crate::{commands, components, style};

#[derive(Subcommand, Debug)]
pub enum KeysCommand {
    /// Print each channel's program id and keypair status
    List(KeysArgs),
    /// Generate a keypair for a channel and record its id in Solex.yaml
    New(NewArgs),
    /// Verify that every keypair on disk matches its channel's program_id
    Check(KeysArgs),
}

#[derive(Args, Debug, Default)]
pub struct KeysArgs {
    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct NewArgs {
    /// Channel to generate a keypair for
    #[arg(long, short)]
    pub channel: String,

    /// Overwrite an existing keypair (this changes the program address!)
    #[arg(long)]
    pub force: bool,

    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl KeysCommand {
    pub fn execute(&self) -> Result<()> {
        match self {
            KeysCommand::List(a) => list(a),
            KeysCommand::New(a) => new(a),
            KeysCommand::Check(a) => check(a),
        }
    }
}

fn list(args: &KeysArgs) -> Result<()> {
    let project = commands::load_project(args.manifest.as_deref())?;
    let m = project.channels();
    println!();
    for name in m.names() {
        let kp = project.channel_keypair(&name)?;
        let status = if kp.is_file() {
            style::dim(&kp.display().to_string())
        } else {
            style::dim("(no keypair on disk)")
        };
        println!(
            "{}",
            components::fields(&[(
                &name,
                format!("{}  {}", m.program_id(&name).unwrap_or("(unset)"), status)
            )])
        );
    }
    Ok(())
}

fn new(args: &NewArgs) -> Result<()> {
    let mut project = commands::load_project(args.manifest.as_deref())?;
    project.channels().channel(&args.channel)?;
    let path = project.channel_keypair(&args.channel)?;
    if path.exists() && !args.force {
        return Err(CoreError::message(format!(
            "keypair already exists: {}\n  Pass --force to replace it. This changes the program \
             address for channel `{}`.",
            path.display(),
            args.channel
        )));
    }
    let kp = Keypair::generate();
    kp.write(&path)?;
    let id = kp.pubkey();

    let rel = path
        .strip_prefix(project.root())
        .map(|p| p.to_path_buf())
        .unwrap_or(path.clone());
    let ch = project
        .loaded
        .manifest
        .channels
        .channels
        .get_mut(&args.channel)
        .expect("channel validated above");
    ch.program_id = Some(id.clone());
    ch.keypair = Some(rel);
    project.save()?;

    println!(
        "{}",
        components::ok(&format!(
            "{} → {}",
            components::channel(&args.channel),
            style::bold(&id)
        ))
    );
    println!("{}", components::tip(&path.display().to_string()));
    Ok(())
}

fn check(args: &KeysArgs) -> Result<()> {
    let project = commands::load_project(args.manifest.as_deref())?;
    let m = project.channels();
    let mut failures = 0;
    for name in m.names() {
        let declared = m.program_id(&name)?;
        if let Err(reason) = keys::validate_pubkey(declared) {
            println!(
                "{}",
                components::fail(&format!(
                    "{}  program_id `{declared}` is invalid: {reason}",
                    components::channel(&name)
                ))
            );
            failures += 1;
            continue;
        }
        let path = project.channel_keypair(&name)?;
        if !path.is_file() {
            println!(
                "{}",
                components::ok(&format!(
                    "{}  {declared} {}",
                    components::channel(&name),
                    style::dim("(no keypair on disk; upgrades only)")
                ))
            );
            continue;
        }
        let actual = keys::pubkey_of(&path)?;
        if actual == declared {
            println!(
                "{}",
                components::ok(&format!("{}  {declared}", components::channel(&name)))
            );
        } else {
            println!(
                "{}",
                components::fail(&format!(
                    "{}  keypair {} is for {actual}, manifest says {declared}",
                    components::channel(&name),
                    path.display()
                ))
            );
            failures += 1;
        }
    }
    if failures > 0 {
        return Err(CoreError::message(format!(
            "{failures} channel(s) failed the check"
        )));
    }
    Ok(())
}
