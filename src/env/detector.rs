use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Detector {
    pub shell: String,
    pub shell_version: Option<String>,
    pub os: String,
    pub platform: String,
    pub path: String,
    pub cwd: String,
    pub user: String,
    pub available_tools: Vec<ToolInfo>,
    pub package_manager: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolInfo {
    pub name: String,
    pub version: Option<String>,
}

static CACHED_DETECTOR: Lazy<RwLock<Option<Arc<Detector>>>> = Lazy::new(|| RwLock::new(None));

impl Detector {
    /// Returns a cached Detector, creating it on first call.
    /// The environment rarely changes within a process lifetime,
    /// so subsequent calls return the same instance.
    pub async fn cached() -> Detector {
        let guard = CACHED_DETECTOR.read().await;
        if let Some(detector) = guard.as_ref() {
            return Detector::clone(detector);
        }
        drop(guard);

        let detector = Arc::new(Self::new().await.unwrap_or_default());
        *CACHED_DETECTOR.write().await = Some(detector.clone());
        Detector::clone(&detector)
    }

    /// Invalidate the cached detector. Next call to cached() will re-detect.
    pub async fn invalidate_cache() {
        *CACHED_DETECTOR.write().await = None;
    }

    pub async fn new() -> anyhow::Result<Self> {
        // OS and platform via Rust stdlib (no shell spawn)
        let os = std::env::consts::OS.to_string();
        let platform = std::env::consts::ARCH.to_string();

        // Shell detection and version probes run concurrently
        let (shell, bash_ver, zsh_ver) = tokio::join!(
            Self::run_cmd("sh", &["-c", "echo $SHELL"]),
            Self::run_cmd("bash", &["-c", "echo $BASH_VERSION"]),
            Self::run_cmd("zsh", &["-c", "echo $ZSH_VERSION"]),
        );
        let shell = shell.unwrap_or_else(|_| "/bin/sh".to_string());
        let shell_version = if shell.contains("bash") {
            bash_ver.ok().map(|v| extract_version(&v))
        } else if shell.contains("zsh") {
            zsh_ver.ok().map(|v| extract_version(&v))
        } else {
            None
        };
        let path = std::env::var("PATH").unwrap_or_default();
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        // Parallel tool detection — spawn all version checks concurrently
        let tools_to_check = [
            "git", "node", "docker", "cargo", "npm", "python3", "pip3", "go", "rustc",
        ];
        let mut set = tokio::task::JoinSet::new();
        for &tool in &tools_to_check {
            set.spawn(async move {
                let output = Command::new(tool).arg("--version").output().await;
                (tool.to_string(), output)
            });
        }
        let mut available_tools = Vec::new();
        while let Some(res) = set.join_next().await {
            if let Ok((name, Ok(output))) = res {
                if output.status.success() {
                    available_tools.push(ToolInfo {
                        name,
                        version: Some(extract_version(
                            String::from_utf8_lossy(&output.stdout)
                                .lines()
                                .next()
                                .unwrap_or(""),
                        )),
                    });
                }
            }
        }
        // Sort for deterministic output
        available_tools.sort_by(|a, b| a.name.cmp(&b.name));

        // Parallel package manager detection
        let package_manager = Self::detect_package_manager().await;

        Ok(Self {
            shell,
            shell_version,
            os,
            platform,
            path,
            cwd,
            user,
            available_tools,
            package_manager,
        })
    }

    pub fn suggest_install_command(&self, tool: &str) -> Option<String> {
        match self.package_manager.as_deref() {
            Some("brew") => Some(format!("brew install {}", tool)),
            Some("apt") => Some(format!("sudo apt update && sudo apt install -y {}", tool)),
            Some("yum") => Some(format!("sudo yum install -y {}", tool)),
            Some("dnf") => Some(format!("sudo dnf install -y {}", tool)),
            Some("pacman") => Some(format!("sudo pacman -S --noconfirm {}", tool)),
            Some("choco") => Some(format!("choco install {}", tool)),
            _ => None,
        }
    }

    pub fn execution_shell(&self) -> String {
        "sh".to_string()
    }

    pub fn recovery_shell(&self) -> Option<String> {
        let shell = self.shell.trim();
        if shell.is_empty() || shell == "sh" || shell.ends_with("/sh") {
            None
        } else {
            Some(shell.to_string())
        }
    }

    async fn detect_package_manager() -> Option<String> {
        let mut set = tokio::task::JoinSet::new();
        let managers = [
            ("brew", "brew"),
            ("apt", "apt"),
            ("dnf", "dnf"),
            ("yum", "yum"),
            ("pacman", "pacman"),
            ("choco", "choco"),
        ];
        for (pm, cmd) in &managers {
            let pm = pm.to_string();
            let cmd = cmd.to_string();
            set.spawn(async move {
                let result = Command::new(&cmd).arg("--version").output().await;
                (pm, result.is_ok_and(|o| o.status.success()))
            });
        }

        // Collect all results, return first match in priority order
        let mut found: Vec<(String, bool)> = Vec::new();
        while let Some(res) = set.join_next().await {
            if let Ok(entry) = res {
                found.push(entry);
            }
        }
        // Priority order
        for (pm, _) in &managers {
            if found.iter().any(|(p, ok)| p == pm && *ok) {
                return Some(pm.to_string());
            }
        }
        None
    }

    async fn run_cmd(cmd: &str, args: &[&str]) -> anyhow::Result<String> {
        let output = Command::new(cmd).args(args).output().await?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(anyhow::anyhow!(
                "Command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ))
        }
    }
}

/// Extract a clean version number from raw --version output.
/// Strips common prefixes (e.g. "Docker version 27.3.1, build ce12230" → "27.3.1").
fn extract_version(raw: &str) -> String {
    let version = raw
        .trim()
        .trim_start_matches(|c: char| !c.is_ascii_digit() && c != 'v' && c != 'V')
        .trim_start_matches(|c: char| c == 'v' || c == 'V')
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    if version.is_empty() {
        // If no version-like prefix found, return the original (trimmed)
        raw.trim().to_string()
    } else {
        version
    }
}
