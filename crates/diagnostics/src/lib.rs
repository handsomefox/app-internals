//! Daily log files with a size cap, and a panic hook that logs the panic.
//!
//! A desktop app has nowhere to print, so a log file is the only way to find
//! out what happened after the fact. Every run appends to
//! `<dir>/<prefix>-<YYYY-MM-DD>.log`, which the user can attach to an issue.
//! A file stops growing at [`MAX_LOG_BYTES`] for the day, and the folder keeps
//! the newest [`MAX_LOG_DAYS`] files.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

/// The most one day's log file grows to. Later lines that day are dropped.
pub const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
/// How many daily log files the folder keeps.
pub const MAX_LOG_DAYS: usize = 7;

/// Installs the global tracing subscriber and the panic hook.
///
/// Logs go to daily files in `dir`, named after `prefix`. `RUST_LOG` sets the
/// filter when it is set, and `default_filter`, such as `"info"`, otherwise.
/// When `dir` is `None` or cannot be created, logs go to stderr instead.
///
/// Returns the folder the log files go to, when file logging is set up.
pub fn init(dir: Option<&Path>, prefix: &str, default_filter: &str) -> Option<PathBuf> {
    let filter = || {
        EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new(default_filter))
            .unwrap_or_else(|_| EnvFilter::new("info"))
    };
    let dir = dir.filter(|dir| fs::create_dir_all(dir).is_ok());
    if let Some(dir) = dir {
        prune_old_logs(dir, prefix);
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter())
            .with_ansi(false)
            .with_writer(DailyWriter::new(dir.to_owned(), prefix.to_owned()))
            .try_init();
    } else {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter())
            .with_writer(io::stderr)
            .try_init();
    }
    install_panic_hook();
    dir.map(Path::to_owned)
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic: {info}\n{backtrace}");
        default_hook(info);
    }));
}

fn log_name(prefix: &str, date: &str) -> String {
    format!("{prefix}-{date}.log")
}

fn today() -> String {
    jiff::Zoned::now().strftime("%Y-%m-%d").to_string()
}

/// Deletes the oldest of this app's log files beyond [`MAX_LOG_DAYS`]. Dated
/// names sort by date, and other files in the folder are left alone.
fn prune_old_logs(dir: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let start = format!("{prefix}-");
    let mut logs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(&start)
                        && Path::new(name)
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("log"))
                })
        })
        .collect();
    logs.sort();
    let excess = logs.len().saturating_sub(MAX_LOG_DAYS);
    for path in logs.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
}

/// Appends to today's file, switching files when the date changes, and drops
/// what would grow the day's file past [`MAX_LOG_BYTES`].
#[derive(Clone)]
struct DailyWriter(Arc<Mutex<WriterState>>);

struct WriterState {
    dir: PathBuf,
    prefix: String,
    date: String,
    file: Option<File>,
    bytes: u64,
}

impl DailyWriter {
    fn new(dir: PathBuf, prefix: String) -> Self {
        Self(Arc::new(Mutex::new(WriterState {
            dir,
            prefix,
            date: String::new(),
            file: None,
            bytes: 0,
        })))
    }
}

impl<'a> MakeWriter<'a> for DailyWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl WriterState {
    fn write(&mut self, date: String, buffer: &[u8]) -> io::Result<usize> {
        if self.date != date {
            let path = self.dir.join(log_name(&self.prefix, &date));
            let file = OpenOptions::new().create(true).append(true).open(path)?;
            self.bytes = file.metadata()?.len();
            self.file = Some(file);
            self.date = date;
            prune_old_logs(&self.dir, &self.prefix);
        }
        let remaining =
            usize::try_from(MAX_LOG_BYTES.saturating_sub(self.bytes)).unwrap_or(usize::MAX);
        if remaining > 0 {
            let file = self
                .file
                .as_mut()
                .ok_or_else(|| io::Error::other("log file unavailable"))?;
            file.write_all(&buffer[..buffer.len().min(remaining)])?;
            self.bytes += u64::try_from(buffer.len().min(remaining)).unwrap_or(u64::MAX);
        }
        // The whole line counts as written, so tracing does not retry what the
        // cap dropped.
        Ok(buffer.len())
    }
}

impl Write for DailyWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("log lock poisoned"))?
            .write(today(), buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("log lock poisoned"))?;
        state.file.as_mut().map_or(Ok(()), Write::flush)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(dir: &Path) -> WriterState {
        WriterState {
            dir: dir.to_owned(),
            prefix: "app".to_owned(),
            date: String::new(),
            file: None,
            bytes: 0,
        }
    }

    #[test]
    fn pruning_keeps_the_newest_days_and_other_files() {
        let dir = tempfile::tempdir().unwrap();
        for day in 1..=10 {
            fs::write(
                dir.path()
                    .join(log_name("app", &format!("2026-07-{day:02}"))),
                "",
            )
            .unwrap();
        }
        fs::write(dir.path().join(log_name("other", "2026-07-01")), "").unwrap();
        fs::write(dir.path().join("app-notes.txt"), "").unwrap();

        prune_old_logs(dir.path(), "app");

        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        let mut expected: Vec<String> = (4..=10)
            .map(|day| log_name("app", &format!("2026-07-{day:02}")))
            .collect();
        expected.push("app-notes.txt".to_owned());
        expected.push(log_name("other", "2026-07-01"));
        expected.sort();
        assert_eq!(left, expected);
    }

    #[test]
    fn a_new_date_starts_a_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = state(dir.path());
        state.write("2026-07-01".to_owned(), b"first\n").unwrap();
        state.write("2026-07-02".to_owned(), b"second\n").unwrap();
        state.file.as_mut().unwrap().flush().unwrap();

        let read = |date: &str| fs::read_to_string(dir.path().join(log_name("app", date))).unwrap();
        assert_eq!(read("2026-07-01"), "first\n");
        assert_eq!(read("2026-07-02"), "second\n");
    }

    #[test]
    fn the_daily_cap_drops_what_would_exceed_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(log_name("app", "2026-07-01"));
        let below_cap = usize::try_from(MAX_LOG_BYTES).unwrap() - 4;
        fs::write(&path, vec![b'x'; below_cap]).unwrap();
        let mut state = state(dir.path());

        assert_eq!(
            state.write("2026-07-01".to_owned(), b"123456\n").unwrap(),
            7
        );
        assert_eq!(state.write("2026-07-01".to_owned(), b"later\n").unwrap(), 6);

        let contents = fs::read(&path).unwrap();
        assert_eq!(contents.len(), usize::try_from(MAX_LOG_BYTES).unwrap());
        assert!(contents.ends_with(b"xxxx1234"));
    }
}
