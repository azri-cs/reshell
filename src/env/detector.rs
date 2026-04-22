use serde::{Deserialize, Serialize};
use tokio::process::Command;

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

impl Detector {
    pub async fn new() -> anyhow::Result<Self> {
        let shell = Self::run_cmd("sh", &["-c", "echo $SHELL"]).await.unwrap_or_else(|_| "/bin/sh".to_string());
        let shell_version = if shell.contains("bash") {
            Self::run_cmd("bash", &["-c", "echo $BASH_VERSION"]).await.ok()
        } else if shell.contains("zsh") {
            Self::run_cmd("zsh", &["-c", "echo $ZSH_VERSION"]).await.ok()
        } else {
            None
        };

        let os = Self::run_cmd("uname", &["-s"]).await.unwrap_or_else(|_| "Unknown".to_string());
        let platform = Self::run_cmd("uname", &["-m"]).await.unwrap_or_else(|_| "unknown".to_string());
        let path = std::env::var("PATH").unwrap_or_default();
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
        let user = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());

        let tools_to_check = vec!["git", "node", "docker", "cargo", "npm", "python3", "pip3", "go", "rustc"];
        let mut available_tools = Vec::new();
        for tool in tools_to_check {
            if let Ok(version) = Self::run_cmd(tool, &["--version"]).await {
                available_tools.push(ToolInfo {
                    name: tool.to_string(),
                    version: Some(version.lines().next().unwrap_or("").to_string()),
                });
            }
        }

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
        for (pm, check_cmd, args) in [
            ("brew", "brew", &["--version"]),
            ("apt", "apt", &["--version"]),
            ("dnf", "dnf", &["--version"]),
            ("yum", "yum", &["--version"]),
            ("pacman", "pacman", &["--version"]),
            ("choco", "choco", &["--version"]),
        ] {
            if Command::new(check_cmd).args(args).output().await.is_ok() {
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
