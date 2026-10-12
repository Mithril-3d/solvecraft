//! File access for the engine. Natively it is the disk. In the browser (wasm32) there is no
//! file system to call synchronously, so files live in memory here: the web host loads them
//! (from the browser's private storage, a file picker or a drop) before a command reads them,
//! and takes what commands wrote ([`take_writes`]) to store it or hand it out as a download.

use std::io::{Error, ErrorKind, Result};

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    /// At most `max` bytes of a regular file. A character device such as `/dev/zero` reports a
    /// length of 0, so checking the metadata and then reading to the end is unbounded; a FIFO
    /// blocks the caller instead. Refuse anything that is not a regular file, and never read
    /// more than the caller's cap plus the byte that proves it was exceeded.
    pub fn read(path: &str, max: usize) -> Result<Vec<u8>> {
        // Cheap and non-blocking: refuse before opening, in case the open itself would block.
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Err(Error::new(ErrorKind::InvalidInput, "not a regular file"));
        }
        if meta.len() > max as u64 {
            return Err(Error::new(ErrorKind::InvalidData, "file too large"));
        }
        let f = std::fs::File::open(path)?;
        // The path may have been swapped between the two calls; the handle decides.
        if !f.metadata()?.is_file() {
            return Err(Error::new(ErrorKind::InvalidInput, "not a regular file"));
        }
        let mut out = Vec::with_capacity(meta.len().min(16 << 20) as usize);
        std::io::Read::read_to_end(&mut std::io::Read::take(f, max as u64 + 1), &mut out)?;
        if out.len() > max {
            return Err(Error::new(ErrorKind::InvalidData, "file too large"));
        }
        Ok(out)
    }
    pub fn write(path: &str, bytes: &[u8]) -> Result<()> {
        std::fs::write(path, bytes)
    }
    pub fn len(path: &str) -> Result<u64> {
        std::fs::metadata(path).map(|m| m.len())
    }
    pub fn exists(path: &str) -> bool {
        std::path::Path::new(path).exists()
    }
    pub fn remove(path: &str) -> Result<()> {
        std::fs::remove_file(path)
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    thread_local! {
        static FILES: RefCell<BTreeMap<String, Vec<u8>>> = const { RefCell::new(BTreeMap::new()) };
        static WRITES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn missing(path: &str) -> Error {
        Error::new(ErrorKind::NotFound, format!("{path}: no such file in this browser"))
    }

    /// At most `max` bytes, to match the native signature.
    pub fn read(path: &str, max: usize) -> Result<Vec<u8>> {
        let bytes = FILES.with(|f| f.borrow().get(path).cloned()).ok_or_else(|| missing(path))?;
        if bytes.len() > max {
            return Err(Error::new(ErrorKind::InvalidData, "file too large"));
        }
        Ok(bytes)
    }
    pub fn write(path: &str, bytes: &[u8]) -> Result<()> {
        FILES.with(|f| f.borrow_mut().insert(path.to_string(), bytes.to_vec()));
        WRITES.with(|w| {
            let mut w = w.borrow_mut();
            w.retain(|p| p != path);
            w.push(path.to_string());
        });
        Ok(())
    }
    pub fn len(path: &str) -> Result<u64> {
        FILES.with(|f| f.borrow().get(path).map(|b| b.len() as u64)).ok_or_else(|| missing(path))
    }
    pub fn exists(path: &str) -> bool {
        FILES.with(|f| f.borrow().contains_key(path))
    }
    pub fn remove(path: &str) -> Result<()> {
        FILES.with(|f| f.borrow_mut().remove(path)).map(|_| ()).ok_or_else(|| missing(path))
    }
    pub fn insert(path: &str, bytes: Vec<u8>) {
        FILES.with(|f| f.borrow_mut().insert(path.to_string(), bytes));
    }
    pub fn take_writes() -> Vec<(String, Vec<u8>)> {
        let paths = WRITES.with(|w| std::mem::take(&mut *w.borrow_mut()));
        paths.into_iter().filter_map(|p| FILES.with(|f| f.borrow().get(&p).cloned()).map(|b| (p, b))).collect()
    }
    pub fn list(prefix: &str) -> Vec<(String, u64)> {
        FILES.with(|f| f.borrow().iter().filter(|(p, _)| p.starts_with(prefix)).map(|(p, b)| (p.clone(), b.len() as u64)).collect())
    }
}

pub use imp::*;

/// A file's text, at most `max` bytes.
pub fn read_to_string(path: &str, max: usize) -> Result<String> {
    String::from_utf8(read(path, max)?).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    #[test]
    fn files_round_trip() {
        let dir = std::env::temp_dir().join(format!("solvecraft-vfs-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("a.txt").to_string_lossy().to_string();
        assert!(!super::exists(&p));
        assert!(super::read(&p, 16).is_err());
        super::write(&p, b"hello").unwrap();
        assert!(super::exists(&p));
        assert_eq!(super::len(&p).unwrap(), 5);
        assert_eq!(super::read_to_string(&p, 16).unwrap(), "hello");
        // The cap is the caller's, not the file's.
        assert!(super::read(&p, 4).is_err());
        assert_eq!(super::read(&p, 5).unwrap(), b"hello");
        super::remove(&p).unwrap();
        assert!(!super::exists(&p));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `metadata().len()` is 0 for a character device and for a FIFO, so a cap checked against
    /// it passes and the read then runs to an end that never comes. Refuse what is not a
    /// regular file, and do it before opening: opening a FIFO blocks until a writer appears.
    #[cfg(all(unix, not(target_arch = "wasm32")))]
    #[test]
    fn special_files_are_refused_without_blocking() {
        let e = super::read("/dev/zero", 1024).unwrap_err();
        assert!(e.to_string().contains("not a regular file"), "{e}");
        assert!(super::read_to_string("/dev/zero", 1024).is_err());

        let dir = std::env::temp_dir().join(format!("solvecraft-vfs-fifo-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let fifo = dir.join("p");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status().map(|s| s.success()).unwrap_or(false);
        if made {
            let path = fifo.to_string_lossy().to_string();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(super::read(&path, 1024).is_err());
            });
            let refused = rx.recv_timeout(std::time::Duration::from_secs(5));
            assert_eq!(refused, Ok(true), "reading a FIFO must be refused, not block");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory is not a file either.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn directories_are_not_files() {
        let dir = std::env::temp_dir().to_string_lossy().to_string();
        assert!(super::read(&dir, 1024).is_err());
    }
}
