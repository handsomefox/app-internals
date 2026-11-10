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

Every crate has the same version, and one tag releases all of them.

## What an update checks

- It installs only from a published, non-prerelease `v<major>.<minor>.<patch>` release that is newer than the running version.
- The archive must match its line in the release's `SHA256SUMS` asset. When GitHub records a digest for the archive, the archive must match that too. A release whose GitHub digest cannot be read is not offered.
- Downloads are size-capped, and only links inside the app's own GitHub releases are followed.
- The new executable is extracted next to the running one and hashed again before the swap. Windows cannot overwrite a running executable, so the swap renames the running file aside first. If the swap fails, the original is put back. The next start removes what is left.

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
app-update = { git = "https://github.com/handsomefox/app-internals", tag = "v0.2.1", version = "=0.2.1" }
app-update-ui = { git = "https://github.com/handsomefox/app-internals", tag = "v0.2.1", version = "=0.2.1" }
app-diagnostics = { git = "https://github.com/handsomefox/app-internals", tag = "v0.2.1", version = "=0.2.1" }
app-folders = { git = "https://github.com/handsomefox/app-internals", tag = "v0.2.1", version = "=0.2.1" }
```

Start logging first, so a failure while the window opens still reaches the log:

```rust
let logs = app_folders::app_data_dir("win-toolkit").map(|dir| dir.join("logs"));
app_diagnostics::init(logs.as_deref(), "win-toolkit", "info");
```

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
