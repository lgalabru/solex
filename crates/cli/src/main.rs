//! `solex` — ship Solana program features per release channel.

use clap::{CommandFactory, Parser, Subcommand};

mod boxed;
mod commands;
mod style;

#[derive(Parser, Debug)]
#[command(
    name = "solex",
    version,
    about = "Every increment ships. Only the proven ones reach stable.",
    propagate_version = true
)]
struct Cli {
    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scaffold a new Solex project (Anchor program + manifest + channel keypairs)
    Init(commands::init::InitCommand),
    /// Compile the program for one channel (or all)
    Build(commands::build::BuildCommand),
    /// Deploy a channel's build to its cluster
    Deploy(commands::deploy::DeployCommand),
    /// Terminate a channel: close its program or buffers and reclaim rent
    Terminate(commands::terminate::TerminateCommand),
    /// Print the environment that selects a channel, for other build pipelines
    Env(commands::env::EnvCommand),
    /// Run a command with a channel selected (e.g. cargo +nightly build-bpf)
    Exec(commands::exec::ExecCommand),
    /// List channels, program ids and what each one ships
    Channels(commands::channels::ChannelsCommand),
    /// Show the #[channel] matrix: which items ship on which channel
    Features(commands::features::FeaturesCommand),
    /// Manage per-channel program keypairs
    #[command(subcommand)]
    Keys(commands::keys::KeysCommand),
    /// Run `cargo test` with the channel selected
    Test(commands::test::TestCommand),
    /// Print the resolved channel name (for scripts)
    Channel(commands::channel::ChannelCommand),
    /// Install the agent skill and optionally register the MCP server
    Setup(commands::setup::SetupCommand),
    /// Run the MCP server over stdio (channels, feature matrix, builds, deploy plans)
    Mcp(commands::mcp::McpCommand),
    /// Generate shell completions
    Completions(commands::completions::CompletionsCommand),
}

fn main() {
    let cli = Cli::parse();
    style::init(!cli.no_color);
    let result = match cli.command {
        Command::Init(cmd) => cmd.execute(),
        Command::Build(cmd) => cmd.execute(),
        Command::Deploy(cmd) => cmd.execute(),
        Command::Terminate(cmd) => cmd.execute(),
        Command::Env(cmd) => cmd.execute(),
        Command::Exec(cmd) => cmd.execute(),
        Command::Channels(cmd) => cmd.execute(),
        Command::Features(cmd) => cmd.execute(),
        Command::Keys(cmd) => cmd.execute(),
        Command::Test(cmd) => cmd.execute(),
        Command::Channel(cmd) => cmd.execute(),
        Command::Setup(cmd) => cmd.execute(),
        Command::Mcp(cmd) => cmd.execute(),
        Command::Completions(cmd) => {
            cmd.execute(&mut Cli::command());
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("{} {e}", style::fail());
        std::process::exit(e.exit_code());
    }
}
