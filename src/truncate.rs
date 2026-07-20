use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

// shift bytes down and truncate tail
pub fn shift_and_truncate(p: &Path, bytes: u64) -> std::io::Result<bool> {
    if bytes == 0 {
        return Ok(false);
    }

    let sz = std::fs::metadata(p)?.len();
    if bytes >= sz {
        std::fs::remove_file(p)?;
        return Ok(true);
    }

    let mut f = OpenOptions::new().read(true).write(true).open(p)?;
    let mut buf = vec![0u8; 4 * 1024 * 1024];

    let mut rpos = bytes;
    let nsz = sz - bytes;

    while rpos < sz {
        f.seek(SeekFrom::Start(rpos))?;
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }

        let wpos = rpos - bytes;
        f.seek(SeekFrom::Start(wpos))?;
        f.write_all(&buf[..n])?;

        rpos += n as u64;
    }

    f.set_len(nsz)?;
    f.sync_all()?;

    Ok(false)
}

// punch sparse hole on ntfs to reclaim physical disk space without shifting offsets
#[cfg(windows)]
pub fn punch_sparse_hole(p: &Path, off: u64, len: u64) -> std::io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        FILE_ZERO_DATA_INFORMATION, FSCTL_SET_SPARSE, FSCTL_SET_ZERO_DATA,
    };

    if len == 0 || !p.exists() {
        return Ok(false);
    }

    let f = OpenOptions::new().write(true).open(p)?;
    let handle = f.as_raw_handle() as _;
    let mut ret_bytes = 0u32;

    // mark as sparse file
    unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_SPARSE,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            &mut ret_bytes,
            std::ptr::null_mut(),
        );
    }

    // zero out and deallocate physical sectors
    let info = FILE_ZERO_DATA_INFORMATION {
        FileOffset: off as i64,
        BeyondFinalZero: (off + len) as i64,
    };

    let res = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_ZERO_DATA,
            &info as *const _ as *const _,
            std::mem::size_of::<FILE_ZERO_DATA_INFORMATION>() as u32,
            std::ptr::null_mut(),
            0,
            &mut ret_bytes,
            std::ptr::null_mut(),
        )
    };

    Ok(res != 0)
}

#[cfg(not(windows))]
pub fn punch_sparse_hole(_p: &Path, _off: u64, _len: u64) -> std::io::Result<bool> {
    Ok(false)
}

// stream reader for single file truncation
#[allow(dead_code)]
pub struct ShiftReader {
    p: PathBuf,
    chunk_sz: usize,
    buf: Vec<u8>,
    pos: usize,
    len: usize,
    eof: bool,
    first: bool,
}

#[allow(dead_code)]
impl ShiftReader {
    pub fn new(p: PathBuf, chunk_sz: usize) -> std::io::Result<Self> {
        Ok(Self {
            p,
            chunk_sz: chunk_sz.max(1024 * 1024),
            buf: Vec::new(),
            pos: 0,
            len: 0,
            eof: false,
            first: true,
        })
    }

    fn next_chunk(&mut self) -> std::io::Result<()> {
        if self.eof {
            return Ok(());
        }

        if !self.first && self.len > 0 {
            let _ = shift_and_truncate(&self.p, self.len as u64);
        }

        self.first = false;

        if !self.p.exists() {
            self.eof = true;
            self.len = 0;
            self.pos = 0;
            return Ok(());
        }

        let mut f = match OpenOptions::new().read(true).open(&self.p) {
            Ok(f) => f,
            Err(_) => {
                self.eof = true;
                self.len = 0;
                self.pos = 0;
                return Ok(());
            }
        };

        let mut tmp = vec![0u8; self.chunk_sz];
        let n = f.read(&mut tmp)?;

        if n == 0 {
            self.eof = true;
            self.len = 0;
            self.pos = 0;
            let _ = std::fs::remove_file(&self.p);
        } else {
            tmp.truncate(n);
            self.buf = tmp;
            self.pos = 0;
            self.len = n;
        }

        Ok(())
    }
}

impl Read for ShiftReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len {
            self.next_chunk()?;
        }

        if self.eof || self.len == 0 {
            return Ok(0);
        }

        let avail = self.len - self.pos;
        let n = avail.min(out.len());
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;

        Ok(n)
    }
}
