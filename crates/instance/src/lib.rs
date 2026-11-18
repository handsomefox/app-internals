//! One running copy per app. Launching the app again brings the running
//! copy's window to the front, and the new process exits.
//!
//! [`acquire`] takes a lock named after the app in the user's session. The
//! first copy gets [`Instance::Primary`] and keeps its [`Guard`] until it
//! exits. A later launch gets [`Instance::Secondary`]: it has already asked the
//! first copy to come forward, and it should exit without opening a window.
//! The first copy learns of each later launch through
//! [`Guard::on_activate`].
//!
//! ```no_run
//! // On any error, run without the guard rather than refuse to start.
//! let guard = match app_instance::acquire("win-toolkit") {
//!     Ok(app_instance::Instance::Primary(guard)) => Some(guard),
//!     Ok(app_instance::Instance::Secondary) => return,
//!     Err(_error) => None, // Log it.
//! };
//! // Once the window exists, with its context in `ctx`:
//! if let Some(guard) = &guard {
//!     let watching = guard.on_activate(move || {
//!         // ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
//!         // ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
//!         // ctx.request_repaint();
//!     });
//!     if let Err(_error) = watching {
//!         // Log it; the app runs on.
//!     }
//! }
//! // Keep `guard` until `main` returns.
//! ```
//!
//! # Where to call it
//!
//! Call [`acquire`] once, on the main thread, on the normal startup path that
//! opens the window, and keep the guard until `main` returns. A Windows mutex
//! belongs to the thread that took it, so a guard dropped on another thread
//! cannot release it early: the lock then lasts until the main thread exits.
//!
//! Processes that are not the app's window do not call it: an elevated helper,
//! a monitor mode, or any other copy the app starts for a job of its own. Each
//! of them would otherwise see the window's lock and exit.
//!
//! # Updates
//!
//! `app-update` starts the updated executable while the old copy may still
//! hold the lock. It marks that process with [`hand_off`], and [`acquire`] in a
//! marked process waits up to 15 seconds for the lock instead of reporting
//! another copy. `acquire` removes the mark from its own environment, so
//! processes the app starts later do not inherit it. If the old copy still
//! holds the lock after the wait, the new one runs anyway, unguarded, because
//! the old copy no longer has a window to bring forward.
//!
//! # Rights
//!
//! The lock and the event live in the session's `Local\` namespace. They
//! grant full access to the user who created them and to SYSTEM, and carry a
//! Medium integrity label, so copies of the same user share them whether or
//! not they run as administrator. Another user's copy, as with an
//! administrator who signed in at a UAC prompt, gets [`Error::Create`]
//! (access denied). Treat any error as "run without the guard": refusing to
//! start would leave the user with no window.
//!
//! # Other platforms
//!
//! Off Windows, [`acquire`] always returns [`Instance::Primary`], and
//! [`Guard::on_activate`] never calls its callback. Every launch opens its own
//! window, as before.

#[cfg(any(windows, test))]
use std::ffi::OsStr;
use std::io;
use std::marker::PhantomData;
#[cfg(any(windows, test))]
use std::time::Duration;

/// The environment variable [`hand_off`] sets: the process id of the copy
/// handing over. Version N sets it and version N+1 reads it, so never rename
/// it.
#[cfg(windows)]
const HANDOFF_VAR: &str = "APP_INSTANCE_HANDOFF";
/// How long an updated copy waits for the copy that started it to exit.
#[cfg(any(windows, test))]
const HANDOFF_WAIT: Duration = Duration::from_secs(15);
/// Keeps `Local\app-instance.<id>.activate` well inside the 260 characters
/// Windows allows for an object name.
const MAX_ID_LEN: usize = 200;

/// Why [`acquire`] or [`Guard::on_activate`] failed. In every case the app
/// should run without the guard rather than refuse to start.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The app id is empty, too long, or holds a backslash or NUL.
    #[error("{0:?} cannot name an instance lock: use 1 to 200 characters without \\ or NUL")]
    InvalidId(String),
    /// Windows refused to create or open the lock or one of the events. Access
    /// is denied when a copy with administrator rights created them.
    #[error("could not create the {object}: {source}")]
    Create {
        object: &'static str,
        #[source]
        source: io::Error,
    },
    /// Waiting for the lock failed.
    #[error("could not wait for the instance lock: {0}")]
    Wait(#[source] io::Error),
    /// Another copy holds the lock, but it could not be asked to come forward.
    #[error("could not signal the running copy: {0}")]
    Signal(#[source] io::Error),
    /// The thread that watches for later launches could not start.
    #[error("could not start watching for other launches: {0}")]
    Watch(#[source] io::Error),
}

/// What [`acquire`] found.
#[must_use]
pub enum Instance {
    /// This is the app's running copy. Keep the guard until the process
    /// exits.
    Primary(Guard),
    /// Another copy is running and has been asked to come to the front. Exit
    /// without opening a window.
    Secondary,
}

/// The app's instance lock, held until the guard drops.
///
/// It must drop on the thread that took it, so it is neither `Send` nor
/// `Sync`.
pub struct Guard {
    #[cfg(windows)]
    held: windows::Held,
    _thread_bound: PhantomData<*const ()>,
}

impl Guard {
    /// Runs `callback` on a background thread each time another launch of
    /// the app asks this copy to come forward. A launch made before this call
    /// is delivered once the watch starts. Calling it again replaces the
    /// earlier callback.
    ///
    /// Off Windows this never calls `callback`.
    ///
    /// # Errors
    ///
    /// Returns an error when the watching thread or its stop event cannot be
    /// created.
    pub fn on_activate(&self, callback: impl Fn() + Send + 'static) -> Result<(), Error> {
        #[cfg(windows)]
        {
            self.held.on_activate(callback)
        }
        #[cfg(not(windows))]
        {
            drop(callback);
            Ok(())
        }
    }
}

/// Takes the instance lock for `app_id`, or asks the copy that holds it to
/// come to the front.
///
/// `app_id` names the lock in the user's session, so it must be the same in
/// every version of the app. Use the executable name, such as `win-toolkit`.
///
/// # Errors
///
/// Returns an error when `app_id` cannot name a lock, or when Windows refuses
/// to create, wait on, or signal the lock or its event. See [`Error`].
pub fn acquire(app_id: &str) -> Result<Instance, Error> {
    validate(app_id)?;
    #[cfg(windows)]
    {
        windows::acquire(app_id, take_handoff())
    }
    #[cfg(not(windows))]
    {
        Ok(Instance::Primary(Guard {
            _thread_bound: PhantomData,
        }))
    }
}

/// Marks `command` as the app's successor: [`acquire`] in that process waits
/// for this one to release the lock instead of reporting a running copy.
/// `app-update` calls this when it restarts into an update.
///
/// Off Windows it leaves `command` unchanged.
pub fn hand_off(command: &mut std::process::Command) {
    #[cfg(windows)]
    command.env(HANDOFF_VAR, std::process::id().to_string());
    #[cfg(not(windows))]
    let _ = command;
}

fn validate(app_id: &str) -> Result<(), Error> {
    let length = app_id.chars().count();
    if length == 0 || length > MAX_ID_LEN || app_id.contains(['\\', '\0']) {
        return Err(Error::InvalidId(app_id.to_owned()));
    }
    Ok(())
}

/// How long to wait for the lock, given the value of [`HANDOFF_VAR`]. Only a
/// process id, as [`hand_off`] writes it, counts as a mark.
#[cfg(any(windows, test))]
fn handoff_wait(mark: Option<&OsStr>) -> Duration {
    match mark.and_then(OsStr::to_str).map(str::parse::<u32>) {
        Some(Ok(_)) => HANDOFF_WAIT,
        _ => Duration::ZERO,
    }
}

/// Reads and removes this process's hand-off mark.
#[cfg(windows)]
fn take_handoff() -> Duration {
    let mark = std::env::var_os(HANDOFF_VAR);
    if mark.is_some() {
        // SAFETY: on Windows the environment functions are thread-safe, and
        // the standard library documents `remove_var` as always safe there.
        unsafe { std::env::remove_var(HANDOFF_VAR) };
    }
    let wait = handoff_wait(mark.as_deref());
    if !wait.is_zero() {
        tracing::info!(
            from = ?mark,
            "started by an update; waiting for the previous copy to exit"
        );
    }
    wait
}

/// The name of the lock or the event in the session. Every version of an app
/// must find the lock an earlier version holds, so never change this.
#[cfg(any(windows, test))]
fn object_name(app_id: &str, kind: &str) -> String {
    format!("Local\\app-instance.{app_id}.{kind}")
}

#[cfg(windows)]
mod windows {
    use std::cell::RefCell;
    use std::io;
    use std::marker::PhantomData;
    use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
    use std::sync::Arc;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        HANDLE, LocalFree, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateMutexW, GetCurrentProcess, INFINITE, OpenProcessToken, ReleaseMutex,
        SetEvent, WaitForMultipleObjects, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    use windows_sys::core::PWSTR;

    use super::{Error, Guard, Instance, object_name};

    pub(super) struct Held {
        mutex: OwnedHandle,
        /// Whether this process owns `mutex`. A copy that timed out waiting
        /// for an update's hand-off holds the handle without owning it.
        owned: bool,
        activate: Arc<OwnedHandle>,
        watcher: RefCell<Option<Watcher>>,
    }

    impl Drop for Held {
        fn drop(&mut self) {
            if self.owned {
                // SAFETY: `mutex` is a valid mutex handle. Release fails
                // harmlessly when this thread does not own it.
                unsafe { ReleaseMutex(self.mutex.as_raw_handle()) };
            }
        }
    }

    /// Stops its watching thread when dropped.
    struct Watcher {
        stop: Arc<OwnedHandle>,
    }

    impl Drop for Watcher {
        fn drop(&mut self) {
            // SAFETY: `stop` is a valid event handle.
            unsafe { SetEvent(self.stop.as_raw_handle()) };
        }
    }

    impl Held {
        pub(super) fn on_activate(
            &self,
            callback: impl Fn() + Send + 'static,
        ) -> Result<(), Error> {
            let stop = Arc::new(create_event(None, true, "stop event")?);
            let activate = Arc::clone(&self.activate);
            let watched_stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("app-instance-activate".to_owned())
                .spawn(move || watch(&activate, &watched_stop, &callback))
                .map_err(Error::Watch)?;
            // Dropping the previous watcher stops its thread.
            *self.watcher.borrow_mut() = Some(Watcher { stop });
            Ok(())
        }
    }

    fn watch(activate: &OwnedHandle, stop: &OwnedHandle, callback: &dyn Fn()) {
        let handles: [HANDLE; 2] = [activate.as_raw_handle(), stop.as_raw_handle()];
        loop {
            // SAFETY: both handles are valid events kept alive by the `Arc`s
            // this thread holds.
            let waited = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
            if waited == WAIT_OBJECT_0 {
                callback();
            } else if waited == WAIT_OBJECT_0 + 1 {
                return;
            } else {
                tracing::warn!(
                    error = %io::Error::last_os_error(),
                    "stopped watching for other launches"
                );
                return;
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The security descriptor every named object gets, freed on drop.
    ///
    /// With default security, an object an elevated copy creates carries the
    /// elevated token's default DACL (Administrators and SYSTEM) and a High
    /// integrity label, so a normal copy of the same user could neither see
    /// the lock nor signal the event, and would open a second window. This
    /// grants the user who runs the process, and SYSTEM, full access, and
    /// labels the object Medium with no-write-up, so a copy at either level
    /// can wait on the lock and signal the event.
    struct Security {
        descriptor: PSECURITY_DESCRIPTOR,
    }

    impl Drop for Security {
        fn drop(&mut self) {
            // SAFETY: `descriptor` came from
            // ConvertStringSecurityDescriptorToSecurityDescriptorW, which
            // allocates it with LocalAlloc.
            unsafe { LocalFree(self.descriptor) };
        }
    }

    impl Security {
        fn for_current_user() -> Result<Self, Error> {
            let sid = current_user_sid().map_err(|source| Error::Create {
                object: "security descriptor",
                source,
            })?;
            let sddl = wide(&format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)S:(ML;;NW;;;ME)"));
            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            // SAFETY: `sddl` is NUL-terminated and outlives the call, and
            // `descriptor` is a valid out pointer; the size is not wanted.
            let converted = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &raw mut descriptor,
                    std::ptr::null_mut(),
                )
            };
            if converted == 0 {
                return Err(Error::Create {
                    object: "security descriptor",
                    source: io::Error::last_os_error(),
                });
            }
            Ok(Self { descriptor })
        }

        fn attributes(&self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
                lpSecurityDescriptor: self.descriptor,
                bInheritHandle: 0,
            }
        }
    }

    /// The SID of the user this process runs as, in its `S-1-…` string form.
    fn current_user_sid() -> io::Result<String> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: a plain call; the pseudo handle needs no closing.
        let process = unsafe { GetCurrentProcess() };
        // SAFETY: `process` is this process's pseudo handle, and `token` is a
        // valid out pointer.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken succeeded, so `token` is open and ours.
        let token = unsafe { OwnedHandle::from_raw_handle(token) };
        let mut needed = 0u32;
        // SAFETY: a size query with no buffer; it fails and sets `needed`.
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &raw mut needed,
            )
        };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        // u64 storage keeps the TOKEN_USER at the start suitably aligned.
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `buffer` holds at least `needed` bytes.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &raw mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: GetTokenInformation wrote a TOKEN_USER at the start of the
        // aligned buffer, and its SID points inside the buffer.
        let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut text: PWSTR = std::ptr::null_mut();
        // SAFETY: `sid` is valid while `buffer` lives, and `text` is a valid
        // out pointer.
        if unsafe { ConvertSidToStringSidW(sid, &raw mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0;
        // SAFETY: on success `text` is a NUL-terminated string, so every
        // index up to and including its terminator is in bounds.
        while unsafe { text.wrapping_add(len).read() } != 0 {
            len += 1;
        }
        // SAFETY: `text` holds `len` initialized characters before its
        // terminator.
        let string = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) });
        // SAFETY: ConvertSidToStringSidW allocated `text` with LocalAlloc, and
        // nothing reads it after this.
        unsafe { LocalFree(text.cast()) };
        Ok(string)
    }

    /// Takes ownership of a handle a create call returned, or reads the
    /// error when it returned null.
    fn owned(handle: HANDLE, object: &'static str) -> Result<OwnedHandle, Error> {
        if handle.is_null() {
            return Err(Error::Create {
                object,
                source: io::Error::last_os_error(),
            });
        }
        // SAFETY: a non-null handle from a create call is open, and nothing
        // else owns it.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }

    /// An event, auto-reset unless `manual_reset`, named when `name` is set.
    fn create_event(
        name: Option<&str>,
        manual_reset: bool,
        object: &'static str,
    ) -> Result<OwnedHandle, Error> {
        // Only a named event is shared with other copies; a private one keeps
        // default security.
        let security = name.map(|_| Security::for_current_user()).transpose()?;
        let attributes = security.as_ref().map(Security::attributes);
        let attributes_ptr = attributes
            .as_ref()
            .map_or(std::ptr::null(), std::ptr::from_ref);
        let name = name.map(wide);
        let name_ptr = name.as_ref().map_or(std::ptr::null(), Vec::as_ptr);
        // SAFETY: the attributes are null or point at a descriptor that
        // outlives the call, and the name is null or NUL-terminated and
        // outlives the call.
        let handle = unsafe { CreateEventW(attributes_ptr, i32::from(manual_reset), 0, name_ptr) };
        owned(handle, object)
    }

    fn millis(wait: Duration) -> u32 {
        u32::try_from(wait.as_millis()).map_or(INFINITE - 1, |millis| millis.min(INFINITE - 1))
    }

    /// [`super::acquire`] once `app_id` is valid, waiting up to `wait` for the
    /// lock.
    pub(super) fn acquire(app_id: &str, wait: Duration) -> Result<Instance, Error> {
        // The event exists before the lock is taken, so a later launch that
        // finds the lock held always finds the event open, and its signal
        // waits until the watcher starts.
        let activate = create_event(
            Some(&object_name(app_id, "activate")),
            false,
            "activation event",
        )?;
        let name = wide(&object_name(app_id, "lock"));
        let security = Security::for_current_user()?;
        let attributes = security.attributes();
        // SAFETY: the attributes point at a descriptor that outlives the
        // call, no initial owner, and a NUL-terminated name that outlives the
        // call.
        let mutex = unsafe { CreateMutexW(&raw const attributes, 0, name.as_ptr()) };
        let mutex = owned(mutex, "instance lock")?;
        // SAFETY: `mutex` is a valid mutex handle.
        let waited = unsafe { WaitForSingleObject(mutex.as_raw_handle(), millis(wait)) };
        let owned = match waited {
            // An abandoned lock belonged to a copy that ended without
            // releasing it; it is this process's now.
            WAIT_OBJECT_0 | WAIT_ABANDONED => true,
            WAIT_TIMEOUT if wait.is_zero() => {
                signal(&activate)?;
                return Ok(Instance::Secondary);
            }
            WAIT_TIMEOUT => {
                tracing::warn!(
                    ?wait,
                    "the previous copy still holds the instance lock; running without it"
                );
                false
            }
            _ => return Err(Error::Wait(io::Error::last_os_error())),
        };
        Ok(Instance::Primary(Guard {
            held: Held {
                mutex,
                owned,
                activate: Arc::new(activate),
                watcher: RefCell::new(None),
            },
            _thread_bound: PhantomData,
        }))
    }

    fn signal(activate: &OwnedHandle) -> Result<(), Error> {
        // This launch came from the user, so it may take the foreground, and
        // it passes that on to the running copy. A failure only means the
        // window may flash in the taskbar instead of coming forward.
        // SAFETY: a plain call with no pointers.
        unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        // SAFETY: `activate` is a valid event handle.
        if unsafe { SetEvent(activate.as_raw_handle()) } == 0 {
            return Err(Error::Signal(io::Error::last_os_error()));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::ffi::OsStr;
        use std::sync::mpsc;
        use std::time::Instant;

        /// A lock name no other test or process uses.
        fn id(test: &str) -> String {
            format!("app-instance-test-{}-{test}", std::process::id())
        }

        fn primary(id: &str, wait: Duration) -> Guard {
            match acquire(id, wait).unwrap() {
                Instance::Primary(guard) => guard,
                Instance::Secondary => panic!("expected to hold the lock"),
            }
        }

        /// Runs `acquire` on another thread, since the lock's owning thread
        /// would simply take it again.
        fn elsewhere(id: &str, wait: Duration) -> Result<bool, Error> {
            let id = id.to_owned();
            std::thread::spawn(move || {
                acquire(&id, wait).map(|instance| matches!(instance, Instance::Primary(_)))
            })
            .join()
            .unwrap()
        }

        #[test]
        fn shared_objects_grant_the_current_user() {
            let sid = current_user_sid().expect("SID");
            assert!(sid.starts_with("S-1-"), "{sid}");
            let security = Security::for_current_user().expect("descriptor");
            assert!(!security.attributes().lpSecurityDescriptor.is_null());
        }

        #[test]
        fn a_second_launch_signals_the_first() {
            let id = id("signal");
            let guard = primary(&id, Duration::ZERO);
            let (sender, receiver) = mpsc::channel();
            guard.on_activate(move || sender.send(()).unwrap()).unwrap();
            assert!(!elsewhere(&id, Duration::ZERO).unwrap());
            receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(!elsewhere(&id, Duration::ZERO).unwrap());
            receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        }

        #[test]
        fn a_launch_before_the_watch_starts_is_delivered() {
            let id = id("early");
            let guard = primary(&id, Duration::ZERO);
            assert!(!elsewhere(&id, Duration::ZERO).unwrap());
            let (sender, receiver) = mpsc::channel();
            guard.on_activate(move || sender.send(()).unwrap()).unwrap();
            receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        }

        #[test]
        fn a_replaced_or_dropped_watch_stops() {
            let id = id("replace");
            let guard = primary(&id, Duration::ZERO);
            let (first, first_calls) = mpsc::channel();
            guard.on_activate(move || first.send(()).unwrap()).unwrap();
            let (second, second_calls) = mpsc::channel();
            guard.on_activate(move || second.send(()).unwrap()).unwrap();
            // The first watcher's thread has exited and dropped its sender.
            assert_eq!(
                first_calls.recv_timeout(Duration::from_secs(5)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            );
            assert!(!elsewhere(&id, Duration::ZERO).unwrap());
            second_calls.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(guard);
            assert_eq!(
                second_calls.recv_timeout(Duration::from_secs(5)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            );
        }

        #[test]
        fn dropping_the_guard_releases_the_lock() {
            let id = id("release");
            let guard = primary(&id, Duration::ZERO);
            assert!(!elsewhere(&id, Duration::ZERO).unwrap());
            drop(guard);
            assert!(elsewhere(&id, Duration::ZERO).unwrap());
        }

        #[test]
        fn a_lock_whose_owner_ended_is_taken_over() {
            let id = id("abandoned");
            let owner = id.clone();
            std::thread::spawn(move || {
                // Leaked, so the thread ends owning the lock without
                // releasing it, as a process that crashed would.
                std::mem::forget(primary(&owner, Duration::ZERO));
            })
            .join()
            .unwrap();
            let _guard = primary(&id, Duration::ZERO);
        }

        #[test]
        fn an_update_waits_for_the_previous_copy() {
            let id = id("handoff");
            let (ready, is_ready) = mpsc::channel();
            let owner = id.clone();
            let previous = std::thread::spawn(move || {
                let guard = primary(&owner, Duration::ZERO);
                ready.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(300));
                drop(guard);
            });
            is_ready.recv().unwrap();
            let started = Instant::now();
            assert!(elsewhere(&id, Duration::from_secs(10)).unwrap());
            assert!(started.elapsed() < Duration::from_secs(10));
            previous.join().unwrap();
        }

        #[test]
        fn an_update_runs_unguarded_when_the_previous_copy_does_not_exit() {
            let id = id("handoff-timeout");
            let guard = primary(&id, Duration::ZERO);
            let (sender, receiver) = mpsc::channel();
            guard.on_activate(move || sender.send(()).unwrap()).unwrap();
            assert!(elsewhere(&id, Duration::from_millis(200)).unwrap());
            // Running anyway is not a second launch, so nothing is signalled.
            assert_eq!(
                receiver.recv_timeout(Duration::from_millis(300)),
                Err(mpsc::RecvTimeoutError::Timeout)
            );
        }

        #[test]
        fn hand_off_marks_the_command_with_this_process() {
            let mut command = std::process::Command::new("app.exe");
            super::super::hand_off(&mut command);
            let marks: Vec<_> = command
                .get_envs()
                .filter(|(name, _)| *name == super::super::HANDOFF_VAR)
                .collect();
            let pid = std::process::id().to_string();
            assert_eq!(
                marks,
                [(
                    OsStr::new(super::super::HANDOFF_VAR),
                    Some(OsStr::new(&pid))
                )]
            );
        }

        #[test]
        fn taking_the_mark_removes_it() {
            // No other test reads this variable through the public `acquire`.
            // SAFETY: the environment functions are thread-safe on Windows.
            unsafe { std::env::set_var(super::super::HANDOFF_VAR, "4242") };
            assert_eq!(super::super::take_handoff(), super::super::HANDOFF_WAIT);
            assert_eq!(std::env::var_os(super::super::HANDOFF_VAR), None);
            assert_eq!(super::super::take_handoff(), Duration::ZERO);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_must_name_a_session_object() {
        for id in ["win-toolkit", "Elden Ring Save Guard", "osu!lazer cleaner"] {
            assert!(validate(id).is_ok(), "{id}");
        }
        let long = "a".repeat(MAX_ID_LEN + 1);
        for id in ["", "a\\b", "a\0b", long.as_str()] {
            assert!(matches!(validate(id), Err(Error::InvalidId(_))), "{id:?}");
        }
        assert!(validate(&"a".repeat(MAX_ID_LEN)).is_ok());
    }

    #[test]
    fn only_a_process_id_marks_an_update() {
        assert_eq!(handoff_wait(Some(OsStr::new("4242"))), HANDOFF_WAIT);
        for mark in [None, Some(""), Some("yes"), Some("-1"), Some("4242 ")] {
            assert_eq!(
                handoff_wait(mark.map(OsStr::new)),
                Duration::ZERO,
                "{mark:?}"
            );
        }
    }

    #[test]
    fn object_names_stay_apart_from_other_locks() {
        assert_eq!(
            object_name("win-toolkit", "lock"),
            "Local\\app-instance.win-toolkit.lock"
        );
        assert_ne!(object_name("app", "lock"), object_name("app", "activate"));
    }

    #[cfg(not(windows))]
    #[test]
    fn elsewhere_every_launch_is_the_primary() {
        let Instance::Primary(guard) = acquire("app-instance-test").unwrap() else {
            panic!("expected the primary copy");
        };
        guard.on_activate(|| unreachable!()).unwrap();
        assert!(matches!(
            acquire("app-instance-test").unwrap(),
            Instance::Primary(_)
        ));
        let mut command = std::process::Command::new("app");
        hand_off(&mut command);
        assert_eq!(command.get_envs().count(), 0);
    }
}
