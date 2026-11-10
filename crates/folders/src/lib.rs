//! The user folders a desktop app keeps its files in, and opening a folder in
//! the system file manager.
//!
//! Windows answers where `AppData` lives through `SHGetKnownFolderPath`, not
//! through the environment: `%LOCALAPPDATA%` is absent from a service's
//! environment and stale after a profile redirection. Each lookup falls back
//! to the environment when the shell cannot answer. Elsewhere the environment
//! is all there is, and an app's own folder follows the XDG base directory
//! rule.

use std::io;
use std::path::{Path, PathBuf};

/// `%LOCALAPPDATA%`.
#[must_use]
pub fn local_app_data() -> Option<PathBuf> {
    #[cfg(windows)]
    let known = windows::known_folder(&windows::FOLDERID_LocalAppData);
    #[cfg(not(windows))]
    let known = None;
    known.or_else(|| env_path("LOCALAPPDATA"))
}

/// `%APPDATA%`, the roaming application data folder.
#[must_use]
pub fn roaming_app_data() -> Option<PathBuf> {
    #[cfg(windows)]
    let known = windows::known_folder(&windows::FOLDERID_RoamingAppData);
    #[cfg(not(windows))]
    let known = None;
    known.or_else(|| env_path("APPDATA"))
}

/// `%ProgramData%`.
#[must_use]
pub fn program_data() -> Option<PathBuf> {
    #[cfg(windows)]
    let known = windows::known_folder(&windows::FOLDERID_ProgramData);
    #[cfg(not(windows))]
    let known = None;
    known.or_else(|| env_path("ProgramData"))
}

/// `%USERPROFILE%`, the user's home folder.
#[must_use]
pub fn profile() -> Option<PathBuf> {
    #[cfg(windows)]
    let known = windows::known_folder(&windows::FOLDERID_Profile);
    #[cfg(not(windows))]
    let known = None;
    known.or_else(|| env_path("USERPROFILE"))
}

/// `$XDG_DATA_HOME`, or `~/.local/share` when it is unset, empty, or relative.
///
/// The XDG base directory specification says a relative value is invalid and
/// must be ignored.
#[must_use]
pub fn data_home() -> Option<PathBuf> {
    resolve_data_home(env_path("XDG_DATA_HOME"), env_path("HOME").as_deref())
}

fn resolve_data_home(xdg: Option<PathBuf>, home: Option<&Path>) -> Option<PathBuf> {
    xdg.filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(".local/share")))
}

/// The app's own folder, named `name`: under `%LOCALAPPDATA%` on Windows and
/// under [`data_home`] elsewhere.
#[must_use]
pub fn app_data_dir(name: &str) -> Option<PathBuf> {
    let base = if cfg!(windows) {
        local_app_data()
    } else {
        data_home()
    };
    base.map(|base| base.join(name))
}

/// Opens a folder the app owns in the system file manager, creating it first
/// so the file manager never opens on a missing path.
///
/// # Errors
///
/// Returns the error when the folder cannot be created or opened.
pub fn reveal(directory: &Path) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    open(directory)
}

/// Opens an existing folder, such as a game's install folder, in the system
/// file manager. Unlike [`reveal`], it never creates the folder.
///
/// On Windows this asks the shell through `ShellExecuteW`. Spawning
/// `explorer.exe` is unreliable: Explorer hands the path to the running shell
/// and exits, so the launch can fail after `spawn` has already reported
/// success. Call it from the window thread, where winit has already
/// initialized COM for the shell extensions Explorer loads.
///
/// # Errors
///
/// Returns the error when the path contains NUL, or the shell or `xdg-open`
/// refuses it.
pub fn open(directory: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows::open(directory)
    }
    #[cfg(not(windows))]
    {
        let mut child = std::process::Command::new("xdg-open")
            .arg(directory)
            .spawn()?;
        // An unwaited child stays a zombie until the app exits. `xdg-open`
        // hands the folder to the file manager and returns, so waiting on a
        // thread is brief and never blocks the caller.
        let reaper = std::thread::Builder::new()
            .name("xdg-open-reaper".to_owned())
            .spawn(move || match child.wait() {
                Ok(status) if !status.success() => {
                    tracing::warn!(%status, "xdg-open could not open the folder");
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "could not wait for xdg-open"),
            });
        if let Err(error) = reaper {
            tracing::warn!(%error, "could not start a thread to wait for xdg-open");
        }
        Ok(())
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(name)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    pub(super) use windows_sys::Win32::UI::Shell::{
        FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_ProgramData, FOLDERID_RoamingAppData,
    };
    use windows_sys::Win32::UI::Shell::{SHGetKnownFolderPath, ShellExecuteW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows_sys::core::GUID;

    /// Values at or below this mean `ShellExecuteW` failed. Above it, the
    /// return value is a legacy instance handle.
    const SHELL_EXECUTE_MIN_SUCCESS: usize = 32;

    /// Asks Windows for a known folder, returning `None` when the lookup fails
    /// so the caller can fall back to the environment.
    pub(super) fn known_folder(id: &GUID) -> Option<PathBuf> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: `id` is a known-folder GUID, and `raw` receives an allocation
        // the call owns until `CoTaskMemFree` below releases it.
        let status = unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &raw mut raw) };
        if status < 0 || raw.is_null() {
            // SAFETY: on failure `raw` is null or an allocation the caller
            // must still release, and `CoTaskMemFree` accepts null.
            unsafe { CoTaskMemFree(raw.cast()) };
            return None;
        }

        let mut length = 0;
        loop {
            // SAFETY: a successful call returns a NUL-terminated UTF-16
            // string, and the scan stops at the terminator, so `length` stays
            // inside the allocation.
            let unit = unsafe { raw.add(length) };
            // SAFETY: `unit` points inside the allocation, as above.
            if unsafe { *unit } == 0 {
                break;
            }
            length += 1;
        }
        // SAFETY: `length` counts the units before the terminator, all within
        // the allocation.
        let path = String::from_utf16(unsafe { std::slice::from_raw_parts(raw, length) })
            .ok()
            .map(PathBuf::from);
        // SAFETY: `raw` is the allocation returned above. It is no longer read,
        // and this releases it exactly once.
        unsafe { CoTaskMemFree(raw.cast()) };
        path
    }

    pub(super) fn open(directory: &Path) -> io::Result<()> {
        let mut path: Vec<u16> = directory.as_os_str().encode_wide().collect();
        if path.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the folder path contains NUL",
            ));
        }
        path.push(0);
        let verb: Vec<u16> = "open\0".encode_utf16().collect();

        // SAFETY: both strings are NUL-terminated and outlive this synchronous
        // call, and every other argument is the documented null.
        let instance = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        let code = instance as usize;
        if code <= SHELL_EXECUTE_MIN_SUCCESS {
            return Err(io::Error::other(format!(
                "the shell refused to open the folder (code {code})"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_folder_is_named_after_the_app() {
        if let Some(dir) = app_data_dir("some-app") {
            assert!(dir.ends_with("some-app"));
        }
    }

    #[test]
    fn an_empty_variable_is_not_a_path() {
        // SAFETY: no other test reads or writes this variable.
        unsafe { std::env::set_var("APP_FOLDERS_EMPTY_TEST_VAR", "") };
        assert_eq!(env_path("APP_FOLDERS_EMPTY_TEST_VAR"), None);
        // SAFETY: as above.
        unsafe { std::env::remove_var("APP_FOLDERS_EMPTY_TEST_VAR") };
    }

    // `/xdg` is absolute only on Unix, and the rule being tested is the XDG one.
    #[cfg(unix)]
    #[test]
    fn an_absolute_xdg_data_home_wins_over_home() {
        assert_eq!(
            resolve_data_home(Some(PathBuf::from("/xdg")), Some(Path::new("/home/me"))),
            Some(PathBuf::from("/xdg"))
        );
        assert_eq!(
            resolve_data_home(None, Some(Path::new("/home/me"))),
            Some(PathBuf::from("/home/me/.local/share"))
        );
        assert_eq!(resolve_data_home(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_relative_xdg_data_home_is_ignored() {
        assert_eq!(
            resolve_data_home(Some(PathBuf::from("xdg")), Some(Path::new("/home/me"))),
            Some(PathBuf::from("/home/me/.local/share"))
        );
        assert_eq!(resolve_data_home(None, None), None);
    }
}
