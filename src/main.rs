use std::{env, error::Error, path::PathBuf, process::Command as ProcessCommand};

use llm_relay::{
    build_app, build_http_client,
    config::Config,
    security::{generate_and_store_api_key, load_or_create_api_key, read_api_key},
};
use tokio::{net::TcpListener, runtime::Builder};
use tracing::info;

fn main() {
    if let Err(error) = run() {
        eprintln!("startup_error={error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    match parse_command()? {
        Command::Run { config_path } => run_server(config_path),
        Command::Start => run_service_command("start"),
        Command::Stop => run_service_command("stop"),
        Command::Restart => run_service_command("restart"),
        Command::Status => run_service_command("status"),
        Command::Logs => run_logs_command(),
        Command::GenerateKey { config_path, force } => generate_key(config_path, force),
        Command::ShowKey { config_path } => show_key(config_path),
        Command::Help => {
            print_help();
            Ok(())
        }
    }
}

fn run_server(config_path: PathBuf) -> Result<(), Box<dyn Error + Send + Sync>> {
    init_tracing();

    let config = Config::load_from_path(&config_path)?;
    let api_key_path = config.security.api_key_file.clone();
    let api_key = load_or_create_api_key(&api_key_path)?;
    let worker_threads = config.runtime.worker_threads;

    Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()?
        .block_on(async move { serve(config, config_path, api_key).await })
}

async fn serve(
    config: Config,
    config_path: PathBuf,
    api_key: String,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let bind_addr = format!("{}:{}", config.server.host, config.server.port);
    let client = build_http_client(&config)?;
    let app = build_app(config, client, api_key);
    let listener = TcpListener::bind(&bind_addr).await?;
    let local_addr = listener.local_addr()?;

    info!(
        address = %local_addr,
        config = %config_path.display(),
        "llm-relay listening"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %error, "failed to listen for shutdown signal");
    }
    info!("shutdown signal received");
}

fn init_tracing() {
    let filter = env::var("RUST_LOG").unwrap_or_else(|_| "llm_relay=info,tower_http=warn".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

fn generate_key(config_path: PathBuf, force: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = Config::load_from_path(config_path)?;
    let path = config.security.api_key_file;
    let key = if force {
        generate_and_store_api_key(&path)?
    } else {
        load_or_create_api_key(&path)?
    };

    println!("API key: {key}");
    println!("Stored at: {}", path.display());
    if force {
        println!("Restart the relay for the new key to take effect.");
    }
    Ok(())
}

fn show_key(config_path: PathBuf) -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = Config::load_from_path(config_path)?;
    let path = config.security.api_key_file;
    let key = read_api_key(&path)?;
    println!("{key}");
    Ok(())
}

fn run_service_command(action: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
    let status = ProcessCommand::new("systemctl")
        .arg(action)
        .arg("llm-relay")
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("systemctl {action} llm-relay failed with status {status}").into())
    }
}

fn run_logs_command() -> Result<(), Box<dyn Error + Send + Sync>> {
    let status = ProcessCommand::new("journalctl")
        .args(["-u", "llm-relay", "-f"])
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("journalctl failed with status {status}").into())
    }
}

enum Command {
    Run { config_path: PathBuf },
    Start,
    Stop,
    Restart,
    Status,
    Logs,
    GenerateKey { config_path: PathBuf, force: bool },
    ShowKey { config_path: PathBuf },
    Help,
}

fn parse_command() -> Result<Command, Box<dyn Error + Send + Sync>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(Command::Run {
            config_path: parse_config_path(&[])?,
        });
    };

    match command {
        "--help" | "-h" => Ok(Command::Help),
        "run" => {
            if contains_help_flag(&args[1..]) {
                Ok(Command::Help)
            } else {
                Ok(Command::Run {
                    config_path: parse_config_path(&args[1..])?,
                })
            }
        }
        "start" => service_command_without_arguments(Command::Start, &args[1..]),
        "stop" => service_command_without_arguments(Command::Stop, &args[1..]),
        "restart" => service_command_without_arguments(Command::Restart, &args[1..]),
        "status" => service_command_without_arguments(Command::Status, &args[1..]),
        "logs" => service_command_without_arguments(Command::Logs, &args[1..]),
        "generate-key" => {
            if contains_help_flag(&args[1..]) {
                Ok(Command::Help)
            } else {
                let (config_args, force) = split_force_flag(&args[1..])?;
                Ok(Command::GenerateKey {
                    config_path: parse_config_path(&config_args)?,
                    force,
                })
            }
        }
        "show-key" => {
            if contains_help_flag(&args[1..]) {
                Ok(Command::Help)
            } else {
                Ok(Command::ShowKey {
                    config_path: parse_config_path(&args[1..])?,
                })
            }
        }
        "-c" | "--config" => {
            if contains_help_flag(&args) {
                Ok(Command::Help)
            } else {
                Ok(Command::Run {
                    config_path: parse_config_path(&args)?,
                })
            }
        }
        _ if command.starts_with('-') => Err(format!("unknown option: {command}").into()),
        _ => Err(format!("unknown command: {command}").into()),
    }
}

fn service_command_without_arguments(
    command: Command,
    args: &[String],
) -> Result<Command, Box<dyn Error + Send + Sync>> {
    if args.is_empty() {
        Ok(command)
    } else {
        Err(format!("unexpected arguments: {}", args.join(" ")).into())
    }
}

fn split_force_flag(args: &[String]) -> Result<(Vec<String>, bool), Box<dyn Error + Send + Sync>> {
    let mut config_args = Vec::new();
    let mut force = false;

    for arg in args {
        if arg == "--force" {
            force = true;
        } else {
            config_args.push(arg.clone());
        }
    }

    Ok((config_args, force))
}

fn contains_help_flag(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
}

fn parse_config_path(args: &[String]) -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    let mut config_path = default_config_path();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "-c" | "--config" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("missing value after --config".into());
                };
                config_path = PathBuf::from(value);
            }
            value if value.starts_with("--config=") => {
                let value = value.trim_start_matches("--config=");
                if value.is_empty() {
                    return Err("missing value after --config=".into());
                }
                config_path = PathBuf::from(value);
            }
            "--help" | "-h" => return Ok(config_path),
            other => return Err(format!("unknown option: {other}").into()),
        }
        index += 1;
    }

    Ok(config_path)
}

fn default_config_path() -> PathBuf {
    if let Some(path) = env::var_os("LLM_RELAY_CONFIG") {
        return PathBuf::from(path);
    }

    let installed_path = PathBuf::from("/etc/llm-relay/config.yaml");
    if installed_path.exists() {
        installed_path
    } else {
        PathBuf::from("config/config.yaml")
    }
}

fn print_help() {
    println!(
        "\
LLM Relay

Usage:
  llm-relay [run] [--config <path>]
  llm-relay <command>

Commands:
  start                         Start the systemd service
  stop                          Stop the systemd service
  restart                       Restart the systemd service
  status                        Show systemd service status
  logs                          Follow systemd journal logs
  generate-key [--force]        Create or rotate the local relay API key
  show-key                      Print the local relay API key
  help                          Show this help

Options:
  -c, --config <path>           Configuration file path
  -h, --help                    Show this help

Client authentication:
  Put the relay key in the proxy URL:
  /proxy/<relay-api-key>/<provider>/<provider-path>
"
    );
}
