use crate::journal::{Journal, JournalFlusher};
use crate::safety::is_cancelled;
use crate::truncate::punch_sparse_hole;
use crate::ui::ProgressState;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use unrar::Archive;

pub fn extract_rar(
    vols: &[PathBuf],
    out_dir: &Path,
    password: Option<&str>,
    j: &mut Journal,
    state: &ProgressState,
    keep: bool,
) -> anyhow::Result<()> {
    let first = match vols.first() {
        Some(v) => v,
        None => anyhow::bail!("no rar volume specified"),
    };

    let mut archive = if let Some(pass) = password {
        Archive::with_password(first, pass).open_for_processing()?
    } else {
        Archive::new(first).open_for_processing()?
    };

    let flusher = JournalFlusher::start();
    let mut last_save = Instant::now();
    let is_single = vols.len() == 1;
    let mut ext_bytes = 0u64;
    let mut hole_off = 0u64;

    while let Some(header) = archive.read_header()? {
        if is_cancelled() {
            flusher.queue(j);
            anyhow::bail!("extraction cancelled");
        }

        let is_dir = header.entry().is_directory();
        let name = header.entry().filename.to_string_lossy().to_string();
        let sz = header.entry().unpacked_size as u64;

        if is_dir {
            fs::create_dir_all(out_dir.join(&name))?;
            archive = header.skip()?;
            continue;
        }

        if j.is_done(&name) {
            archive = header.skip()?;
            continue;
        }

        let dest = out_dir.join(&name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        archive = header.extract_to(&dest)?;
        j.mark_done(&name);

        ext_bytes += sz;
        state.inc_bytes(sz);
        state.inc_file_count();
        state.add_log(&format!("extracted: {}", name));

        // save journal every 2s in background
        if last_save.elapsed() >= Duration::from_secs(2) {
            flusher.queue(j);
            last_save = Instant::now();
        }

        // for single rar, punch ntfs sparse hole to reclaim physical disk sectors every 256MB
        if is_single && !keep && ext_bytes >= 256 * 1024 * 1024 {
            if let Ok(true) = punch_sparse_hole(first, hole_off, ext_bytes) {
                state.add_reclaimed(ext_bytes);
                j.add_trunc(ext_bytes);
                hole_off += ext_bytes;
                ext_bytes = 0;
            }
        }
    }

    flusher.queue(j);
    flusher.finish();

    if !keep {
        for vol in vols {
            if vol.exists() {
                if let Ok(meta) = fs::metadata(vol) {
                    state.add_reclaimed(meta.len());
                }
                let _ = fs::remove_file(vol);
            }
        }
    }

    j.save()?;
    Ok(())
}
