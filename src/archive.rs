use crate::journal::{Journal, JournalFlusher};
use crate::safety::{is_cancelled, sanitize_entry_path};
use crate::truncate::shift_and_truncate;
use crate::ui::ProgressState;
use flate2::read::GzDecoder;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tar::Archive as TarArchive;
use zip::ZipArchive;

pub fn extract_zip_single(
    p: &Path,
    out_dir: &Path,
    password: Option<&str>,
    chunk_sz: u64,
    j: &mut Journal,
    state: &ProgressState,
    keep: bool,
) -> anyhow::Result<()> {
    let f = File::open(p)?;
    let mut zip = ZipArchive::new(f)?;
    let total_entries = zip.len();

    let flusher = JournalFlusher::start();
    let mut last_save = Instant::now();
    let mut uncompressed_since_last_trunc = 0u64;

    for i in 0..total_entries {
        if is_cancelled() {
            flusher.queue(j);
            anyhow::bail!("extraction cancelled by user");
        }

        let mut entry = if let Some(pass) = password {
            zip.by_index_decrypt(i, pass.as_bytes())?
        } else {
            zip.by_index(i)?
        };

        let name = entry.name().to_string();
        if name.ends_with('/') || name.ends_with('\\') {
            if let Some(clean_dir) = sanitize_entry_path(&name) {
                fs::create_dir_all(out_dir.join(clean_dir))?;
            }
            continue;
        }

        if j.is_done(&name) {
            continue;
        }

        let clean_rel = match sanitize_entry_path(&name) {
            Some(path) => path,
            None => {
                state.add_log(&format!("skipped unsafe zip-slip entry: {}", name));
                continue;
            }
        };

        let out_path = out_dir.join(&clean_rel);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let exp_crc = entry.crc32();

        let mut out_file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&out_path)?;

        let mut buf = vec![0u8; 64 * 1024];
        let mut written = 0u64;
        let mut hasher = crc32fast::Hasher::new();

        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out_file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            written += n as u64;
            state.inc_bytes(n as u64);
        }

        // Use sync_data for fast data durability without expensive filesystem metadata syncs
        let _ = out_file.sync_data();

        if exp_crc != 0 && hasher.finalize() != exp_crc {
            anyhow::bail!("crc check failed for file: {}", name);
        }

        j.mark_done(&name);
        state.inc_file_count();
        state.add_log(&format!("extracted: {}", name));

        uncompressed_since_last_trunc += written;

        // Save journal every 2 seconds or on the last file
        if last_save.elapsed() >= Duration::from_secs(2) || i == total_entries - 1 {
            flusher.queue(j);
            last_save = Instant::now();
        }

        if !keep && chunk_sz > 0 && uncompressed_since_last_trunc >= chunk_sz {
            if let Ok(true) = shift_and_truncate(p, uncompressed_since_last_trunc) {
                state.add_reclaimed(uncompressed_since_last_trunc);
                j.add_trunc(uncompressed_since_last_trunc);
            }
            flusher.queue(j);
            uncompressed_since_last_trunc = 0;
        }
    }

    flusher.queue(j);
    flusher.finish();

    if !keep && p.exists() {
        if let Ok(meta) = fs::metadata(p) {
            state.add_reclaimed(meta.len());
        }
        let _ = fs::remove_file(p);
    }

    Ok(())
}

pub fn extract_zip_multi(
    vols: &[PathBuf],
    out_dir: &Path,
    password: Option<&str>,
    j: &mut Journal,
    state: &ProgressState,
    keep: bool,
) -> anyhow::Result<()> {
    for (idx, vol) in vols.iter().enumerate() {
        if is_cancelled() {
            j.save()?;
            anyhow::bail!("extraction cancelled");
        }

        if !vol.exists() {
            continue;
        }

        let res = extract_zip_single(vol, out_dir, password, 0, j, state, true);
        if let Err(e) = res {
            state.add_log(&format!("error extracting volume {:?}: {}", vol.file_name().unwrap_or_default(), e));
            return Err(e);
        }

        if !keep && idx < vols.len() - 1 {
            if let Ok(meta) = fs::metadata(vol) {
                state.add_reclaimed(meta.len());
            }
            if fs::remove_file(vol).is_ok() {
                j.add_deleted_vol(vol);
                j.save()?;
            }
        }
    }

    if !keep {
        if let Some(last) = vols.last() {
            if last.exists() {
                if let Ok(meta) = fs::metadata(last) {
                    state.add_reclaimed(meta.len());
                }
                let _ = fs::remove_file(last);
            }
        }
    }

    Ok(())
}

pub fn extract_tar_gz(
    p: &Path,
    out_dir: &Path,
    j: &mut Journal,
    state: &ProgressState,
    keep: bool,
) -> anyhow::Result<()> {
    let f = File::open(p)?;
    let gz = GzDecoder::new(f);
    let mut tar = TarArchive::new(gz);

    for entry in tar.entries()? {
        if is_cancelled() {
            j.save()?;
            anyhow::bail!("extraction cancelled");
        }

        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let name = path.to_string_lossy().to_string();

        if j.is_done(&name) {
            continue;
        }

        let sz = entry.size();
        entry.unpack_in(out_dir)?;
        j.mark_done(&name);

        state.inc_bytes(sz);
        state.inc_file_count();
        state.add_log(&format!("extracted: {}", name));
    }

    j.save()?;
    if !keep && p.exists() {
        if let Ok(meta) = fs::metadata(p) {
            state.add_reclaimed(meta.len());
        }
        let _ = fs::remove_file(p);
    }
    Ok(())
}
