use std::path::PathBuf;

use clap::Args;
use solex_core::{
    Result, build,
    deploy::{self, DeployOptions},
};

use crate::{commands, style};

#[derive(Args, Debug, Default)]
pub struct DeployCommand {
    /// Channel to deploy (default: SOLEX_CHANNEL or the manifest default)
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Build the channel first
    #[arg(long)]
    pub build: bool,

    /// Override the channel's cluster (moniker or RPC URL)
    #[arg(long, short)]
    pub url: Option<String>,

    /// Fee payer keypair (default: Solana CLI default keypair)
    #[arg(long, short)]
    pub keypair: Option<PathBuf>,

    /// Override the channel's upgrade authority keypair
    #[arg(long)]
    pub upgrade_authority: Option<PathBuf>,

    /// Print the deploy command without running it
    #[arg(long)]
    pub dry_run: bool,

    /// Stream Solana CLI output
    #[arg(long, short)]
    pub verbose: bool,

    /// Path to the manifest (Solex.toml, Solex.yaml, or a host manifest with a [solex] section)
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl DeployCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let channel = project.channels().resolve(self.channel.as_deref())?;

        if self.build {
            let opts = build::BuildOptions {
                verbose: self.verbose,
                ..Default::default()
            };
            let sp = style::spinner(&format!("Building {}...", style::accent(&channel)));
            let r = build::build(&project, &channel, &opts);
            sp.finish_and_clear();
            r?;
        }

        let opts = DeployOptions {
            url: self.url.clone(),
            payer: self.keypair.clone(),
            upgrade_authority: self.upgrade_authority.clone(),
            verbose: self.verbose,
        };
        let plan = deploy::plan(&project, &channel, &opts)?;

        println!();
        println!(
            "  {:<10} {}",
            style::dim("channel"),
            style::accent(&plan.channel)
        );
        println!("  {:<10} {}", style::dim("program"), plan.program_id);
        println!(
            "  {:<10} {}",
            style::dim("cluster"),
            plan.cluster.as_deref().unwrap_or("(solana CLI default)")
        );
        println!("  {:<10} {}", style::dim("binary"), plan.so_path.display());
        if let Some(auth) = &plan.upgrade_authority {
            println!("  {:<10} {}", style::dim("authority"), auth.display());
        }
        println!();

        if self.dry_run {
            println!("{}", build::display_command(&plan.command()));
            return Ok(());
        }

        let sp = style::spinner("Deploying...");
        let r = deploy::deploy(&plan, self.verbose);
        sp.finish_and_clear();
        let id = r?;
        println!(
            "  {} Deployed {} to {}",
            style::ok(),
            style::accent(&plan.channel),
            style::bold(&id)
        );
        println!();
        Ok(())
    }
}
