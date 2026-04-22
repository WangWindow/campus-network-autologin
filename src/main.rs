mod autostart;
mod config;
mod daemon;
mod network;
mod portal;
mod tui;

use std::time::Duration;

use anyhow::{Result, bail};
use autostart::{install_autostart, remove_autostart, show_autostart_path};
use clap::{Args, Parser, Subcommand};
use config::AppConfig;
use daemon::run_daemon;
use network::detect_campus_environment;
use portal::{LoginStatus, PortalClient, ProbeStatus};
use tui::run_setup_tui;

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    about = "Campus network auto-login for Dr.COM-like portals"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the background monitor with low-frequency checks.
    Run,
    /// Submit one login attempt and verify internet access.
    Login,
    /// Show whether the network is online or captive.
    Status,
    /// Manage the config file from the command line.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Print important filesystem paths.
    Paths,
    /// Manage auto-start registration for the current user.
    Autostart {
        #[command(subcommand)]
        command: AutostartCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Set config values directly from the CLI.
    Set(ConfigSetArgs),
    /// Show the current config with the password redacted.
    Show,
    /// Launch the interactive setup wizard.
    Init,
}

#[derive(Debug, Subcommand)]
enum AutostartCommand {
    /// Install auto-start for the current user.
    Install,
    /// Remove auto-start for the current user.
    Remove,
    /// Show the auto-start file path for this platform.
    Path,
}

#[derive(Debug, Args)]
struct ConfigSetArgs {
    #[arg(long)]
    username: Option<String>,
    #[arg(long)]
    password: Option<String>,
    #[arg(long)]
    portal_url: Option<String>,
    #[arg(long)]
    probe_url: Option<String>,
    #[arg(long)]
    online_check_interval_secs: Option<u64>,
    #[arg(long)]
    request_timeout_secs: Option<u64>,
    #[arg(long, value_delimiter = ',')]
    campus_cidrs: Option<Vec<String>>,
    #[arg(long, value_delimiter = ',')]
    campus_gateways: Option<Vec<String>>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => interactive_setup(), // default to interactive setup if no command is provided
        Some(Command::Run) => {
            let config = AppConfig::load_required()?;
            run_daemon(&config)
        }
        Some(Command::Login) => {
            let config = AppConfig::load_required()?;
            match detect_campus_environment(&config)? {
                network::CampusEnvironment::OnCampus(reason) => {
                    println!("campus network detected: {reason}");
                }
                network::CampusEnvironment::OffCampus(reason) => {
                    bail!("campus network not detected: {reason}");
                }
            }
            let client = portal_client(&config)?;
            match client.login_and_verify(&config)? {
                outcome if outcome.status == LoginStatus::Success => {
                    println!("login successful: {}", outcome.detail);
                    Ok(())
                }
                outcome => {
                    bail!("login failed: {}", outcome.detail);
                }
            }
        }
        Some(Command::Status) => {
            let config = AppConfig::load_required()?;
            match detect_campus_environment(&config)? {
                network::CampusEnvironment::OnCampus(reason) => {
                    println!("environment: on-campus ({reason})");
                }
                network::CampusEnvironment::OffCampus(reason) => {
                    println!("environment: off-campus ({reason})");
                    return Ok(());
                }
            }
            let client = portal_client(&config)?;
            match client.probe(&config)? {
                ProbeStatus::Online => println!("status: online"),
                ProbeStatus::NeedsLogin => println!("status: captive portal detected"),
                ProbeStatus::Unreachable(detail) => println!("status: probe failed ({detail})"),
            }
            Ok(())
        }
        Some(Command::Config { command }) => run_config_command(command),
        Some(Command::Paths) => {
            println!("config: {}", AppConfig::config_path()?.display());
            Ok(())
        }
        Some(Command::Autostart { command }) => run_autostart_command(command),
    }
}

fn run_config_command(command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Set(args) => {
            let mut config = AppConfig::load().unwrap_or_default();
            let mut changed = false;

            if let Some(username) = args.username {
                config.auth.username = username;
                changed = true;
            }
            if let Some(password) = args.password {
                config.auth.password = password;
                changed = true;
            }
            if let Some(portal_url) = args.portal_url {
                config.auth.portal_url = portal_url;
                changed = true;
            }
            if let Some(probe_url) = args.probe_url {
                config.detect.probe_url = probe_url;
                changed = true;
            }
            if let Some(interval) = args.online_check_interval_secs {
                config.daemon.online_check_interval_secs = interval;
                changed = true;
            }
            if let Some(timeout) = args.request_timeout_secs {
                config.detect.request_timeout_secs = timeout;
                changed = true;
            }
            if let Some(campus_cidrs) = args.campus_cidrs {
                config.campus.ipv4_cidrs = campus_cidrs;
                changed = true;
            }
            if let Some(campus_gateways) = args.campus_gateways {
                config.campus.gateway_hosts = campus_gateways;
                changed = true;
            }

            if !changed {
                bail!(
                    "no values provided; use --username/--password/--portal-url/--probe-url/--campus-cidrs/--campus-gateways"
                );
            }

            config.validate()?;
            config.save()?;
            println!("saved config to {}", AppConfig::config_path()?.display());
            Ok(())
        }
        ConfigCommand::Show => {
            let config = AppConfig::load_required()?;
            println!("{}", config.redacted_toml()?);
            Ok(())
        }
        ConfigCommand::Init => interactive_setup(),
    }
}

fn interactive_setup() -> Result<()> {
    run_setup_tui(AppConfig::load().unwrap_or_default())
}

fn run_autostart_command(command: AutostartCommand) -> Result<()> {
    match command {
        AutostartCommand::Install => {
            let path = install_autostart()?;
            println!("installed autostart file: {}", path.display());
            Ok(())
        }
        AutostartCommand::Remove => {
            let path = remove_autostart()?;
            println!("removed autostart file: {}", path.display());
            Ok(())
        }
        AutostartCommand::Path => {
            println!("autostart file: {}", show_autostart_path()?.display());
            Ok(())
        }
    }
}

fn portal_client(config: &AppConfig) -> Result<PortalClient> {
    PortalClient::new(Duration::from_secs(config.detect.request_timeout_secs))
}
