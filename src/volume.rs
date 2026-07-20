use regex::Regex;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

// clean up volume extensions
pub fn robust_basename_split(name: &str) -> String {
    let pats = [
        r"(?i)(.*)\.part\d+$",
        r"(?i)(.*)\.z\d+$",
        r"(?i)(.*)\.r\d+$",
        r"(?i)(.*)\.zip$",
        r"(?i)(.*)\.rar$",
        r"(?i)(.*)\.\d+$",
        r"(?i)(.*)\.tar\.gz$",
        r"(?i)(.*)\.tar$",
        r"(?i)(.*)\.gz$",
        r"(?i)(.*)\.ZIP\.\d+$",
        r"(?i)(.*)\.ZIP\.Z\d+$",
        r"(?i)(.*)\.RAR\.\d+$",
        r"(?i)(.*)\.RAR\.PART\d+$",
        r"(?i)(.*)\.PART\d+\.RAR$",
        r"(?i)(.*)\.7z\.\d+$",
    ];

    let mut cur = name.to_string();
    let mut loop_again = true;

    while loop_again {
        loop_again = false;
        for p in &pats {
            if let Ok(re) = Regex::new(p) {
                if let Some(caps) = re.captures(&cur) {
                    if let Some(mat) = caps.get(1) {
                        cur = mat.as_str().to_string();
                        loop_again = true;
                        break;
                    }
                }
            }
        }
    }

    cur
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Str(String),
    Num(u64),
}

// natural sort helper
pub fn natural_sort_key(p: &Path) -> Vec<Part> {
    let fname = p.file_name().and_then(OsStr::to_str).unwrap_or("").to_lowercase();
    let re = Regex::new(r"(\d+)").unwrap();
    let mut res = Vec::new();
    let mut last = 0;

    for m in re.find_iter(&fname) {
        if m.start() > last {
            res.push(Part::Str(fname[last..m.start()].to_string()));
        }
        if let Ok(n) = m.as_str().parse::<u64>() {
            res.push(Part::Num(n));
        } else {
            res.push(Part::Str(m.as_str().to_string()));
        }
        last = m.end();
    }

    if last < fname.len() {
        res.push(Part::Str(fname[last..].to_string()));
    }

    res
}

// find matching volume files in directory
pub fn discover_volumes(p: &Path) -> std::io::Result<Vec<PathBuf>> {
    let dir = p.parent().unwrap_or_else(|| Path::new("."));
    let fname = p.file_name().and_then(OsStr::to_str).unwrap_or("");

    let base = robust_basename_split(fname);
    let esc = regex::escape(&base);

    let pats = vec![
        format!(r"(?i)^{}\.z\d+$", esc),
        format!(r"(?i)^{}\.zip$", esc),
        format!(r"(?i)^{}\.zip\.\d+$", esc),
        format!(r"(?i)^{}\.r\d+$", esc),
        format!(r"(?i)^{}\.rar$", esc),
        format!(r"(?i)^{}\.rar\.\d+$", esc),
        format!(r"(?i)^{}\.part\d+\.rar$", esc),
    ];

    let regexes: Vec<Regex> = pats.iter().filter_map(|x| Regex::new(x).ok()).collect();
    let mut vols = Vec::new();

    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                if let Some(f) = path.file_name().and_then(OsStr::to_str) {
                    if regexes.iter().any(|re| re.is_match(f)) {
                        vols.push(path);
                    }
                }
            }
        }
    }

    vols.sort_by(|a, b| natural_sort_key(a).cmp(&natural_sort_key(b)));

    if vols.is_empty() {
        vols.push(p.to_path_buf());
    }

    Ok(vols)
}
