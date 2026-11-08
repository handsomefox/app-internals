# Repository guidelines

The [README](README.md) covers what the library does and how an app uses it. This page covers the rules for changing it.

## Five apps install through this code

DLSS Updater, Elden Ring Save Guard, osu!lazer Cleaner, Windows Cleaner, and Windows Toolkit each depend on this crate, pinned to a tag. Every installed copy of those apps updates through it. A bug here can strand every user on a version that cannot install the fix, and only a manual download gets them out. Change it conservatively.

## Do not break the release contract

The tag format, the archive name and layout, the `SHA256SUMS` lookup, and the check against GitHub's recorded digest are a contract with every installed copy. The README lists them. Renaming any part strands users on the version before the rename.

Do not weaken a check: hashes, digests, size caps, link filtering, or the extraction path checks.

`Settings` and `CheckPeriod` are saved by each app with its own state. Keep their serde shape, so existing users' settings still load with the same meaning.

`APP_UPDATE_PRETEND_VERSION` must work only in debug builds.

## The public API

Each app's updater UI calls the public API directly. A change to a `pub` signature or to observable behaviour needs a matching change in all five apps, released together. Prefer private changes.

## Dependencies

Version requirements in `Cargo.toml` are minimums, not exact `=` pins. That departs from the apps' rule of pinning everything exactly, on purpose. The apps pin every dependency with `=` and commit their `Cargo.lock`, so the versions they build with are still exact. Exact pins here would make every dependency bump in an app fail until this crate released first.

Keep the feature lists exactly as they are. Each app links one copy of `reqwest`, `rustls`, `flate2`, and `zip`, and a feature added here turns on in all five apps. `reqwest` must keep `rustls-no-provider`, with `ring` selected through the direct `rustls` dependency.

## Tests

Unit tests live in `src/lib.rs`. They run a local HTTP server, so nothing reaches GitHub. Cover the rejection path, not only the success path: a mismatched checksum, a malformed digest, an archive with the wrong layout, a link outside the releases, and a failed swap.

CI runs the tests on Windows as well as Linux, because the executable swap only runs on Windows. Do not drop the Windows job.

CI cannot reach a real release. Before you tag a change, run a debug build of one app against this commit, with `APP_UPDATE_PRETEND_VERSION` set to an older version, and walk the whole flow on Windows.

## Release a change

1. Add a `## <version>` section to `CHANGELOG.md`, and set `version` in `Cargo.toml` to match.
2. Push to `main` and wait for green CI.
3. Push an annotated `v<version>` tag on that commit, and create a GitHub release from the changelog section.
4. In each of the five apps, bump the `tag` and `version` of the `app-update` dependency, under one commit subject shared by all five, with Windows Toolkit first. Release each app as usual.

## Bump CI tool pins by hand

`scripts/install-ci-tool.sh` downloads cargo-audit, cargo-machete, actionlint, and zizmor from their release pages and checks each archive against a pinned SHA-256 before it extracts anything. Dependabot cannot bump these pins. To bump one, change its row in the script and take the new hash from the digest GitHub records for the asset:

```sh
gh release view <tag> -R <owner>/<repo> --json assets --jq '.assets[] | select(.name == "<asset>") | .digest'
```

The script is byte-for-byte the same as in the five apps. Change it in all six repositories at once.

CI runs actionlint, shellcheck, and `zizmor --persona pedantic` on every push. Run all three before you push a workflow change.
