use std::{
    fs::{File, OpenOptions},
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::config::AppConfig;

pub struct DaemonLogger {
    file: Option<File>,
}

impl DaemonLogger {
    pub fn new() -> Self {
        let file = AppConfig::log_path().ok().and_then(|path| {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        });

        Self { file }
    }

    pub fn info(&mut self, message: impl AsRef<str>) {
        self.write("INFO", message.as_ref());
    }

    pub fn warn(&mut self, message: impl AsRef<str>) {
        self.write("WARN", message.as_ref());
    }

    pub fn error(&mut self, message: impl AsRef<str>) {
        self.write("ERROR", message.as_ref());
    }

    fn write(&mut self, level: &str, message: &str) {
        eprintln!("[{level}] {message}");

        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "[{}][{level}] {message}", unix_timestamp_secs());
            let _ = file.flush();
        }
    }
}

fn unix_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
