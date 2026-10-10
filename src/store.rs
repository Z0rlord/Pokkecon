use crate::model::Signal;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub trait Store {
    fn put(&mut self, s: Signal) -> std::io::Result<()>;
    fn all(&self) -> Vec<Signal>;
}

#[derive(Default)]
pub struct MemStore(Vec<Signal>);

impl Store for MemStore {
    fn put(&mut self, s: Signal) -> std::io::Result<()> {
        self.0.push(s);
        Ok(())
    }
    fn all(&self) -> Vec<Signal> {
        self.0.clone()
    }
}

/// Signals persisted as JSON lines: one signal per line, appended on `put`.
/// Keeps the same data in memory; the file is the source of truth across runs.
pub struct FileStore {
    path: PathBuf,
    signals: Vec<Signal>,
}

impl FileStore {
    /// Open a store at `path`, creating the file on first write. A corrupt
    /// line fails the whole load instead of silently dropping history.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut signals = Vec::new();
        if let Ok(f) = File::open(&path) {
            for line in BufReader::new(f).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let s = serde_json::from_str(&line)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                signals.push(s);
            }
        }
        Ok(Self { path, signals })
    }
}

impl Store for FileStore {
    fn put(&mut self, s: Signal) -> std::io::Result<()> {
        let line = serde_json::to_string(&s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut f = OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{}", line)?;
        self.signals.push(s);
        Ok(())
    }
    fn all(&self) -> Vec<Signal> {
        self.signals.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use std::time::{Duration, SystemTime};

    fn sig(id: u64, who: &str) -> Signal {
        Signal {
            id,
            who: Handle(who.into()),
            kind: Kind::Offer,
            category: Category::Lend,
            cell: Cell("a".into()),
            expires: SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000),
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pokkecon-test-{}-{}-{}.jsonl", tag, std::process::id(), nanos))
    }

    #[test]
    fn file_store_roundtrips_signals() {
        let path = temp_path("roundtrip");
        {
            let mut s = FileStore::open(&path).unwrap();
            s.put(sig(1, "ann")).unwrap();
            s.put(sig(2, "bo")).unwrap();
        }
        let s = FileStore::open(&path).unwrap();
        assert_eq!(s.all(), vec![sig(1, "ann"), sig(2, "bo")]);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn corrupt_line_fails_the_load() {
        let path = temp_path("corrupt");
        {
            let mut s = FileStore::open(&path).unwrap();
            s.put(sig(1, "ann")).unwrap();
        }
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "not json").unwrap();
        drop(f);
        assert!(FileStore::open(&path).is_err());
        std::fs::remove_file(&path).ok();
    }
}
