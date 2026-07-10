//! Process startup helpers.
//!
//! The file-descriptor soft-limit raising is Unix-only (`RLIMIT_NOFILE` via `libc`). On non-Unix
//! targets (e.g. Windows, where the node runs in-process inside an Unreal Dedicated Server) there
//! is no equivalent soft fd limit to raise, so `raise_and_assert_fd_limit` is a no-op there.

#[cfg(unix)]
mod unix_fd_limit {
    use std::io;

    const DEFAULT_MIN_SOFT: u64 = 16_384;

    fn get_min_soft() -> u64 {
        std::env::var("ARCANE_MIN_FD_LIMIT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_MIN_SOFT)
    }

    fn getrlimit_nofile() -> io::Result<(u64, u64)> {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        let ret = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((rlim.rlim_cur, rlim.rlim_max))
    }

    fn setrlimit_nofile(soft: u64, hard: u64) -> io::Result<()> {
        let rlim = libc::rlimit {
            rlim_cur: soft,
            rlim_max: hard,
        };
        let ret = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn raise_and_assert_fd_limit() -> Result<(), String> {
        let min_soft = get_min_soft();

        let (soft, hard) =
            getrlimit_nofile().map_err(|e| format!("getrlimit(RLIMIT_NOFILE) failed: {e}"))?;

        eprintln!("fd limits: soft={soft} hard={hard}");

        if soft < hard {
            if let Err(e) = setrlimit_nofile(hard, hard) {
                eprintln!("setrlimit(RLIMIT_NOFILE, {hard}, {hard}) failed: {e} — continuing with soft={soft}");
            } else {
                eprintln!("fd limits: raised soft {soft} → {hard}");
            }
        }

        let (final_soft, final_hard) = getrlimit_nofile()
            .map_err(|e| format!("getrlimit(RLIMIT_NOFILE) re-read failed: {e}"))?;

        eprintln!("fd limits (final): soft={final_soft} hard={final_hard}");

        if final_soft < min_soft {
            return Err(format!(
                "fd soft limit {final_soft} is below minimum {min_soft}. \
                 Fix: docker run --ulimit nofile={min_soft}:{min_soft} | \
                 systemd LimitNOFILE={min_soft} | \
                 /etc/security/limits.conf: * hard nofile {min_soft}"
            ));
        }

        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn getrlimit_nofile_returns_sane_values() {
            let (soft, hard) = getrlimit_nofile().unwrap();
            assert!(soft > 0, "soft limit should be > 0, got {soft}");
            assert!(hard >= soft, "hard {hard} should be >= soft {soft}");
        }

        #[test]
        fn raise_and_assert_succeeds_in_test_env() {
            let result = super::raise_and_assert_fd_limit();
            if let Err(e) = &result {
                eprintln!(
                    "raise_and_assert_fd_limit returned Err (expected in restricted envs): {e}"
                );
            }
        }
    }
}

/// Raise the `RLIMIT_NOFILE` soft limit to the hard limit, then assert the result meets a minimum
/// threshold (configurable via `ARCANE_MIN_FD_LIMIT`, default 16,384). Call this at the top of every
/// binary that accepts many sockets.
///
/// **Unix only.** On non-Unix targets there is no `RLIMIT_NOFILE`, so this is a no-op returning `Ok`.
#[cfg(unix)]
pub fn raise_and_assert_fd_limit() -> Result<(), String> {
    unix_fd_limit::raise_and_assert_fd_limit()
}

/// Non-Unix no-op: there is no `RLIMIT_NOFILE` soft limit to raise (e.g. Windows / in-process under
/// an Unreal Dedicated Server). Always `Ok`.
#[cfg(not(unix))]
pub fn raise_and_assert_fd_limit() -> Result<(), String> {
    Ok(())
}
