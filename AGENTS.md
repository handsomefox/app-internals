# Repository guidelines

The [README](README.md) covers what each crate does and how an app uses it. This page covers the rules for changing them.

## What belongs here

A crate here holds plumbing that every app needs and no app should copy. It knows nothing about any one app: an app passes in its name, its folders, and its look. Code that only one app needs stays in that app.

Each shared feature gets its own crate under `crates/`, named for what it does: `crates/folders` is the `app-folders` package. Do not grow an existing crate to hold something unrelated, and do not add a catch-all crate.

`app-update-ui` is the only crate that may depend on egui. The others must build without a GUI, because Save Guard's core builds with `--no-default-features`.

## Five apps install through `app-update`

DLSS Updater, Elden Ring Save Guard, osu!lazer Cleaner, Windows Cleaner, and Windows Toolkit each depend on these crates, pinned to a tag. Every installed copy of those apps updates through `app-update`. A bug there can strand every user on a version that cannot install the fix, and only a manual download gets them out. Change it conservatively.

## Do not break the release contract

The tag format, the archive name and layout, the `SHA256SUMS` lookup, and the check against GitHub's recorded digest are a contract with every installed copy. The README lists them. Renaming any part strands users on the version before the rename.

Do not weaken a check: hashes, digests, size caps, link filtering, or the extraction path checks.

`Settings` and `CheckPeriod` are saved by each app with its own state. Keep their serde shape, so existing users' settings still load with the same meaning.

`APP_UPDATE_PRETEND_VERSION` must work only in debug builds.

The folder an app passes to `app_folders::app_data_dir` holds its users' data, backups, and settings. Never change where that function puts a folder.

## The public API

The apps call these crates' public API directly. A change to a `pub` signature or to observable behaviour needs a matching change in all five apps, released together. Prefer private changes.

## Dependencies

Version requirements in the root `Cargo.toml` are minimums, not exact `=` pins. That departs from the apps' rule of pinning everything exactly, on purpose. The apps pin every dependency with `=` and commit their `Cargo.lock`, so the versions they build with are still exact. Exact pins here would make every dependency bump in an app fail until this repository released first.

Keep the feature lists exactly as they are. Each app links one copy of every dependency, and a feature added here turns on in all five apps. `reqwest` must keep `rustls-no-provider`, with `ring` selected through the direct `rustls` dependency.

`egui` and `windows-sys` must resolve to the version every app already locks, or an app builds two copies and the egui types stop matching. To move an app to a new egui minor version, release a version of this repository that requires it first, then move all five apps together.

## Tests

Unit tests live in each crate's `src/lib.rs`. The `app-update` tests run a local HTTP server, so nothing reaches GitHub. Cover the rejection path, not only the success path: a mismatched checksum, a malformed digest, an archive with the wrong layout, a link outside the releases, and a failed swap.

CI runs the tests on Windows as well as Linux, because the executable swap, the known folders, and the shell only run on Windows.

CI cannot reach a real release or a real window. Before you tag a change, run a debug build of one app against this commit, with `APP_UPDATE_PRETEND_VERSION` set to an older version, and walk the whole update flow on Windows. Open the log folder from the app's About window too.

## Release a change

1. Add a `## <version>` section to `CHANGELOG.md`, and set `version` in `[workspace.package]` and the `app-update` entry in `[workspace.dependencies]` to match.
2. Push to `main` and wait for green CI.
3. Push an annotated `v<version>` tag on that commit, and create a GitHub release from the changelog section.
4. In each of the five apps, bump the `tag` and `version` of every crate from here, under one commit subject shared by all five, with Windows Toolkit first. Release each app as usual.

## CI

`.github/workflows/ci.yml` only calls the shared workflow in [handsomefox/app-workflows](https://github.com/handsomefox/app-workflows), pinned by commit, as the apps do. The checks and the CI tool pins live there. Dependabot moves the pin when that repository releases.

CI runs actionlint and `zizmor --persona pedantic` on every push. Run both before you push a workflow change.
