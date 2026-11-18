# Changelog

## 0.3.0

- Check the downloaded `SHA256SUMS` against the digest GitHub records for it, as the archive already was. A mismatch fails the download with `Error::ChecksumMismatch`. A release whose `SHA256SUMS` digest cannot be read is not offered, as with the archive.
- Mark the restarted executable with `app_instance::hand_off`, so the updated copy waits for the old one to release its instance lock instead of exiting as a second copy. Without the mark, an app that holds the lock would close on its next update and leave the user with no window.
- Open only folders from `app_folders::open`. A path that is not a folder now fails with `ErrorKind::NotADirectory` before the shell sees it, and Windows opens the folder with the `explore` verb instead of `open`, so a file is never run.
- Add `app-instance`, which keeps one running copy of an app. A second launch asks the first copy to come to the front and exits. Off Windows every launch runs as before.
- Add `app-elevation`, which starts a program with administrator rights through the UAC prompt. It quotes arguments for the C runtime, reports a declined prompt as `Error::Cancelled`, owns the process handle so the app can wait for the exit code, and reports whether the running process is elevated.
- Add `app-paths`, the path guard for apps that delete or move files. `is_safe_path` accepts a path only when it lies strictly inside a trusted root with no link or reparse point on the way. It rejects `..`, names that Win32 trims or reads as a device or stream, and paths that `std::path::absolute` reads differently. `overlap` follows links to tell whether two folders overlap.
- Build with Rust 1.100.0.
- Check the crates with `handsomefox/app-workflows` 0.2.0.

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
