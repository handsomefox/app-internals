# app-internals

[![CI](https://github.com/handsomefox/app-internals/actions/workflows/ci.yml/badge.svg)](https://github.com/handsomefox/app-internals/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

Rust crates that these desktop apps share, so none of them keeps its own copy:

- [DLSS Updater](https://github.com/handsomefox/dlss-updater)
- [Elden Ring Save Guard](https://github.com/handsomefox/eldenring-backuptool)
- [osu!lazer Cleaner](https://github.com/handsomefox/osu-lazer-cleaner)
- [Windows Cleaner](https://github.com/handsomefox/win-cleaner)
- [Windows Toolkit](https://github.com/handsomefox/win-toolkit)

| Crate | Folder | What it does |
| --- | --- | --- |
| `app-update` | `crates/update` | Updates the app from its own GitHub releases. It checks for a newer release, downloads the archive for the running platform, verifies it, and swaps in the new executable on restart. |
| `app-update-ui` | `crates/update-ui` | The egui content of the "App updates" dialog and the header chip's text. Each app keeps its own window and look. |
| `app-diagnostics` | `crates/diagnostics` | Daily log files with a 10 MiB daily cap, seven days kept, and a panic hook that logs the panic. |
| `app-folders` | `crates/folders` | The Windows known folders, with an environment fallback, the app's own data folder, and opening a folder in the system file manager. |
| `app-process` | `crates/process` | Whether a process with a given executable name is running. It finds processes by name only and never opens a handle to one. |
| `app-instance` | `crates/instance` | One running copy per app. A second launch brings the first copy's window to the front and exits. |
| `app-elevation` | `crates/elevation` | Starts a program with administrator rights through the UAC prompt, and waits for its exit code. |
| `app-paths` | `crates/paths` | Whether a path lies strictly inside a trusted folder with no link on the way, and whether two folders overlap once links are followed. |

Every crate has the same version, and one tag releases all of them.

## What an update checks

- It installs only from a published, non-prerelease `v<major>.<minor>.<patch>` release that is newer than the running version.
- The archive must match its line in the release's `SHA256SUMS` asset. When GitHub records a digest for the archive or for `SHA256SUMS`, each file must match its digest too. A release whose GitHub digests cannot be read is not offered.
- Downloads are size-capped, and only links inside the app's own GitHub releases are followed.
- The new executable is extracted next to the running one under a name of its own, and hashed again before the swap. The copy of the app that downloaded it holds a lock on it until then, so another copy starting up leaves it alone. Windows cannot overwrite a running executable, so the swap renames the running file aside first. If the swap fails, the original is put back. The next start removes what is left.
- The restarted copy is marked with `app_instance::hand_off`, so it waits for the old copy to release the instance lock instead of exiting as a second copy.

## The release layout

The layout is a contract with every installed copy of an app. Renaming any part strands users on the version before the rename.

- The tag is `v<major>.<minor>.<patch>`.
- The archive is `<bin>-<version>-<platform>.zip`, or `.tar.gz` on Linux, and holds `<bin>-<version>-<platform>/<bin>[.exe]`.
- A `SHA256SUMS` asset lists the archive.
- The platform is `windows-x86_64` or `linux-x86_64`. `app_update::platform()` returns it, and `None` where no release is published.

## Use it

Add the crates from this repository, pinned to a tag. Every crate an app uses takes the same tag:

```toml
[dependencies]
app-update = { git = "https://github.com/handsomefox/app-internals", tag = "v0.3.0", version = "=0.3.0" }
app-update-ui = { git = "https://github.com/handsomefox/app-internals", tag = "v0.3.0", version = "=0.3.0" }
app-diagnostics = { git = "https://github.com/handsomefox/app-internals", tag = "v0.3.0", version = "=0.3.0" }
app-folders = { git = "https://github.com/handsomefox/app-internals", tag = "v0.3.0", version = "=0.3.0" }
app-process = { git = "https://github.com/handsomefox/app-internals", tag = "v0.3.0", version = "=0.3.0" }
app-instance = { git = "https://github.com/handsomefox/app-internals", tag = "v0.3.0", version = "=0.3.0" }
app-elevation = { git = "https://github.com/handsomefox/app-internals", tag = "v0.3.0", version = "=0.3.0" }
app-paths = { git = "https://github.com/handsomefox/app-internals", tag = "v0.3.0", version = "=0.3.0" }
```

Start logging first, so a failure while the window opens still reaches the log:

```rust
let logs = app_folders::app_data_dir("win-toolkit").map(|dir| dir.join("logs"));
app_diagnostics::init(logs.as_deref(), "win-toolkit", "info");
```

Take the instance lock next, on the startup path that opens the window. A second launch exits here:

```rust
let instance = match app_instance::acquire("win-toolkit") {
    Ok(app_instance::Instance::Primary(guard)) => Some(guard),
    Ok(app_instance::Instance::Secondary) => return Ok(()),
    // Run without the guard rather than refuse to start.
    Err(error) => {
        tracing::warn!(%error, "running without the single-instance lock");
        None
    }
};

// Once the window exists:
if let Some(guard) = &instance {
    let ctx = cc.egui_ctx.clone();
    let watching = guard.on_activate(move || {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    });
    if let Err(error) = watching {
        tracing::warn!(%error, "a second launch will not bring this window forward");
    }
}
```

Keep the guard until `main` returns. An elevated helper, a monitor mode, or any other copy the app starts for a job of its own does not call `acquire`.

Create one `Updater` for the window, call `poll` once per frame, and restart after the event loop ends:

```rust
const APP_INFO: app_update::AppInfo = app_update::AppInfo {
    name: "Windows Toolkit",
    owner: "handsomefox",
    repo: "win-toolkit",
    bin: "win-toolkit",
    version: env!("CARGO_PKG_VERSION"),
};

// `repaint` runs on the updater's background thread when a check or a
// download reports back, so the window can redraw.
let mut updater = app_update::Updater::new(APP_INFO, move || ctx.request_repaint());

// Each frame. `Settings` holds the check period and the skipped version;
// persist it with the app's own state.
updater.poll(&mut settings);

// After the event loop has ended and the app's state is saved:
app_update::restart_if_requested()?;
```

`app-update-ui` draws the dialog's content inside the app's own window, through a `Look` that the app implements for its colors, spacing, icons, and buttons. The app runs the `Action` it returns with `app_update_ui::apply`.

## Test an update by hand

Debug builds read `APP_UPDATE_PRETEND_VERSION` as the running version. Run a debug build with it set to an older version, and the current release counts as newer, so the download, verification, swap, and restart run against real release assets. Release builds ignore the variable.

## Build and check

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

CI runs the tests and Clippy on Ubuntu and Windows, plus `cargo machete`, `cargo audit`, actionlint, and zizmor. It calls the same shared workflow as the apps, from [handsomefox/app-workflows](https://github.com/handsomefox/app-workflows).

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE).
