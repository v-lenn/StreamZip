use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use unrar::Archive;

pub static CANCELLED: AtomicBool = AtomicBool::new(false);

pub fn install_signal_handler() {
    let _ = ctrlc::set_handler(move || {
        eprintln!("\nreceived signal, cleaning up and exiting safely...");
        CANCELLED.store(true, Ordering::SeqCst);
    });
}

pub fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

pub fn check_space(dir: &Path, needed: u64) -> anyhow::Result<()> {
    if let Ok(avail) = fs2::available_space(dir) {
        if avail < needed {
            eprintln!(
                "warning: free space ({} MB) might be less than extracted size ({} MB)",
                avail / (1024 * 1024),
                needed / (1024 * 1024)
            );
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn check_crc(data: &[u8], exp: u32) -> bool {
    if exp == 0 {
        return true;
    }
    let mut h = crc32fast::Hasher::new();
    h.update(data);
    h.finalize() == exp
}

// Zip-Slip security protection (prevents directory traversal outside out_dir)
pub fn sanitize_entry_path(rel_path: &str) -> Option<PathBuf> {
    let path = Path::new(rel_path);
    let mut clean = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(c) => clean.push(c),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
            Component::CurDir => {}
        }
    }
    if clean.as_os_str().is_empty() {
        None
    } else {
        Some(clean)
    }
}

// Pre-flight scan for ZIP payload verification
pub fn verify_zip(p: &Path) -> anyhow::Result<()> {
    let f = std::fs::File::open(p)?;
    let mut archive = zip::ZipArchive::new(f)?;
    let mut buf = vec![0u8; 64 * 1024];

    for i in 0..archive.len() {
        if is_cancelled() {
            anyhow::bail!("verify cancelled by user");
        }
        let mut entry = archive.by_index(i)?;
        let exp_crc = entry.crc32();
        let mut hasher = crc32fast::Hasher::new();

        loop {
            let n = entry.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }

        if exp_crc != 0 && hasher.finalize() != exp_crc {
            anyhow::bail!("pre-flight verification failed for file: {}", entry.name());
        }
    }
    Ok(())
}

// Pre-flight scan for RAR header & entry validation
pub fn verify_rar(p: &Path) -> anyhow::Result<()> {
    let mut open_archive = Archive::new(p).open_for_processing()?;
    while let Some(header) = open_archive.read_header()? {
        if is_cancelled() {
            anyhow::bail!("verify cancelled by user");
        }
        open_archive = header.skip()?;
    }
    Ok(())
}
