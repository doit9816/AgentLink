use anyhow::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run {
        config_path: String,
        platforms: Vec<String>,
        projects: Vec<String>,
    },
    ValidateConfig {
        path: String,
    },
    Help,
    Version,
    Feishu {
        args: Vec<String>,
    },
    Setup {
        args: Vec<String>,
    },
}

impl Command {
    pub fn parse(args: Vec<String>) -> Result<Self> {
        if args.is_empty() {
            return Ok(Self::Run {
                config_path: "agentlink.toml".to_string(),
                platforms: Vec::new(),
                projects: Vec::new(),
            });
        }
        if args[0] == "feishu" {
            return Ok(Self::Feishu {
                args: args[1..].to_vec(),
            });
        }
        if args[0] == "setup" {
            return Ok(Self::Setup {
                args: args[1..].to_vec(),
            });
        }

        let mut config_path: Option<String> = None;
        let mut platforms = Vec::new();
        let mut projects = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-h" | "--help" => return Ok(Self::Help),
                "-V" | "--version" => return Ok(Self::Version),
                "-c" | "--config" => {
                    i += 1;
                    let Some(value) = args.get(i) else {
                        anyhow::bail!("--config requires a path");
                    };
                    config_path = Some(value.clone());
                }
                "-p" | "--platform" | "--channel" => {
                    i += 1;
                    let Some(value) = args.get(i) else {
                        anyhow::bail!("{} requires a value", args[i - 1]);
                    };
                    extend_csv(&mut platforms, value);
                }
                "--project" => {
                    i += 1;
                    let Some(value) = args.get(i) else {
                        anyhow::bail!("--project requires a value");
                    };
                    extend_csv(&mut projects, value);
                }
                "--validate-config" => {
                    i += 1;
                    let Some(value) = args.get(i) else {
                        anyhow::bail!("--validate-config requires a path");
                    };
                    return Ok(Self::ValidateConfig {
                        path: value.clone(),
                    });
                }
                value if !value.starts_with('-') && config_path.is_none() => {
                    config_path = Some(value.to_string());
                }
                other => anyhow::bail!("unknown argument `{other}`"),
            }
            i += 1;
        }

        Ok(Self::Run {
            config_path: config_path.unwrap_or_else(|| "agentlink.toml".to_string()),
            platforms,
            projects,
        })
    }
}

fn extend_csv(values: &mut Vec<String>, raw: &str) {
    values.extend(
        raw.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    );
}

pub fn print_help(version: &str) {
    println!(
        r#"agentlink {version}

USAGE:
  agentlink [--config <path>] [--platform <id|name|type>] [--project <name>]
  agentlink <path>
  agentlink --validate-config <path>
  agentlink setup --platform <name> --config <path> --project <name>
  agentlink feishu setup --config <path> --project <name>
  agentlink feishu setup --config <path> --project <name> --app <id:secret>

OPTIONS:
  -c, --config <path>       Config file path. Default: agentlink.toml
  -p, --platform <selector> Platform/channel to start. Repeat or use comma list.
      --channel <selector>  Alias of --platform.
      --project <name>      Start only this project. Repeat or use comma list.
      --validate-config     Validate config and exit.
  -V, --version             Print version.
  -h, --help                Print help.
"#
    );
}
