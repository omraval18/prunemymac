//! Directory listing used by every scanner. On macOS one
//! `getattrlistbulk(2)` call returns names, types and allocated sizes for
//! many entries at once, instead of one `lstat` per file.

#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// One directory listing. Names live in a shared byte arena so files that
/// end up folded into the summary never get their own allocation.
#[derive(Default)]
pub(crate) struct Listing {
    names: Vec<u8>,
    pub(crate) entries: Vec<Entry>,
}

pub(crate) struct Entry {
    name: (u32, u32),
    pub(crate) kind: Kind,
    pub(crate) dev: i32,
    /// Allocated bytes on disk (files only).
    pub(crate) size: u64,
    /// Logical length; larger than `size` for sparse files.
    pub(crate) len: u64,
}

impl Listing {
    pub(crate) fn name(&self, e: &Entry) -> &str {
        std::str::from_utf8(&self.names[e.name.0 as usize..(e.name.0 + e.name.1) as usize]).unwrap_or("?")
    }
    fn push(&mut self, name: &[u8], kind: Kind, dev: i32, size: u64, len: u64) {
        let start = self.names.len() as u32;
        // Lossy conversion keeps the arena valid UTF-8.
        self.names.extend_from_slice(String::from_utf8_lossy(name).as_bytes());
        let name = (start, self.names.len() as u32 - start);
        self.entries.push(Entry { name, kind, dev, size, len });
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum Kind {
    File,
    Dir,
    Other,
}

#[cfg(target_os = "macos")]
const ATTR_CMN_ERROR: u32 = 0x2000_0000;
#[cfg(target_os = "macos")]
const VREG: u32 = 1;
#[cfg(target_os = "macos")]
const VDIR: u32 = 2;

/// Read one directory with `getattrlistbulk`. Entries are packed as:
/// length, returned-attrs set, [error], name ref, devid, objtype, [allocsize].
#[cfg(target_os = "macos")]
pub(crate) fn read_bulk(path: &Path) -> std::io::Result<Listing> {
    let cpath = CString::new(path.as_os_str().as_bytes())?;
    let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    struct Fd(i32);
    impl Drop for Fd {
        fn drop(&mut self) {
            unsafe { libc::close(self.0) };
        }
    }
    let _guard = Fd(fd);

    let mut attrs = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS | ATTR_CMN_ERROR | libc::ATTR_CMN_NAME | libc::ATTR_CMN_DEVID | libc::ATTR_CMN_OBJTYPE,
        volattr: 0,
        dirattr: 0,
        fileattr: libc::ATTR_FILE_TOTALSIZE | libc::ATTR_FILE_ALLOCSIZE,
        forkattr: 0,
    };
    // One reusable buffer per thread: zeroing a fresh 64 KB per directory
    // costs gigabytes of memset over a full scan.
    thread_local! {
        static BUF: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(vec![0u8; 64 * 1024]);
    }
    BUF.with_borrow_mut(|buf| read_into(fd, &mut attrs, buf))
}

#[cfg(target_os = "macos")]
fn read_into(fd: i32, attrs: &mut libc::attrlist, buf: &mut [u8]) -> std::io::Result<Listing> {
    let mut out = Listing::default();
    loop {
        let n = unsafe {
            libc::getattrlistbulk(fd, attrs as *mut _ as *mut _, buf.as_mut_ptr() as *mut _, buf.len(), 0)
        };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if n == 0 {
            break;
        }
        let mut off = 0usize;
        for _ in 0..n {
            let entry = &buf[off..];
            let len = u32::from_ne_bytes(entry[0..4].try_into().unwrap()) as usize;
            let rd = |at: usize| u32::from_ne_bytes(entry[at..at + 4].try_into().unwrap());
            let common = rd(4);
            let fileattr = rd(16);
            let mut p = 24;
            let mut failed = false;
            if common & ATTR_CMN_ERROR != 0 {
                failed = rd(p) != 0;
                p += 4;
            }
            let mut name: &[u8] = b"";
            if common & libc::ATTR_CMN_NAME != 0 {
                let data_off = i32::from_ne_bytes(entry[p..p + 4].try_into().unwrap());
                let start = (p as isize + data_off as isize) as usize;
                if let Ok(c) = CStr::from_bytes_until_nul(&entry[start..len]) {
                    name = c.to_bytes();
                }
                p += 8;
            }
            let mut dev = 0;
            if common & libc::ATTR_CMN_DEVID != 0 {
                dev = rd(p) as i32;
                p += 4;
            }
            let mut kind = Kind::Other;
            if common & libc::ATTR_CMN_OBJTYPE != 0 {
                kind = match rd(p) {
                    VREG => Kind::File,
                    VDIR => Kind::Dir,
                    _ => Kind::Other,
                };
                p += 4;
            }
            let rd64 = |at: usize| u64::from_ne_bytes(entry[at..at + 8].try_into().unwrap());
            let mut file_len = 0;
            if fileattr & libc::ATTR_FILE_TOTALSIZE != 0 {
                file_len = rd64(p);
                p += 8;
            }
            let mut size = 0;
            if fileattr & libc::ATTR_FILE_ALLOCSIZE != 0 {
                size = rd64(p);
            }
            if !failed && !name.is_empty() {
                // Length is only meaningful for regular files (symlinks report their target string).
                out.push(name, kind, dev, size, if kind == Kind::File { file_len } else { 0 });
            }
            off += len;
        }
    }
    Ok(out)
}

/// Portable listing. On Linux, trust `d_type` when the kernel provides it so
/// sockets/fifos/symlinks skip `lstat`; files and dirs still need metadata for
/// size and device id (same-filesystem checks).
pub(crate) fn read_std(path: &Path) -> std::io::Result<Listing> {
    let mut out = Listing::default();
    for e in std::fs::read_dir(path)?.flatten() {
        let name = e.file_name();
        #[cfg(target_os = "linux")]
        {
            if let Ok(ft) = e.file_type() {
                if ft.is_symlink() || (!ft.is_dir() && !ft.is_file()) {
                    out.push(name.as_bytes(), Kind::Other, 0, 0, 0);
                    continue;
                }
            }
        }
        let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
        let kind = if m.is_dir() { Kind::Dir } else if m.is_file() { Kind::File } else { Kind::Other };
        out.push(name.as_bytes(), kind, m.dev() as i32, m.blocks() * 512, if m.is_file() { m.len() } else { 0 });
    }
    Ok(out)
}

/// Fast listing with a portable fallback.
pub(crate) fn read_entries(path: &Path) -> std::io::Result<Listing> {
    #[cfg(target_os = "macos")]
    { read_bulk(path).or_else(|_| read_std(path)) }
    #[cfg(not(target_os = "macos"))]
    { read_std(path) }
}

