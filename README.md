# app-update

[![CI](https://github.com/handsomefox/app-update/actions/workflows/ci.yml/badge.svg)](https://github.com/handsomefox/app-update/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

A Rust library that updates a desktop app from its own GitHub releases. It checks for a newer release, shows its notes, downloads the archive for the running platform, verifies it, and swaps in the new executable on restart.

It updates these apps:

- [DLSS Updater](https://github.com/handsomefox/dlss-updater)
- [Elden Ring Save Guard](https://github.com/handsomefox/eldenring-backuptool)
- [osu!lazer Cleaner](https://github.com/handsomefox/osu-lazer-cleaner)
- [Windows Cleaner](https://github.com/handsomefox/win-cleaner)
- [Windows Toolkit](https://github.com/handsomefox/win-toolkit)

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

Add it from this repository, pinned to a tag:

```toml
[dependencies]
app-update = { git = "https://github.com/handsomefox/app-update", tag = "v0.1.0", version = "=0.1.0" }
```

Create one `Updater` for the window, call `poll` once per frame, and restart after the event loop ends:

```rust
const APP_INFO: app_update::AppInfo = app_update::AppInfo {
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
if let Some(release) = updater.notification(&settings) {
    // Offer `release.version` and the notes of `updater.update()`.
}

// After the event loop has ended and the app's state is saved:
app_update::restart_if_requested()?;
```

`check_now`, `download`, `install_and_restart`, `skip`, and `dismiss` drive the rest of the flow from the app's own buttons.

## Test an update by hand

Debug builds read `APP_UPDATE_PRETEND_VERSION` as the running version. Run a debug build with it set to an older version, and the current release counts as newer, so the download, verification, swap, and restart run against real release assets. Release builds ignore the variable.

## Build and check

```sh
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
```

CI runs the tests and Clippy on Ubuntu and Windows, plus `cargo machete`, `cargo audit`, actionlint, shellcheck, and zizmor.

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE).
