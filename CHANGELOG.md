# Changelog

## 0.2.0

- Rename the repository to `app-internals` and make it a workspace of crates that share one version and one tag. `app-update` moves to `crates/update` with its history.
- Add `AppInfo::name`, the app's name as users see it. Every app's `AppInfo` needs the new field.
- Add `app-update-ui`, the content of the "App updates" dialog and the header chip's text. Each app keeps its own window and look through the `Look` trait.
- Add `app-diagnostics`, which writes daily log files with a 10 MiB daily cap, keeps seven of them, and logs panics.
- Add `app-folders`, which finds the Windows known folders and the app's own data folder, and opens a folder in the system file manager.
- Check the crates with the shared workflow from `handsomefox/app-workflows`, in place of a copy of it here.

## 0.1.0

- Publish the updater as its own crate. It is the code DLSS Updater 1.4.5, Elden Ring Save Guard 1.2.3, osu!lazer Cleaner 1.7.3, Windows Cleaner 1.2.3, and Windows Toolkit 0.4.3 already ship, with the same behaviour. Its history starts at the first version those apps carried.
- License it under the Apache License, Version 2.0.
