use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

pub fn resolve_work_dir_path(input: &str, base: &str) -> Result<PathBuf> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("work directory path is empty"));
    }
    let expanded = expand_tilde(trimmed)?;
    let path = if expanded.is_absolute() {
        expanded
    } else {
        PathBuf::from(base).join(expanded)
    };
    let canonical = path.canonicalize().unwrap_or(path);
    if !canonical.is_dir() {
        return Err(anyhow!(
            "work directory does not exist: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn expand_tilde(path: &str) -> Result<PathBuf> {
    if path == "~" {
        return home_dir().ok_or_else(|| anyhow!("HOME not set"));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        let home = home_dir().ok_or_else(|| anyhow!("HOME not set"))?;
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(path))
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

pub fn path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
