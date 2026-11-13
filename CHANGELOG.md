# Changelog

## 0.2.3

- Keep the staged update's lock while it is read and swapped in. The install used to release the lock to read the file, and a second copy of the app starting in that moment could delete the download, so the install failed.
- Refuse to install a staged update over one another copy of the app already installed. Two open copies could each download an update, and the second to restart could swap an older version over a newer one. The refused copy says that another copy installed an update and asks for a restart.
- Remove only staged downloads with the name the updater gives them, `<exe>.new-<pid>-<hex time>`, plus the plain `<exe>.new` from before 0.2.2. A file such as `tool.exe.new-backup` beside the app is no longer deleted on startup.
- Prune only log files named `<prefix>-YYYY-MM-DD.log` around a date that exists. A log renamed to keep it, such as `app-2026-07-01-saved.log`, was counted as a day and could be deleted.

## 0.2.2

- Stage each downloaded update under its own name, and hold a lock on it until it is installed. A second copy of the app that starts while the first has an update waiting no longer deletes that download, so the first copy's install no longer fails. Startup still removes downloads that no running copy holds, and the plain `<exe>.new` that earlier versions staged.
- Add `app-process`, which reports whether a process with a given executable name is running. It enumerates names only, through Toolhelp on Windows and `/proc` on Linux, and never opens a handle to a process.
- Wait for `xdg-open` in the background after opening a folder on Linux, so it no longer stays behind as a zombie process until the app exits.

## 0.2.1

- Fall back to stderr when today's log file cannot be opened, instead of dropping every line. A later day's file that cannot be opened sends its lines to stderr as well.

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
