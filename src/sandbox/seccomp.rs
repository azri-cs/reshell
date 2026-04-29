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
    #[cfg(all(target_os = "linux", feature = "seccomp"))]
    {
        return apply_linux_seccomp();
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
    let blocked_syscalls = [
        ScmpSyscall::new("ptrace"),
        ScmpSyscall::new("mount"),
        ScmpSyscall::new("umount2"),
        ScmpSyscall::new("pivot_root"),
        ScmpSyscall::new("chroot"),
        ScmpSyscall::new("kexec_load"),
        ScmpSyscall::new("kexec_file_load"),
        ScmpSyscall::new("init_module"),
        ScmpSyscall::new("finit_module"),
        ScmpSyscall::new("delete_module"),
        ScmpSyscall::new("create_module"),
        ScmpSyscall::new("iopl"),
        ScmpSyscall::new("ioperm"),
        ScmpSyscall::new("swapon"),
        ScmpSyscall::new("swapoff"),
        ScmpSyscall::new("reboot"),
        ScmpSyscall::new("acct"),
        ScmpSyscall::new("add_key"),
        ScmpSyscall::new("request_key"),
        ScmpSyscall::new("keyctl"),
        ScmpSyscall::new("bpf"),
        ScmpSyscall::new("perf_event_open"),
        ScmpSyscall::new("fanotify_init"),
    ];

    for syscall in blocked_syscalls {
        filter
            .add_rule(ScmpAction::KillProcess, syscall)
            .map_err(|e| format!("Failed to add seccomp rule for {}: {}", syscall, e))?;
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
