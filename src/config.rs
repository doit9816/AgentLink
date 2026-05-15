use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_data_dir")]
    pub data_dir: String,
    pub projects: Vec<ProjectConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub agent: AgentConfig,
    #[serde(default, alias = "default_channels")]
    pub default_platforms: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<PlatformConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub options: toml::value::Table,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlatformConfig {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub options: toml::value::Table,
}

fn default_data_dir() -> String {
    "./data".to_string()
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&raw)?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if self.projects.is_empty() {
            return Err(anyhow!("config requires at least one [[projects]]"));
        }
        let mut names = std::collections::BTreeSet::new();
        for project in &self.projects {
            if project.name.trim().is_empty() {
                return Err(anyhow!("project name is required"));
            }
            if !names.insert(project.name.clone()) {
                return Err(anyhow!("duplicate project `{}`", project.name));
            }
            if project.agent.kind.trim().is_empty() {
                return Err(anyhow!("project `{}` agent.type is required", project.name));
            }
            if project.platforms.is_empty() {
                return Err(anyhow!(
                    "project `{}` requires at least one platform",
                    project.name
                ));
            }
            for platform in &project.platforms {
                if platform.kind.trim().is_empty() {
                    return Err(anyhow!(
                        "project `{}` platform.type is required",
                        project.name
                    ));
                }
            }
            for requested in &project.default_platforms {
                if project.find_platform(requested).is_none() {
                    return Err(anyhow!(
                        "project `{}` default platform `{}` was not found",
                        project.name,
                        requested
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ProjectConfig {
    pub fn selected_platforms<'a>(
        &'a self,
        requested: &[String],
    ) -> Result<Vec<&'a PlatformConfig>> {
        let selectors = if requested.is_empty() {
            &self.default_platforms
        } else {
            requested
        };
        if selectors.is_empty() {
            return Ok(self.platforms.iter().collect());
        }

        let mut selected = Vec::new();
        for selector in selectors {
            let Some(platform) = self.find_platform(selector) else {
                return Err(anyhow!(
                    "project `{}` platform `{}` was not found",
                    self.name,
                    selector
                ));
            };
            if !selected
                .iter()
                .any(|existing: &&PlatformConfig| std::ptr::eq(*existing, platform))
            {
                selected.push(platform);
            }
        }
        Ok(selected)
    }

    pub fn find_platform(&self, selector: &str) -> Option<&PlatformConfig> {
        self.platforms
            .iter()
            .find(|platform| platform.matches_selector(selector))
    }
}

impl PlatformConfig {
    pub fn display_name(&self) -> String {
        self.id
            .clone()
            .or_else(|| option_name(&self.options))
            .unwrap_or_else(|| self.kind.clone())
    }

    pub fn matches_selector(&self, selector: &str) -> bool {
        let selector = selector.trim();
        !selector.is_empty()
            && (self.id.as_deref() == Some(selector)
                || self.kind == selector
                || option_name(&self.options).as_deref() == Some(selector))
    }
}

fn option_name(options: &toml::value::Table) -> Option<String> {
    options
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn project_uses_default_platforms_when_no_cli_selection() {
        let raw = r#"
data_dir = "./data"

[[projects]]
name = "demo"
default_platforms = ["feishu-main"]

[projects.agent]
type = "mock"

[[projects.platforms]]
id = "feishu-main"
type = "feishu"

[projects.platforms.options]
name = "feishu"
dry_run = true

[[projects.platforms]]
id = "qq-local"
type = "qq"

[projects.platforms.options]
name = "qq"
dry_run = true
"#;
        let cfg: Config = toml::from_str(raw).unwrap();
        cfg.validate().unwrap();
        let selected = cfg.projects[0].selected_platforms(&[]).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].display_name(), "feishu-main");
    }

    #[test]
    fn cli_platform_selection_can_match_id_name_or_type() {
        let raw = r#"
data_dir = "./data"

[[projects]]
name = "demo"

[projects.agent]
type = "mock"

[[projects.platforms]]
id = "line-prod"
type = "line"

[projects.platforms.options]
name = "line-main"
dry_run = true

[[projects.platforms]]
type = "qq"

[projects.platforms.options]
name = "qq-local"
dry_run = true
"#;
        let cfg: Config = toml::from_str(raw).unwrap();
        cfg.validate().unwrap();
        let selected = cfg.projects[0]
            .selected_platforms(&["line-main".to_string(), "qq".to_string()])
            .unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].display_name(), "line-prod");
        assert_eq!(selected[1].display_name(), "qq-local");
    }
}
