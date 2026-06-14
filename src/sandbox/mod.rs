pub mod allowlist;
pub mod overlay;
pub mod paths;
pub mod scrubber;
pub mod seccomp;

use std::path::{Path, PathBuf};

/// Network isolation policy for sandboxed execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// Inherit the host network (default, no isolation).
    Inherit,
    /// Allow only localhost connections.
    LocalhostOnly,
    /// Block all network access.
    None,
}

/// A sandbox strategy for isolating command execution.
pub trait Sandbox: Send + Sync {
    /// Prepare the sandbox environment (e.g. mount overlay, set up namespaces).
    /// Returns a context handle that stays alive for the duration of execution.
    fn prepare(&self, cwd: &Path) -> anyhow::Result<Box<dyn SandboxContext>>;

    /// The network policy enforced by this sandbox.
    fn network_policy(&self) -> NetworkPolicy;

    /// Human-readable name for this sandbox strategy.
    fn name(&self) -> &'static str;
}

/// A handle that keeps the sandbox active. Dropping it cleans up resources.
pub trait SandboxContext: Send + Sync {
    /// Get the effective working directory inside the sandbox.
    fn work_dir(&self) -> &Path;
}

/// No-op sandbox that runs commands without isolation.
pub struct NoopSandbox;

impl Sandbox for NoopSandbox {
    fn prepare(&self, cwd: &Path) -> anyhow::Result<Box<dyn SandboxContext>> {
        let cwd = cwd.to_path_buf();
        Ok(Box::new(NoopContext { cwd }))
    }

    fn network_policy(&self) -> NetworkPolicy {
        NetworkPolicy::Inherit
    }

    fn name(&self) -> &'static str {
        "none"
    }
}

struct NoopContext {
    cwd: PathBuf,
}

impl SandboxContext for NoopContext {
    fn work_dir(&self) -> &Path {
        &self.cwd
    }
}

/// Docker-based sandbox (requires Docker daemon).
#[cfg(any())]
pub mod docker;

/// Landlock-based sandbox (Linux kernel >= 5.13).
#[cfg(target_os = "linux")]
pub mod landlock;

/// Dispatch sandbox by mode string.
pub fn create_sandbox(mode: &str) -> anyhow::Result<Box<dyn Sandbox>> {
    match mode {
        "none" => Ok(Box::new(NoopSandbox)),
        "overlay" => Ok(Box::new(overlay::OverlaySandbox::new()?)),
        #[cfg(target_os = "linux")]
        "landlock" => {
            let sb = landlock::LandlockSandbox::new()?;
            Ok(Box::new(sb))
        }
        #[cfg(any())]
        "docker" => {
            let sb = docker::DockerSandbox::new()?;
            Ok(Box::new(sb))
        }
        _ => anyhow::bail!("Unknown sandbox mode: {}. Use 'none', 'overlay', 'landlock', or 'docker'.", mode),
    }
}
