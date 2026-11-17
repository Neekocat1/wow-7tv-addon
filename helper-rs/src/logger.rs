//! helper.log: "2026-11-17 02:45:01,123 INFO message", rotated at 1 MB with two backups
//! (same format and policy as the Python helper's RotatingFileHandler).

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_BYTES: u64 = 1_000_000;
const BACKUPS: u32 = 2;

struct Logger {
    path: PathBuf,
    file: Mutex<Option<File>>,
    console: bool,
}

fn level_name(l: Level) -> &'static str {
    match l {
        Level::Error => "ERROR",
        Level::Warn => "WARNING",
        Level::Info => "INFO",
        Level::Debug => "DEBUG",
        Level::Trace => "TRACE",
    }
}

impl Logger {
    fn rotate(&self, slot: &mut Option<File>) {
        *slot = None;
        for i in (1..BACKUPS).rev() {
            let from = self.path.with_extension(format!("log.{i}"));
            let to = self.path.with_extension(format!("log.{}", i + 1));
            let _ = std::fs::rename(&from, &to);
        }
        let _ = std::fs::rename(&self.path, self.path.with_extension("log.1"));
    }

    fn open(&self) -> Option<File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .ok()
    }
}

impl Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= Level::Info
            && (m.target().starts_with("seventv_helper")
                || m.target().starts_with("SevenTVEmotesHelper"))
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {} {}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S,%3f"),
            level_name(record.level()),
            record.args()
        );
        if self.console {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        let mut slot = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            *slot = self.open();
        }
        let too_big = slot
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .is_some_and(|m| m.len() + line.len() as u64 > MAX_BYTES);
        if too_big {
            self.rotate(&mut slot);
            *slot = self.open();
        }
        if let Some(f) = slot.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = f.flush();
        }
    }
}

/// Install the logger, plus a panic hook that writes panics to the log.
pub fn init(path: PathBuf, console: bool) {
    let logger = Logger {
        path,
        file: Mutex::new(None),
        console,
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default(info);
    }));
}
