use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::thread::{self, JoinHandle};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub archive_path: PathBuf,
    pub journal_file: PathBuf,
    pub orig_size: u64,
    pub hash: String,
    pub extracted: HashSet<String>,
    pub truncated: u64,
    pub deleted_vols: Vec<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl Journal {
    pub fn get_jpath(p: &Path) -> PathBuf {
        let parent = p.parent().unwrap_or_else(|| Path::new("."));
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("archive");
        parent.join(format!(".streamzip_journal_{}.json", name))
    }

    pub fn get_hash(p: &Path) -> std::io::Result<String> {
        if !p.exists() {
            return Ok("none".to_string());
        }
        let meta = std::fs::metadata(p)?;
        let sz = meta.len();
        let mut buf = vec![0u8; 4096];
        let mut f = File::open(p)?;
        let n = f.read(&mut buf)?;

        let mut crc = crc32fast::Hasher::new();
        crc.update(&sz.to_le_bytes());
        crc.update(&buf[..n]);
        Ok(format!("{:x}", crc.finalize()))
    }

    pub fn new(p: &Path) -> anyhow::Result<Self> {
        let jfile = Self::get_jpath(p);
        let orig_size = if p.exists() {
            std::fs::metadata(p)?.len()
        } else {
            0
        };
        let hash = Self::get_hash(p).unwrap_or_default();

        let j = Self {
            archive_path: p.to_path_buf(),
            journal_file: jfile,
            orig_size,
            hash,
            extracted: HashSet::new(),
            truncated: 0,
            deleted_vols: Vec::new(),
            timestamp: chrono::Utc::now(),
        };

        j.save()?;
        Ok(j)
    }

    pub fn load_or_create(p: &Path) -> anyhow::Result<Self> {
        let jpath = Self::get_jpath(p);
        if jpath.exists() {
            if let Ok(raw) = std::fs::read_to_string(&jpath) {
                if let Ok(j) = serde_json::from_str::<Journal>(&raw) {
                    let current_hash = Self::get_hash(p).unwrap_or_default();
                    if p.exists() && !j.hash.is_empty() && j.hash != current_hash && j.hash != "none" {
                        eprintln!("warning: source archive changed since last session! resetting journal.");
                        return Self::new(p);
                    }
                    return Ok(j);
                }
            }
        }
        Self::new(p)
    }

    pub fn is_done(&self, rel: &str) -> bool {
        self.extracted.contains(rel)
    }

    pub fn mark_done(&mut self, rel: &str) {
        self.extracted.insert(rel.to_string());
    }

    pub fn add_trunc(&mut self, bytes: u64) {
        self.truncated += bytes;
    }

    pub fn add_deleted_vol(&mut self, vol: &Path) {
        if let Some(s) = vol.file_name().and_then(|x| x.to_str()) {
            if !self.deleted_vols.contains(&s.to_string()) {
                self.deleted_vols.push(s.to_string());
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let data = serde_json::to_string_pretty(self)?;
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&self.journal_file)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
        Ok(())
    }

    pub fn cleanup(&self) -> std::io::Result<()> {
        if self.journal_file.exists() {
            std::fs::remove_file(&self.journal_file)?;
        }
        Ok(())
    }
}

pub struct JournalFlusher {
    tx: Sender<Journal>,
    handle: Option<JoinHandle<()>>,
}

impl JournalFlusher {
    pub fn start() -> Self {
        let (tx, rx) = channel::<Journal>();
        let handle = thread::spawn(move || {
            while let Ok(j) = rx.recv() {
                let _ = j.save();
            }
        });

        Self {
            tx,
            handle: Some(handle),
        }
    }

    pub fn queue(&self, j: &Journal) {
        let _ = self.tx.send(j.clone());
    }

    pub fn finish(mut self) {
        drop(self.tx);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
