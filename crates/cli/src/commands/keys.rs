use std::path::PathBuf;

use clap::{Args, Subcommand};
use solex_core::{
    CoreError, Result,
    keys::{self, Keypair},
};

use crate::{commands, components, components::boxed, style};

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
    let sections: Vec<Vec<boxed::Line>> = m
        .names()
        .iter()
        .map(|name| {
            let kp = project.channel_keypair(name).unwrap_or_default();
            let on_disk = if kp.is_file() {
                components::rel(project.root(), &kp)
            } else {
                "none on disk (upgrades only)".into()
            };
            boxed::section(
                &[boxed::styled(style::bold(&components::channel(name)), name)],
                boxed::plain(""),
                &[
                    (
                        "program",
                        boxed::plain(m.program_id(name).unwrap_or("(unset)")),
                    ),
                    ("keypair", boxed::plain(on_disk)),
                ],
            )
        })
        .collect();
    println!("{}", boxed::frame(&sections));
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
    let mut failures: Vec<String> = Vec::new();
    let mut sections: Vec<Vec<boxed::Line>> = Vec::new();
    for name in m.names() {
        let declared = m.program_id(&name)?.to_string();
        let path = project.channel_keypair(&name)?;
        let (status, detail) = match keys::validate_pubkey(&declared) {
            Err(reason) => {
                failures.push(format!(
                    "{name}: program_id `{declared}` is invalid: {reason}"
                ));
                (style::fail(), format!("invalid program_id: {reason}"))
            }
            Ok(()) if !path.is_file() => (style::ok(), "no keypair on disk (upgrades only)".into()),
            Ok(()) => {
                let actual = keys::pubkey_of(&path)?;
                if actual == declared {
                    (style::ok(), components::rel(project.root(), &path))
                } else {
                    failures.push(format!(
                        "{name}: keypair {} is for {actual}, manifest says {declared}",
                        path.display()
                    ));
                    (
                        style::fail(),
                        format!(
                            "keypair is for {}, manifest says {}",
                            components::short(&actual),
                            components::short(&declared)
                        ),
                    )
                }
            }
        };
        sections.push(boxed::section(
            &[
                boxed::styled(status, "x"),
                boxed::styled(style::bold(&components::channel(&name)), &name),
            ],
            boxed::plain(""),
            &[
                ("program", boxed::plain(declared)),
                ("keypair", boxed::plain(detail)),
            ],
        ));
    }
    println!("{}", boxed::frame(&sections));
    if !failures.is_empty() {
        components::print_notice(
            components::NoticeLevel::Error,
            &format!("{} channel(s) failed the check", failures.len()),
            &failures.join("\n"),
        );
        return Err(CoreError::Reported { code: 1 });
    }
    Ok(())
}
