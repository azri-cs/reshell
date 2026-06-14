//! Landlock LSM sandbox for Linux (kernel >= 5.13).
//!
//! Uses Landlock to restrict file system access for a command.
//! Landlock allows unprivileged processes to create sandboxes.

use std::path::{Path, PathBuf};

use super::{Sandbox, SandboxContext, NetworkPolicy, NoopContext};

#[allow(dead_code)]
/// Landlock-based filesystem sandbox.
pub struct LandlockSandbox {
    allowed_read: Vec<PathBuf>,
    allowed_write: Vec<PathBuf>,
}

impl LandlockSandbox {
    pub fn new() -> anyhow::Result<Self> {
        // Check that Landlock is available on this kernel.
        Self::check_landlock_available()?;

        // Default: allow read access to common system paths and read/write to CWD.
        let allowed_read = vec![
            PathBuf::from("/usr"),
            PathBuf::from("/bin"),
            PathBuf::from("/lib"),
            PathBuf::from("/lib64"),
            PathBuf::from("/etc"),
            PathBuf::from("/tmp"),
        ];
        let allowed_write = vec![
            PathBuf::from("/tmp"),
        ];

        Ok(Self {
            allowed_read,
            allowed_write,
        })
    }

    fn check_landlock_available() -> anyhow::Result<()> {
        // Check that the Landlock LSM is enabled by trying to access /proc/sys/kernel/landlock/.
        // This is a best-effort check; if Landlock is not available we fall back to a warning.
        let available = Path::new("/proc/sys/kernel/landlock").exists();
        if !available {
            anyhow::bail!("Landlock is not available on this system (kernel < 5.13 or LSM not enabled)");
        }
        Ok(())
    }
}

impl Sandbox for LandlockSandbox {
    fn prepare(&self, cwd: &Path) -> anyhow::Result<Box<dyn SandboxContext>> {
        // TODO: Apply Landlock rules using the `landlock` crate or raw syscalls.
        // For now, this is a no-op context that records the CWD.
        let _ = cwd;
        Ok(Box::new(NoopContext { cwd: cwd.to_path_buf() }))
    }

    fn network_policy(&self) -> NetworkPolicy {
        // Landlock only controls filesystem access; network is unrestricted by default.
        NetworkPolicy::Inherit
    }

    fn name(&self) -> &'static str {
        "landlock"
    }
}
