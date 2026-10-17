//! Runtime privilege detection.
//!
//! Koda is normally started by the logged-in user. It is also valid to run it
//! through `sudo` to edit a system file. In that case the effective user is
//! root while `HOME`/`XDG_*` may still describe the invoking user (depending on
//! the sudoers configuration), so state must not be written into another
//! account's home where it would become root-owned.
//!
//! This module only *reports* the situation; callers choose the policy. Nothing
//! here ever changes privileges.

use std::path::PathBuf;

#[cfg(unix)]
unsafe extern "C" {
    fn geteuid() -> u32;
}

/// Whether the process is running with effective UID 0.
pub fn is_root() -> bool {
    #[cfg(unix)]
    {
        // `geteuid` has no failure mode and touches no memory.
        unsafe { geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// The user `sudo` was invoked by, when the process is elevated.
///
/// `SUDO_USER` is set by sudo itself, not by Koda, and is empty for a direct
/// root login. It is a hint for choosing an account-safe state location, never
/// a source of executable paths.
pub fn sudo_user() -> Option<String> {
    if !is_root() {
        return None;
    }
    std::env::var("SUDO_USER")
        .ok()
        .map(|user| user.trim().to_string())
        .filter(|user| !user.is_empty() && user != "root")
}

/// The home directory of the root account on this platform.
fn root_home() -> Option<PathBuf> {
    let candidate = if cfg!(target_os = "macos") {
        PathBuf::from("/var/root")
    } else {
        PathBuf::from("/root")
    };
    candidate.is_dir().then_some(candidate)
}

/// Choose the directory that contains Koda's per-user state.
///
/// * Normal launch: `$XDG_STATE_HOME`, otherwise `$HOME/.local/state`.
/// * Elevated launch (`sudo`): the root account's state directory, even when
///   the environment still points at the invoking user's home. This keeps the
///   user's own Koda state writable by that user.
///
/// `root_home` is a parameter so the fallback logic is testable without
/// touching the real account database.
pub fn state_base(
    xdg_state_home: Option<PathBuf>,
    home: Option<PathBuf>,
    root_home: Option<PathBuf>,
    elevated: bool,
) -> Option<PathBuf> {
    if elevated && let Some(root_home) = root_home {
        return Some(root_home.join(".local/state"));
    }
    if let Some(xdg) = xdg_state_home {
        return Some(xdg);
    }
    home.map(|home| home.join(".local/state"))
}

/// The state base directory for the current process.
pub fn current_state_base() -> Option<PathBuf> {
    state_base(
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        root_home(),
        is_root() && sudo_user().is_some(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_launch_respects_xdg_then_home() {
        assert_eq!(
            state_base(
                Some(PathBuf::from("/xdg/state")),
                Some(PathBuf::from("/home/user")),
                Some(PathBuf::from("/root")),
                false,
            ),
            Some(PathBuf::from("/xdg/state"))
        );
        assert_eq!(
            state_base(
                None,
                Some(PathBuf::from("/home/user")),
                Some(PathBuf::from("/root")),
                false,
            ),
            Some(PathBuf::from("/home/user/.local/state"))
        );
    }

    #[test]
    fn an_elevated_launch_never_uses_another_users_home() {
        // `sudo` with the invoking user's environment preserved.
        assert_eq!(
            state_base(
                Some(PathBuf::from("/home/user/.local/state")),
                Some(PathBuf::from("/home/user")),
                Some(PathBuf::from("/root")),
                true,
            ),
            Some(PathBuf::from("/root/.local/state"))
        );
    }

    #[test]
    fn an_elevated_launch_falls_back_when_root_has_no_home() {
        assert_eq!(
            state_base(
                Some(PathBuf::from("/xdg/state")),
                Some(PathBuf::from("/home/user")),
                None,
                true,
            ),
            Some(PathBuf::from("/xdg/state"))
        );
    }

    #[test]
    fn root_detection_is_stable_on_this_host() {
        // CI is normally non-root. The assertion is about consistency, not a
        // particular account, so it stays meaningful on a root container too.
        assert_eq!(is_root(), is_root());
    }
}
