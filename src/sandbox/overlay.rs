use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct OverlaySandbox {
    #[allow(dead_code)]
    lowerdir: PathBuf,
    #[allow(dead_code)]
    upperdir: TempDir,
    #[allow(dead_code)]
    workdir: TempDir,
    #[allow(dead_code)]
    mount_point: Option<PathBuf>,
    active: bool,
}

impl OverlaySandbox {
    pub fn new() -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;

        #[cfg(target_os = "linux")]
        {
            let upperdir = tempfile::tempdir()?;
            let workdir = tempfile::tempdir()?;
            let mount_point = tempfile::tempdir()?;
            let mount_path = mount_point.path().to_path_buf();

            Self::mount_overlay(&cwd, upperdir.path(), workdir.path(), &mount_path)?;

            Ok(Self {
                lowerdir: cwd,
                upperdir,
                workdir,
                mount_point: Some(mount_path),
                active: true,
            })
        }

        #[cfg(not(target_os = "linux"))]
        {
            Ok(Self {
                lowerdir: cwd,
                upperdir: tempfile::tempdir()?,
                workdir: tempfile::tempdir()?,
                mount_point: None,
                active: false,
            })
        }
    }

    #[cfg(target_os = "linux")]
    fn mount_overlay(
        lower: &Path,
        upper: &Path,
        work: &Path,
        target: &Path,
    ) -> anyhow::Result<()> {
        use std::process::Command;

        let opts = format!(
            "lowerdir={},upperdir={},workdir={}",
            lower.display(),
            upper.display(),
            work.display()
        );

        let output = Command::new("mount")
            .args([
                "-t",
                "overlay",
                "overlay",
                "-o",
                &opts,
                &target.display().to_string(),
            ])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("must be superuser") {
                anyhow::bail!("mount requires root privileges: {}", stderr.trim());
            }
            anyhow::bail!(
                "Failed to mount overlay at {}: {}",
                target.display(),
                stderr.trim()
            );
        }
        Ok(())
    }

    pub fn run<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce() -> anyhow::Result<T>,
    {
        if self.active {
            if let Some(ref mount) = self.mount_point {
                let _guard = CwdGuard::enter(mount)?;
                return f();
            }
        }
        f()
    }

    pub fn upper_dir(&self) -> &Path {
        self.upperdir.path()
    }
}

impl Drop for OverlaySandbox {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(ref mount) = self.mount_point {
            let _ = std::process::Command::new("umount").arg(mount).status();
        }
    }
}

struct CwdGuard {
    original: PathBuf,
}

impl CwdGuard {
    fn enter(path: &Path) -> anyhow::Result<Self> {
        let original = std::env::current_dir()?;
        std::env::set_current_dir(path)?;
        Ok(Self { original })
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn overlay_mount_writable_layer() -> anyhow::Result<()> {
        let sandbox = match OverlaySandbox::new() {
            Ok(s) => s,
            Err(e) => {
                let msg = format!("{}", e);
                if msg.contains("must be superuser") || msg.contains("Permission denied") {
                    return Ok(()); // Requires root, skip gracefully
                }
                return Err(e);
            }
        };
        sandbox.run(|| {
            std::fs::write("test.txt", "hello overlay")?;
            Ok(())
        })?;
        assert!(sandbox.upper_dir().join("test.txt").exists());
        Ok(())
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn overlay_noop_on_non_linux() -> anyhow::Result<()> {
        let sandbox = OverlaySandbox::new()?;
        sandbox.run(|| Ok(()))?;
        Ok(())
    }
}
