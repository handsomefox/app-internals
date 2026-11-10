//! Whether a process with a given executable name is running.
//!
//! Processes are found by name only. On Windows that is one
//! `CreateToolhelp32Snapshot` enumeration; on Linux it is each process's
//! `comm` under `/proc`. Neither opens a handle to any process, so a game
//! running anti-cheat never sees one, and a check needs no extra rights.
//!
//! Names compare without regard to ASCII case, the way Windows treats
//! executable names. Any failure to enumerate reads as "nothing is running":
//! callers use this to skip work or to give advice, never as the only guard
//! in front of something destructive.

use std::collections::HashSet;

/// The executable names running at one moment, lowercased.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Running {
    names: HashSet<String>,
}

impl Running {
    /// Lists every running process once. Checking many names against one
    /// snapshot costs a single enumeration.
    #[must_use]
    pub fn snapshot() -> Self {
        Self {
            names: imp::names()
                .into_iter()
                .map(|name| name.to_ascii_lowercase())
                .collect(),
        }
    }

    /// Whether a process named `name` was running, such as `Discord.exe`.
    ///
    /// Linux keeps only the first 15 bytes of a process name, so a longer
    /// `name` also matches its first 15 bytes there.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        if self.names.contains(&name) {
            return true;
        }
        cfg!(target_os = "linux")
            && name.len() > LINUX_COMM_LEN
            && name
                .get(..LINUX_COMM_LEN)
                .is_some_and(|prefix| self.names.contains(prefix))
    }

    /// Whether nothing was found, which is also the answer when the
    /// enumeration failed or the platform has none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Whether a process named `name` is running right now. Use
/// [`Running::snapshot`] to check several names.
#[must_use]
pub fn is_running(name: &str) -> bool {
    Running::snapshot().contains(name)
}

/// How many bytes of a process name Linux keeps in `comm`.
const LINUX_COMM_LEN: usize = 15;

#[cfg(windows)]
mod imp {
    use std::mem::{size_of, zeroed};

    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    pub(super) fn names() -> Vec<String> {
        // SAFETY: takes a process snapshot. It reads no process memory and
        // opens no process handle; an invalid handle is checked below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }

        // SAFETY: PROCESSENTRY32W is plain data, for which all zeroes is valid.
        let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
        entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or_default();

        let mut names = Vec::new();
        // SAFETY: `snapshot` is a live Toolhelp handle and `entry` is a
        // correctly sized PROCESSENTRY32W.
        let mut more = unsafe { Process32FirstW(snapshot, &raw mut entry) } != 0;
        while more {
            let end = entry
                .szExeFile
                .iter()
                .position(|&unit| unit == 0)
                .unwrap_or(entry.szExeFile.len());
            names.push(String::from_utf16_lossy(&entry.szExeFile[..end]));
            // SAFETY: as for Process32FirstW above.
            more = unsafe { Process32NextW(snapshot, &raw mut entry) } != 0;
        }

        // SAFETY: `snapshot` is a handle this function opened and closes once.
        unsafe { CloseHandle(snapshot) };
        names
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub(super) fn names() -> Vec<String> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .flatten()
            // Only numbered entries are processes; the rest is kernel state.
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(|c: char| c.is_ascii_digit())
            })
            .filter_map(|entry| std::fs::read_to_string(entry.path().join("comm")).ok())
            .map(|comm| comm.trim_end_matches('\n').to_owned())
            .collect()
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    pub(super) fn names() -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(names: &[&str]) -> Running {
        Running {
            names: names.iter().map(|name| name.to_ascii_lowercase()).collect(),
        }
    }

    #[test]
    fn names_match_without_regard_to_case() {
        let running = running(&["Discord.exe"]);
        assert!(running.contains("discord.exe"));
        assert!(running.contains("DISCORD.EXE"));
        assert!(!running.contains("Discord"));
        assert!(!running.contains("Discord.exe.old"));
    }

    #[test]
    fn an_empty_snapshot_finds_nothing() {
        let running = Running::default();
        assert!(running.is_empty());
        assert!(!running.contains("anything.exe"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_matches_a_long_name_by_its_kept_prefix() {
        let running = running(&["a-very-long-pro"]);
        assert!(running.contains("a-very-long-process-name"));
        assert!(!running.contains("a-very-long-pr"));
    }

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn this_test_process_is_found_by_its_own_name() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_str().unwrap();
        assert!(
            Running::snapshot().contains(name),
            "enumeration missed this process, named {name:?}"
        );
    }
}
