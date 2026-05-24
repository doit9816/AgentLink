use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

static SEARCH_PATHS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// Paths used when spawning CLI agents from a GUI-launched bridge (minimal default PATH).
pub fn executable_search_paths() -> &'static [PathBuf] {
    SEARCH_PATHS.get_or_init(build_search_paths)
}

pub fn enriched_path_var() -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    executable_search_paths()
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(separator)
}

pub fn resolve_executable(command: &str) -> anyhow::Result<PathBuf> {
    let command = command.trim();
    if command.is_empty() {
        anyhow::bail!("agent command is empty");
    }
    let path = Path::new(command);
    if path.components().count() > 1 || command.contains('\\') || command.contains('/') {
        if is_executable_file(path) {
            return Ok(path.to_path_buf());
        }
        anyhow::bail!("executable not found: {command}");
    }
    for dir in executable_search_paths() {
        for name in executable_names(command) {
            let candidate = dir.join(&name);
            if is_executable_file(&candidate) {
                return Ok(candidate);
            }
        }
    }
    anyhow::bail!(
        "executable `{command}` not found in PATH (install it or set an absolute path such as codex_bin in agentlink.toml)"
    )
}

pub fn apply_enriched_path(command: &mut Command) {
    command.env("PATH", enriched_path_var());
}

pub fn apply_enriched_path_tokio(command: &mut tokio::process::Command) {
    command.env("PATH", enriched_path_var());
}

fn build_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |path: PathBuf| {
        if path.as_os_str().is_empty() {
            return;
        }
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            paths.push(path);
        }
    };

    for path in login_shell_path_entries() {
        push(path);
    }
    #[cfg(target_os = "macos")]
    for path in macos_path_helper_entries() {
        push(path);
    }
    for path in parse_path_env_var() {
        push(path);
    }
    for path in standard_extra_path_entries() {
        push(path);
    }
    paths
}

fn parse_path_env_var() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|value| parse_path_list(&value.to_string_lossy()))
        .unwrap_or_default()
}

fn parse_path_list(raw: &str) -> Vec<PathBuf> {
    let separator = if cfg!(windows) { ';' } else { ':' };
    raw.split(separator)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn login_shell_path_entries() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let output = {
            let mut command = Command::new("powershell");
            command.args([
                "-NoProfile",
                "-Command",
                "[Environment]::GetEnvironmentVariable('PATH','User') + ';' + [Environment]::GetEnvironmentVariable('PATH','Machine')",
            ]);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x08000000;
                command.creation_flags(CREATE_NO_WINDOW);
            }
            command.output()
        };
        return output
            .ok()
            .filter(|value| value.status.success())
            .map(|value| parse_path_list(&String::from_utf8_lossy(&value.stdout)))
            .unwrap_or_default();
    }

    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let Some(output) = Command::new(&shell)
            .args(["-ilc", "printf %s \"$PATH\""])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()
        else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }
        parse_path_list(&String::from_utf8_lossy(&output.stdout))
    }
}

#[cfg(target_os = "macos")]
fn macos_path_helper_entries() -> Vec<PathBuf> {
    let Some(output) = Command::new("/usr/libexec/path_helper")
        .arg("-s")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Some(start) = text.find("PATH=\"") else {
        return Vec::new();
    };
    let rest = &text[start + 6..];
    let Some(end) = rest.find('"') else {
        return Vec::new();
    };
    parse_path_list(&rest[..end])
}

fn standard_extra_path_entries() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if cfg!(windows) {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let home = PathBuf::from(profile);
            paths.push(home.join(".cargo").join("bin"));
            paths.push(home.join("AppData").join("Roaming").join("npm"));
        }
        return paths;
    }

    paths.push(PathBuf::from("/opt/homebrew/bin"));
    paths.push(PathBuf::from("/usr/local/bin"));
    paths.push(PathBuf::from("/usr/bin"));
    paths.push(PathBuf::from("/bin"));
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".local").join("bin"));
        paths.push(home.join(".cargo").join("bin"));
        paths.push(home.join(".npm-global").join("bin"));
        paths.push(home.join(".volta").join("bin"));
        paths.push(
            home.join(".fnm")
                .join("aliases")
                .join("default")
                .join("bin"),
        );
    }
    paths
}

fn executable_names(base: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![format!("{base}.exe"), base.to_string()]
    } else {
        vec![base.to_string()]
    }
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return path
            .metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enriched_path_includes_homebrew_on_macos() {
        let paths = standard_extra_path_entries();
        assert!(paths.iter().any(|path| path.ends_with("homebrew/bin")));
    }
}
