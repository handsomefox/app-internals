//! Starting a program with administrator rights through the UAC prompt.
//!
//! The apps run unelevated, with an `asInvoker` manifest. When one needs
//! administrator rights, it starts a program, often itself, through
//! `ShellExecuteExW` with the `runas` verb, and Windows asks the user. A
//! [`Command`] describes that program, and [`Command::spawn`] returns a
//! [`Child`] that owns the new process's handle, so the app can wait for it and
//! read its exit code. A declined prompt is [`Error::Cancelled`], apart from
//! every real failure.
//!
//! A process without administrator rights cannot signal or end an elevated
//! one, so a [`Child`] can be waited on but not killed.
//!
//! ```no_run
//! # fn main() -> Result<(), app_elevation::Error> {
//! let mut command = app_elevation::Command::new(std::env::current_exe()?);
//! command.arg("--elevated-helper").arg(r"C:\Users\me\AppData\Local\plan.json");
//! match command.spawn() {
//!     Ok(child) => println!("helper exited with {}", child.wait()?),
//!     Err(app_elevation::Error::Cancelled) => println!("the user said no"),
//!     Err(error) => return Err(error),
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Off Windows, [`Command::spawn`] returns [`Error::Unsupported`] and
//! [`is_elevated`] returns `false`. The crate still builds, so portable code
//! can name its types.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Why starting or waiting for an elevated program failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The user declined the UAC prompt. This is a normal answer, not a
    /// failure.
    #[error("the administrator prompt was declined")]
    Cancelled,
    /// Administrator rights exist only on Windows.
    #[error("running as administrator is only available on Windows")]
    Unsupported,
    /// The program, an argument, or the folder holds a NUL character.
    #[error("the {0} contains a NUL character")]
    Nul(&'static str),
    /// The shell reported success but returned no process to wait for.
    #[error("Windows started no process to wait for")]
    NoProcess,
    /// Windows refused the launch or a wait.
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// How the new program's window first appears.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Show {
    /// As usual, `SW_SHOWNORMAL`.
    #[default]
    Normal,
    /// Hidden, `SW_HIDE`, for a console tool whose output is redirected.
    Hidden,
}

/// A program to start with administrator rights.
#[derive(Clone, Debug)]
pub struct Command {
    program: OsString,
    /// The command line after the program, as UTF-16 without a terminator.
    parameters: Vec<u16>,
    directory: Option<PathBuf>,
    show: Show,
}

impl Command {
    /// A command that starts `program`, a path or a name the shell finds.
    #[must_use]
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            parameters: Vec::new(),
            directory: None,
            show: Show::Normal,
        }
    }

    /// Adds one argument, quoted so the program's C runtime or
    /// `CommandLineToArgvW` reads it back unchanged.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.separate();
        append_quoted(&mut self.parameters, &wide(arg.as_ref()));
        self
    }

    /// Adds each of `args`, as [`Command::arg`] does.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.arg(arg);
        }
        self
    }

    /// Adds `text` to the command line as it is, after a space. Use it for a
    /// program with its own quoting rules, such as `cmd.exe /c`.
    pub fn raw_arg(&mut self, text: impl AsRef<OsStr>) -> &mut Self {
        self.separate();
        self.parameters.extend(wide(text.as_ref()));
        self
    }

    /// Starts the program in `directory`. Without it, the program starts in
    /// the shell's default folder, not this process's.
    pub fn current_dir(&mut self, directory: impl AsRef<Path>) -> &mut Self {
        self.directory = Some(directory.as_ref().to_owned());
        self
    }

    /// Sets how the program's window first appears.
    pub fn show(&mut self, show: Show) -> &mut Self {
        self.show = show;
        self
    }

    /// The program this starts.
    #[must_use]
    pub fn get_program(&self) -> &OsStr {
        &self.program
    }

    /// The folder the program starts in, if one was set.
    #[must_use]
    pub fn get_current_dir(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    /// The command line after the program, for logs and tests.
    #[must_use]
    pub fn get_parameters(&self) -> String {
        String::from_utf16_lossy(&self.parameters)
    }

    /// Asks the user for administrator rights and starts the program. This
    /// blocks until the user answers the prompt.
    ///
    /// It initializes COM on the calling thread for the call, as the shell
    /// requires, unless the thread already has an apartment.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Cancelled`] when the user declines, [`Error::Nul`] for
    /// a string Windows cannot take, [`Error::Unsupported`] off Windows, and
    /// [`Error::Io`] or [`Error::NoProcess`] when Windows refuses.
    pub fn spawn(&self) -> Result<Child, Error> {
        #[cfg(windows)]
        {
            windows::spawn(self)
        }
        #[cfg(not(windows))]
        {
            Err(Error::Unsupported)
        }
    }

    fn separate(&mut self) {
        if !self.parameters.is_empty() {
            self.parameters.push(u16::from(b' '));
        }
    }
}

/// A running program started by [`Command::spawn`]. Dropping it closes the
/// handle; the program keeps running.
#[derive(Debug)]
pub struct Child {
    #[cfg(windows)]
    process: std::os::windows::io::OwnedHandle,
    #[cfg(not(windows))]
    never: std::convert::Infallible,
}

impl Child {
    /// The program's process id.
    #[must_use]
    pub fn id(&self) -> u32 {
        #[cfg(windows)]
        {
            windows::id(self)
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }

    /// The exit code if the program has ended, without waiting.
    ///
    /// # Errors
    ///
    /// Returns the error when Windows cannot wait on the process or read its
    /// exit code.
    pub fn try_wait(&self) -> Result<Option<u32>, Error> {
        self.wait_timeout(Duration::ZERO)
    }

    /// Waits as long as it takes for the program to end, and returns its
    /// exit code.
    ///
    /// # Errors
    ///
    /// As [`Child::try_wait`].
    pub fn wait(&self) -> Result<u32, Error> {
        #[cfg(windows)]
        {
            windows::wait(self, None).map(Option::unwrap_or_default)
        }
        #[cfg(not(windows))]
        {
            match self.never {}
        }
    }

    /// Waits up to `timeout` for the program to end, and returns its exit
    /// code, or `None` while it still runs. Waits longer than about 49 days
    /// are cut to that.
    ///
    /// # Errors
    ///
    /// As [`Child::try_wait`].
    pub fn wait_timeout(&self, timeout: Duration) -> Result<Option<u32>, Error> {
        #[cfg(windows)]
        {
            windows::wait(self, Some(timeout))
        }
        #[cfg(not(windows))]
        {
            let _ = timeout;
            match self.never {}
        }
    }
}

/// Whether this process runs with administrator rights. Always `false` off
/// Windows.
///
/// # Errors
///
/// Returns the error when Windows cannot read this process's token.
pub fn is_elevated() -> io::Result<bool> {
    #[cfg(windows)]
    {
        windows::is_elevated()
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

#[cfg(windows)]
pub use windows::ComApartment;

fn wide(text: &OsStr) -> Vec<u16> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        text.encode_wide().collect()
    }
    #[cfg(not(windows))]
    {
        text.to_string_lossy().encode_utf16().collect()
    }
}

/// Appends `arg` as the Microsoft C runtime and `CommandLineToArgvW` parse
/// it: quoted when empty or holding a space or tab, with each `"` escaped
/// and the backslashes before a `"` doubled.
fn append_quoted(line: &mut Vec<u16>, arg: &[u16]) {
    const QUOTE: u16 = b'"' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let quote = arg.is_empty()
        || arg
            .iter()
            .any(|&unit| unit == u16::from(b' ') || unit == u16::from(b'\t'));
    if quote {
        line.push(QUOTE);
    }
    let mut backslashes = 0;
    for &unit in arg {
        if unit == BACKSLASH {
            backslashes += 1;
        } else {
            if unit == QUOTE {
                // Each backslash before a quote needs its own escape, and the
                // quote needs one too.
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes + 1));
            }
            backslashes = 0;
        }
        line.push(unit);
    }
    if quote {
        // Backslashes before the closing quote would escape it.
        line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        line.push(QUOTE);
    }
}

#[cfg(windows)]
mod windows {
    use std::io;
    use std::marker::PhantomData;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        ERROR_CANCELLED, HANDLE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, GetProcessId, INFINITE, OpenProcessToken,
        WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

    use super::{Child, Command, Error, Show, wide};

    /// COM initialized on the calling thread, as a single-threaded apartment
    /// without OLE1 DDE, until the guard drops.
    ///
    /// `ShellExecute` can hand a file to a shell extension that is a COM
    /// object, so Microsoft asks callers to initialize COM first. A thread
    /// that already chose an apartment keeps it, and the guard then does
    /// nothing. It must drop on the thread that made it, so it is neither
    /// `Send` nor `Sync`.
    pub struct ComApartment {
        /// Whether `CoInitializeEx` succeeded, and so must be balanced.
        initialized: bool,
        _thread_bound: PhantomData<*const ()>,
    }

    impl ComApartment {
        /// Initializes COM on this thread unless it already has an
        /// apartment.
        #[must_use]
        pub fn enter() -> Self {
            // SAFETY: no reserved pointer; the result is balanced in `Drop`
            // only when it succeeded.
            let result = unsafe {
                CoInitializeEx(
                    std::ptr::null(),
                    (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).cast_unsigned(),
                )
            };
            Self {
                initialized: result >= 0,
                _thread_bound: PhantomData,
            }
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            if self.initialized {
                // SAFETY: balances the successful `CoInitializeEx` in `enter`
                // on the same thread, which `!Send` guarantees.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// `text` with a NUL terminator, or an error naming `what` when it holds
    /// a NUL already.
    fn terminated(mut text: Vec<u16>, what: &'static str) -> Result<Vec<u16>, Error> {
        if text.contains(&0) {
            return Err(Error::Nul(what));
        }
        text.push(0);
        Ok(text)
    }

    pub(super) fn spawn(command: &Command) -> Result<Child, Error> {
        let file = terminated(wide(&command.program), "program")?;
        let parameters = terminated(command.parameters.clone(), "command line")?;
        let directory = command
            .directory
            .as_ref()
            .map(|directory| terminated(wide(directory.as_os_str()), "folder"))
            .transpose()?;
        let verb: Vec<u16> = "runas\0".encode_utf16().collect();

        let mut info = SHELLEXECUTEINFOW {
            cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>()).unwrap_or_default(),
            // NOCLOSEPROCESS returns the process handle. NOASYNC finishes the
            // launch before returning, which a caller about to exit needs.
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: verb.as_ptr(),
            lpFile: file.as_ptr(),
            lpParameters: if command.parameters.is_empty() {
                std::ptr::null()
            } else {
                parameters.as_ptr()
            },
            lpDirectory: directory.as_ref().map_or(std::ptr::null(), Vec::as_ptr),
            nShow: match command.show {
                Show::Normal => SW_SHOWNORMAL,
                Show::Hidden => SW_HIDE,
            },
            ..Default::default()
        };

        let com = ComApartment::enter();
        // SAFETY: `info` is initialized, and its strings are NUL-terminated
        // and outlive the call.
        let launched = unsafe { ShellExecuteExW(&raw mut info) };
        // Read before anything else can overwrite the thread's last error.
        let error = io::Error::last_os_error();
        drop(com);
        if launched == 0 {
            if error.raw_os_error() == i32::try_from(ERROR_CANCELLED).ok() {
                return Err(Error::Cancelled);
            }
            return Err(Error::Io(error));
        }
        if info.hProcess.is_null() {
            return Err(Error::NoProcess);
        }
        // SAFETY: with SEE_MASK_NOCLOSEPROCESS the caller owns the returned
        // process handle, and nothing else closes it.
        let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess) };
        Ok(Child { process })
    }

    pub(super) fn id(child: &Child) -> u32 {
        // SAFETY: `process` is a valid process handle.
        unsafe { GetProcessId(child.process.as_raw_handle()) }
    }

    /// Waits up to `timeout`, or without limit for `None`.
    pub(super) fn wait(child: &Child, timeout: Option<Duration>) -> Result<Option<u32>, Error> {
        let process: HANDLE = child.process.as_raw_handle();
        let millis = timeout.map_or(INFINITE, |timeout| {
            u32::try_from(timeout.as_millis())
                .map_or(INFINITE - 1, |millis| millis.min(INFINITE - 1))
        });
        // SAFETY: `process` is a valid process handle with SYNCHRONIZE
        // access, as ShellExecuteEx returns it.
        let waited = unsafe { WaitForSingleObject(process, millis) };
        match waited {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Ok(None),
            WAIT_FAILED => return Err(Error::Io(io::Error::last_os_error())),
            other => {
                return Err(Error::Io(io::Error::other(format!(
                    "unexpected wait result {other:#x}"
                ))));
            }
        }
        let mut code = 0;
        // SAFETY: `process` is valid, and `code` is a valid out-pointer.
        if unsafe { GetExitCodeProcess(process, &raw mut code) } == 0 {
            return Err(Error::Io(io::Error::last_os_error()));
        }
        Ok(Some(code))
    }

    pub(super) fn is_elevated() -> io::Result<bool> {
        // SAFETY: returns a pseudo-handle that needs no closing.
        let process = unsafe { GetCurrentProcess() };
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: `process` is valid and `token` is a valid out-pointer.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the call succeeded, so `token` is open and only this owns it.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut written = 0;
        // SAFETY: `elevation` is a writable TOKEN_ELEVATION of the size
        // passed, and `written` is a valid out-pointer.
        let read = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenElevation,
                (&raw mut elevation).cast(),
                u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap_or_default(),
                &raw mut written,
            )
        };
        if read == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(elevation.TokenIsElevated != 0)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_token_can_be_read() {
            assert!(is_elevated().is_ok());
        }

        #[test]
        fn a_nul_is_refused_before_any_prompt() {
            assert!(matches!(
                Command::new("a\0b").spawn(),
                Err(Error::Nul("program"))
            ));
            assert!(matches!(
                Command::new("cmd.exe").raw_arg("/c\0").spawn(),
                Err(Error::Nul("command line"))
            ));
            assert!(matches!(
                Command::new("cmd.exe").current_dir("C:\\a\0").spawn(),
                Err(Error::Nul("folder"))
            ));
        }

        #[test]
        fn com_can_be_entered_twice_on_one_thread() {
            let outer = ComApartment::enter();
            let inner = ComApartment::enter();
            drop(inner);
            drop(outer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> String {
        let mut command = Command::new("program.exe");
        command.args(args);
        command.get_parameters()
    }

    #[test]
    fn plain_arguments_are_left_alone() {
        assert_eq!(line(&[]), "");
        assert_eq!(
            line(&["--elevated", "--after=4242"]),
            "--elevated --after=4242"
        );
        assert_eq!(line(&[r"C:\plain\path.json"]), r"C:\plain\path.json");
    }

    #[test]
    fn spaces_tabs_and_empty_arguments_are_quoted() {
        assert_eq!(line(&["two words"]), r#""two words""#);
        assert_eq!(line(&["tab\there"]), "\"tab\there\"");
        assert_eq!(line(&["", "x"]), r#""" x"#);
    }

    #[test]
    fn quotes_and_the_backslashes_before_them_are_escaped() {
        assert_eq!(line(&[r#"say "hi""#]), r#""say \"hi\"""#);
        assert_eq!(line(&[r#"a\"b"#]), r#"a\\\"b"#);
        assert_eq!(line(&[r"a\\b"]), r"a\\b");
    }

    #[test]
    fn a_trailing_backslash_does_not_escape_the_closing_quote() {
        assert_eq!(line(&[r"C:\Program Files\"]), r#""C:\Program Files\\""#);
        assert_eq!(line(&[r"C:\dir\"]), r"C:\dir\");
        assert_eq!(line(&[r"C:\a b\\"]), r#""C:\a b\\\\""#);
    }

    #[test]
    fn raw_text_is_kept_as_written() {
        let mut command = Command::new("cmd.exe");
        command
            .arg("/d")
            .raw_arg(r#"/c "(sfc /scannow) > "C:\out file.txt" 2>&1""#);
        assert_eq!(
            command.get_parameters(),
            r#"/d /c "(sfc /scannow) > "C:\out file.txt" 2>&1""#
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_nothing_is_elevated() {
        assert!(matches!(
            Command::new("program").spawn(),
            Err(Error::Unsupported)
        ));
        assert!(!is_elevated().unwrap());
    }
}
