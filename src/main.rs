use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use llm_relay::{
    build_app, build_http_client,
    config::Config,
    security::{generate_and_store_api_key, load_or_create_api_key, read_api_key},
};
use tokio::{net::TcpListener, runtime::Builder};
use tracing::info;

const DEFAULT_CONFIG_PATH: &str = "/etc/llm-relay/config.yaml";

fn main() {
    if let Err(error) = run() {
        eprintln!("startup_error={error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
    match parse_command()? {
        Command::Run { paths } => run_server(paths),
        Command::Start => run_service_command("start"),
        Command::Stop => run_service_command("stop"),
        Command::Restart => run_service_command("restart"),
        Command::Status => run_service_command("status"),
        Command::Logs => run_logs_command(),
        Command::GenerateKey { paths, force } => generate_key(paths, force),
        Command::ShowKey { paths } => show_key(paths),
        Command::Help => {
            print_help();
            Ok(())
        }
    }
}

fn run_server(paths: CliPaths) -> Result<(), Box<dyn Error + Send + Sync>> {
    init_tracing();

    let config_path = paths.config_path.clone();
    let config = load_config(&paths)?;
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

fn generate_key(paths: CliPaths, force: bool) -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = load_config(&paths)?;
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

fn show_key(paths: CliPaths) -> Result<(), Box<dyn Error + Send + Sync>> {
    let config = load_config(&paths)?;
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

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run { paths: CliPaths },
    Start,
    Stop,
    Restart,
    Status,
    Logs,
    GenerateKey { paths: CliPaths, force: bool },
    ShowKey { paths: CliPaths },
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliPaths {
    config_path: PathBuf,
    api_key_file: Option<PathBuf>,
}

fn parse_command() -> Result<Command, Box<dyn Error + Send + Sync>> {
    let args: Vec<String> = env::args().skip(1).collect();
    parse_command_args(&args)
}

fn parse_command_args(args: &[String]) -> Result<Command, Box<dyn Error + Send + Sync>> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(Command::Run {
            paths: parse_paths(&[])?,
        });
    };

    match command {
        "--help" | "-h" => Ok(Command::Help),
        "run" => {
            if contains_help_flag(&args[1..]) {
                Ok(Command::Help)
            } else {
                Ok(Command::Run {
                    paths: parse_paths(&args[1..])?,
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
                    paths: parse_paths(&config_args)?,
                    force,
                })
            }
        }
        "show-key" => {
            if contains_help_flag(&args[1..]) {
                Ok(Command::Help)
            } else {
                Ok(Command::ShowKey {
                    paths: parse_paths(&args[1..])?,
                })
            }
        }
        value
            if matches!(value, "-c" | "--config" | "--api-key-file")
                || value.starts_with("--config=")
                || value.starts_with("--api-key-file=") =>
        {
            if contains_help_flag(&args) {
                Ok(Command::Help)
            } else {
                Ok(Command::Run {
                    paths: parse_paths(args)?,
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

fn parse_paths(args: &[String]) -> Result<CliPaths, Box<dyn Error + Send + Sync>> {
    let mut paths = default_paths();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "-c" | "--config" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("missing value after --config".into());
                };
                paths.config_path = PathBuf::from(value);
            }
            value if value.starts_with("--config=") => {
                let value = value.trim_start_matches("--config=");
                if value.is_empty() {
                    return Err("missing value after --config=".into());
                }
                paths.config_path = PathBuf::from(value);
            }
            "--api-key-file" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err("missing value after --api-key-file".into());
                };
                paths.api_key_file = Some(PathBuf::from(value));
            }
            value if value.starts_with("--api-key-file=") => {
                let value = value.trim_start_matches("--api-key-file=");
                if value.is_empty() {
                    return Err("missing value after --api-key-file=".into());
                }
                paths.api_key_file = Some(PathBuf::from(value));
            }
            "--help" | "-h" => return Ok(paths),
            other => return Err(format!("unknown option: {other}").into()),
        }
        index += 1;
    }

    Ok(paths)
}

fn default_paths() -> CliPaths {
    CliPaths {
        config_path: env::var_os("LLM_RELAY_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH)),
        api_key_file: env::var_os("LLM_RELAY_API_KEY_FILE").map(PathBuf::from),
    }
}

fn load_config(paths: &CliPaths) -> Result<Config, Box<dyn Error + Send + Sync>> {
    let mut config = Config::load_from_path(&paths.config_path)?;

    if let Some(api_key_file) = &paths.api_key_file {
        config.security.api_key_file = resolve_key_path(&paths.config_path, api_key_file);
    }

    Ok(config)
}

fn resolve_key_path(config_path: &Path, api_key_file: &Path) -> PathBuf {
    if api_key_file.is_absolute() {
        api_key_file.to_path_buf()
    } else {
        config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(api_key_file)
    }
}

fn print_help() {
    println!(
        "\
LLM Relay

Usage:
  llm-relay [run] [--config <path>] [--api-key-file <path>]
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
      --api-key-file <path>     Override the relay API key storage path
  -h, --help                    Show this help

Defaults:
  Config: /etc/llm-relay/config.yaml
  Key:    security.api_key_file from the selected config
  Env:    LLM_RELAY_CONFIG and LLM_RELAY_API_KEY_FILE override these defaults

Client authentication:
  Put the relay key in the proxy URL:
  /proxy/<relay-api-key>/<provider>/<provider-path>
"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paths_for_direct_run_options() {
        let command = parse_command_args(&args(&[
            "--config=/tmp/relay/config.yaml",
            "--api-key-file=keys/relay.key",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::Run {
                paths: CliPaths {
                    config_path: PathBuf::from("/tmp/relay/config.yaml"),
                    api_key_file: Some(PathBuf::from("keys/relay.key")),
                },
            }
        );
    }

    #[test]
    fn parses_generate_key_with_custom_paths() {
        let command = parse_command_args(&args(&[
            "generate-key",
            "--force",
            "--config",
            "/tmp/relay/config.yaml",
            "--api-key-file",
            "/var/lib/llm-relay/api_key",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::GenerateKey {
                paths: CliPaths {
                    config_path: PathBuf::from("/tmp/relay/config.yaml"),
                    api_key_file: Some(PathBuf::from("/var/lib/llm-relay/api_key")),
                },
                force: true,
            }
        );
    }

    #[test]
    fn resolves_relative_key_paths_beside_the_config() {
        assert_eq!(
            resolve_key_path(
                Path::new("/etc/llm-relay/config.yaml"),
                Path::new("custom-api-key")
            ),
            PathBuf::from("/etc/llm-relay/custom-api-key")
        );
        assert_eq!(
            resolve_key_path(
                Path::new("/etc/llm-relay/config.yaml"),
                Path::new("/var/lib/llm-relay/api_key")
            ),
            PathBuf::from("/var/lib/llm-relay/api_key")
        );
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }
}
