//! Linux seccomp sandbox (opt-in, requires `seccomp` feature).
//!
//! When enabled via `~/.reshell/config.toml` (`[sandbox] seccomp = true`),
//! restricts the child process's system calls to a safe subset.
//!
//! Non-Linux platforms and builds without the `seccomp` feature gracefully
//! degrade to a no-op with a warning.

/// Apply a seccomp filter to the current thread/process.
/// Must be called in the child process before exec.
pub fn apply_seccomp_filter() -> Result<(), String> {
    let result = apply_seccomp_inner();
    if let Err(e) = &result {
        crate::config::warn(&format!("seccomp filter failed: {}", e));
    }
    result
}

fn apply_seccomp_inner() -> Result<(), String> {
    #[cfg(all(target_os = "linux", feature = "seccomp"))]
    {
        apply_linux_seccomp()
    }
    #[cfg(not(all(target_os = "linux", feature = "seccomp")))]
    {
        Err(
            "seccomp sandbox not available: requires Linux and the `seccomp` cargo feature"
                .to_string(),
        )
    }
}

/// Check if seccomp is available on this system.
pub fn is_seccomp_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/proc/sys/kernel/seccomp/actions_avail").exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(all(target_os = "linux", feature = "seccomp"))]
fn apply_linux_seccomp() -> Result<(), String> {
    use libseccomp::{ScmpAction, ScmpFilterContext, ScmpSyscall};

    // Create a filter that allows most syscalls by default,
    // then blocks the dangerous ones.
    let mut filter = ScmpFilterContext::new_filter(ScmpAction::Allow)
        .map_err(|e| format!("Failed to create seccomp filter: {}", e))?;

    // Block dangerous syscalls — these are never needed by typical build/CLI tools
    let blocked_syscall_names = [
        "ptrace",
        "mount",
        "umount2",
        "pivot_root",
        "chroot",
        "kexec_load",
        "kexec_file_load",
        "init_module",
        "finit_module",
        "delete_module",
        "create_module",
        "iopl",
        "ioperm",
        "swapon",
        "swapoff",
        "reboot",
        "acct",
        "add_key",
        "request_key",
        "keyctl",
        "bpf",
        "perf_event_open",
        "fanotify_init",
    ];

    for name in &blocked_syscall_names {
        let syscall = ScmpSyscall::from_name(name)
            .map_err(|e| format!("Failed to resolve syscall '{}': {}", name, e))?;

        filter
            .add_rule(ScmpAction::KillProcess, syscall)
            .map_err(|e| format!("Failed to add seccomp rule for {}: {}", name, e))?;
    }

    // Load the filter
    filter
        .load()
        .map_err(|e| format!("Failed to load seccomp filter: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seccomp_availability_is_detectable() {
        let available = is_seccomp_available();
        if cfg!(target_os = "linux") {
            let _ = available; // May be false in Docker without privileges
        } else {
            assert!(!available, "seccomp should not be available on non-Linux");
        }
    }
}
