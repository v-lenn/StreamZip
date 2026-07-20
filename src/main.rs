mod archive;
mod journal;
mod rar;
mod safety;
mod truncate;
mod ui;
mod volume;

use clap::Parser;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use ui::{AppUi, ProgressState};

#[derive(Parser, Debug)]
#[command(
    name = "strzip",
    author,
    version,
    about = "Streaming extractor with delete-as-you-go disk space optimization"
)]
struct Args {
    /// path to main archive or first volume file
    #[arg(required = true)]
    archive: PathBuf,

    /// chunk size in MB for truncation
    #[arg(short = 'c', long, default_value_t = 512)]
    chunk_size: u64,

    /// archive password
    #[arg(short = 'p', long)]
    password: Option<String>,

    /// output directory (defaults to parent_dir/archive_basename)
    #[arg(short = 'o', long)]
    output: Option<PathBuf>,

    /// verify integrity before starting extraction
    #[arg(long, default_value_t = false)]
    verify_first: bool,

    /// keep archive files (do not delete while extracting)
    #[arg(short = 'k', long, default_value_t = false)]
    keep: bool,

    /// ignore previous session journal and force fresh start from scratch
    #[arg(long, default_value_t = false)]
    clean: bool,

    /// keep journal file after successful extraction
    #[arg(long, default_value_t = false)]
    keep_journal: bool,

    /// disable live per-file logging to maximize extraction speed on archives with thousands of small files
    #[arg(long, default_value_t = false)]
    no_log: bool,

    /// disable full TUI dashboard and use plain terminal text
    #[arg(long, default_value_t = false)]
    no_tui: bool,
}

fn clean_path_display(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        s.to_string()
    }
}

fn detect_format_by_magic(p: &Path) -> Option<String> {
    if let Ok(mut f) = std::fs::File::open(p) {
        let mut buf = [0u8; 8];
        let n = f.read(&mut buf).unwrap_or(0);
        if n >= 4 {
            // zip: PK\x03\x04
            if buf[0] == 0x50 && buf[1] == 0x4B && buf[2] == 0x03 && buf[3] == 0x04 {
                return Some("ZIP".to_string());
            }
            // rar4/rar5: Rar! (0x52 0x61 0x72 0x21)
            if buf[0] == 0x52 && buf[1] == 0x61 && buf[2] == 0x72 && buf[3] == 0x21 {
                return Some("RAR".to_string());
            }
            // 7z: 7z\xBC\xAF
            if buf[0] == 0x37 && buf[1] == 0x7A && buf[2] == 0xBC && buf[3] == 0xAF {
                return Some("7Z".to_string());
            }
            // tar.gz / gzip: \x1f\x8b
            if buf[0] == 0x1F && buf[1] == 0x8B {
                return Some("GZIP".to_string());
            }
            eprintln!(
                "unknown magic bytes: {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X} {:02X}",
                buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]
            );
        }
    }
    None
}

fn main() -> anyhow::Result<()> {
    safety::install_signal_handler();

    let args = Args::parse();
    let archive_path = args.archive.canonicalize().unwrap_or(args.archive);

    if !archive_path.exists() {
        anyhow::bail!("archive file does not exist: {:?}", archive_path);
    }

    let vols = volume::discover_volumes(&archive_path)?;

    let fname = archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("extracted");
    let base_folder = volume::robust_basename_split(fname);

    let out_dir = match args.output {
        Some(d) => d,
        None => {
            let parent = archive_path.parent().unwrap_or_else(|| Path::new("."));
            parent.join(&base_folder)
        }
    };

    std::fs::create_dir_all(&out_dir)?;

    let fname_lower = fname.to_lowercase();
    let magic_fmt = detect_format_by_magic(&archive_path);

    let is_rar = match magic_fmt.as_deref() {
        Some("RAR") => true,
        Some("ZIP") => false,
        _ => {
            fname_lower.ends_with(".rar")
                || fname_lower.contains(".part")
                || vols.iter().any(|v| {
                    let s = v.to_string_lossy().to_lowercase();
                    s.ends_with(".rar") || s.contains(".r0") || s.contains(".r1")
                })
        }
    };

    let is_tar_gz = fname_lower.ends_with(".tar.gz") || fname_lower.ends_with(".tgz");

    let mode_str = if is_rar {
        "RAR"
    } else if is_tar_gz {
        "TAR.GZ"
    } else {
        "ZIP"
    };

    let total_sz: u64 = vols
        .iter()
        .filter_map(|v| std::fs::metadata(v).ok().map(|m| m.len()))
        .sum();

    let _ = safety::check_space(&out_dir, total_sz);

    let state = Arc::new(ProgressState::new(
        fname.to_string(),
        mode_str.to_string(),
        total_sz,
        args.no_log,
    ));

    let ui_thread = if !args.no_tui {
        let state_clone = Arc::clone(&state);
        Some(thread::spawn(move || {
            if let Ok(mut ui) = AppUi::new() {
                while !state_clone.is_finished.load(Ordering::Relaxed) && !safety::is_cancelled() {
                    let _ = ui.render(&state_clone);
                    if ui.handle_events() {
                        safety::CANCELLED.store(true, Ordering::SeqCst);
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                ui.restore();
            }
        }))
    } else {
        println!("⚡ StreamZip v{}", env!("CARGO_PKG_VERSION"));
        println!("archive: {:?}", fname);
        println!("found {} volume(s)", vols.len());
        println!("mode: {}", mode_str);
        None
    };

    if args.clean {
        let jpath = journal::Journal::get_jpath(&archive_path);
        if jpath.exists() {
            let _ = std::fs::remove_file(&jpath);
            state.add_log("cleaned previous journal session");
        }
    }

    if args.verify_first {
        state.set_status("Verifying...");
        state.add_log("running pre-flight verification scan...");
        if is_rar {
            safety::verify_rar(&archive_path)?;
            state.add_log("pre-flight rar verification passed OK");
        } else {
            safety::verify_zip(&archive_path)?;
            state.add_log("pre-flight zip verification passed OK");
        }
    }

    state.set_status("Extracting...");

    let mut j = journal::Journal::load_or_create(&archive_path)?;
    if !args.clean && !j.extracted.is_empty() {
        state.add_log(&format!(
            "resuming previous session ({} files done)",
            j.extracted.len()
        ));
    }

    let chunk_bytes = args.chunk_size * 1024 * 1024;
    let pass = args.password.as_deref();

    let res = if is_rar {
        rar::extract_rar(&vols, &out_dir, pass, &mut j, &state, args.keep)
    } else if is_tar_gz {
        archive::extract_tar_gz(&archive_path, &out_dir, &mut j, &state, args.keep)
    } else if vols.len() > 1 {
        archive::extract_zip_multi(&vols, &out_dir, pass, &mut j, &state, args.keep)
    } else {
        archive::extract_zip_single(
            &archive_path,
            &out_dir,
            pass,
            chunk_bytes,
            &mut j,
            &state,
            args.keep,
        )
    };

    state.is_finished.store(true, Ordering::Relaxed);
    if let Some(t) = ui_thread {
        let _ = t.join();
    }

    match res {
        Ok(_) => {
            if !args.keep_journal {
                j.cleanup()?;
            }
            let display_out = clean_path_display(&out_dir);
            let files_cnt = state.file_count.load(Ordering::Relaxed);
            let reclaimed = state.reclaimed_bytes.load(Ordering::Relaxed) as f64 / 1_048_576.0;

            println!("\n=======================================================");
            println!(" 🎉 EXTRACTION COMPLETE");
            println!("=======================================================");
            println!(" 📁 Output Folder : {}", display_out);
            println!(" 📄 Files Count   : {}", files_cnt);
            println!(" 🗑️ Disk Reclaimed: {:.2} MB", reclaimed);
            println!("=======================================================\n");
        }
        Err(e) => {
            eprintln!("❌ Extraction stopped: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}
