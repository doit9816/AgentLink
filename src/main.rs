use agentlink::app::bootstrap::{default_registry, run, validate_with_registry};
use agentlink::app::command::{print_help, Command};
use agentlink::app::config::Config;
use agentlink::setup::channel_setup::run_setup_command;
use agentlink::setup::feishu_setup::run_feishu_command;
use anyhow::Result;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let command = Command::parse(std::env::args().skip(1).collect())?;
    match command {
        Command::Help => {
            print_help(VERSION);
            Ok(())
        }
        Command::Version => {
            println!("agentlink {VERSION}");
            Ok(())
        }
        Command::ValidateConfig { path } => {
            let config = Config::load(&path)?;
            validate_with_registry(&config, &default_registry())?;
            println!("config ok: {path}");
            Ok(())
        }
        Command::Run {
            config_path,
            platforms,
            projects,
        } => run(&config_path, &platforms, &projects).await,
        Command::Feishu { args } => run_feishu_command(args).await,
        Command::Setup { args } => run_setup_command(args).await,
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("agentlink=info,info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}
