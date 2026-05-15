use crate::core::{Agent, Platform};
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::sync::Arc;

pub type AgentFactory =
    Arc<dyn Fn(toml::value::Table) -> Result<Arc<dyn Agent>> + Send + Sync + 'static>;
pub type PlatformFactory =
    Arc<dyn Fn(toml::value::Table) -> Result<Arc<dyn Platform>> + Send + Sync + 'static>;

#[derive(Default)]
pub struct Registry {
    agents: BTreeMap<String, AgentFactory>,
    platforms: BTreeMap<String, PlatformFactory>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_agent<F>(&mut self, name: impl Into<String>, factory: F)
    where
        F: Fn(toml::value::Table) -> Result<Arc<dyn Agent>> + Send + Sync + 'static,
    {
        self.agents.insert(name.into(), Arc::new(factory));
    }

    pub fn register_platform<F>(&mut self, name: impl Into<String>, factory: F)
    where
        F: Fn(toml::value::Table) -> Result<Arc<dyn Platform>> + Send + Sync + 'static,
    {
        self.platforms.insert(name.into(), Arc::new(factory));
    }

    pub fn create_agent(&self, name: &str, opts: toml::value::Table) -> Result<Arc<dyn Agent>> {
        let Some(factory) = self.agents.get(name) else {
            return Err(anyhow!(
                "unknown agent `{name}`, available: {:?}",
                self.list_agents()
            ));
        };
        factory(opts)
    }

    pub fn create_platform(
        &self,
        name: &str,
        opts: toml::value::Table,
    ) -> Result<Arc<dyn Platform>> {
        let Some(factory) = self.platforms.get(name) else {
            return Err(anyhow!(
                "unknown platform `{name}`, available: {:?}",
                self.list_platforms()
            ));
        };
        factory(opts)
    }

    pub fn list_agents(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    pub fn list_platforms(&self) -> Vec<String> {
        self.platforms.keys().cloned().collect()
    }
}
