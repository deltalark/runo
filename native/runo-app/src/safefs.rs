//! Files Runo reads and writes in folders something else can change too, such as a
//! session folder shared with a Cove VM. Paths are plain names under a root folder (no
//! `..`, nothing absolute), and no step follows a symlink planted there: a VM that
//! swaps `out/` or a file for a link to `~/.zshrc` gets an error, not a write there.

use std::io;
use std::path::{Component, Path};

/// The largest file `read` returns.
pub const READ_LIMIT: u64 = 4 << 20;

fn refused(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why.to_string())
}

/// `rel` as names under a root: no `..`, no root or drive, at least one name.
fn names(rel: &str) -> io::Result<Vec<String>> {
    let mut out = Vec::new();
    for part in Path::new(rel).components() {
        match part {
            Component::Normal(name) => out.push(
                name.to_str()
                    .ok_or_else(|| refused("the path isn't text"))?
                    .to_string(),
            ),
            Component::CurDir => {}
            _ => return Err(refused("the path leaves its folder")),
        }
    }
    if out.is_empty() {
        return Err(refused("the path is empty"));
    }
    Ok(out)
}

/// Write `data` to `rel` under `root`, replacing it whole: written beside it first,
/// then renamed over it, so a reader never sees half. Folders on the way are created.
pub fn write(root: &Path, rel: &str, data: &[u8]) -> io::Result<()> {
    let mut parts = names(rel)?;
    let name = parts.pop().expect("names returns at least one");
    imp::write(root, &parts, &name, data)
}

/// Read `rel` under `root`: a regular file of at most `READ_LIMIT` bytes.
pub fn read(root: &Path, rel: &str) -> io::Result<Vec<u8>> {
    let mut parts = names(rel)?;
    let name = parts.pop().expect("names returns at least one");
    imp::read(root, &parts, &name)
}

/// Remove the file (or link) `rel` under `root`; a missing one is fine.
pub fn remove(root: &Path, rel: &str) -> io::Result<()> {
    let mut parts = names(rel)?;
    let name = parts.pop().expect("names returns at least one");
    match imp::remove(root, &parts, &name) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Create the folder `rel` under `root` and the ones on its way.
pub fn create_dirs(root: &Path, rel: &str) -> io::Result<()> {
    imp::open_dirs(root, &names(rel)?, true).map(|_| ())
}

/// The names in folder `rel` under `root`, when it is a real folder (not a link).
pub fn list(root: &Path, rel: &str) -> io::Result<Vec<String>> {
    let parts = names(rel)?;
    let mut path = root.to_path_buf();
    for part in &parts {
        path.push(part);
        if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(refused("the folder is a link"));
        }
    }
    Ok(std::fs::read_dir(&path)?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect())
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    fn c(name: &str) -> io::Result<CString> {
        CString::new(name).map_err(|_| super::refused("the name holds a NUL"))
    }

    fn check(ret: libc::c_int) -> io::Result<libc::c_int> {
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret)
        }
    }

    /// The folder `parts` under `root`, opened without following a link at any step.
    /// `root` itself is Runo's own path and may be reached through links.
    pub fn open_dirs(root: &Path, parts: &[String], create: bool) -> io::Result<OwnedFd> {
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;
        let root = CString::new(root.as_os_str().as_bytes())
            .map_err(|_| super::refused("the folder holds a NUL"))?;
        // SAFETY: a valid C string and flags; the descriptor is owned from here on.
        let mut dir = unsafe { OwnedFd::from_raw_fd(check(libc::open(root.as_ptr(), flags))?) };
        for part in parts {
            let name = c(part)?;
            let open = |dir: &OwnedFd| {
                // SAFETY: an open directory descriptor and a valid C string.
                check(unsafe {
                    libc::openat(dir.as_raw_fd(), name.as_ptr(), flags | libc::O_NOFOLLOW)
                })
            };
            let fd = match open(&dir) {
                Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                    // SAFETY: as above.
                    let made =
                        check(unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o755) });
                    match made {
                        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(e),
                        _ => open(&dir)?,
                    }
                }
                other => other?,
            };
            // SAFETY: a descriptor openat just returned.
            dir = unsafe { OwnedFd::from_raw_fd(fd) };
        }
        Ok(dir)
    }

    /// Open `name` in `dir` without following a link, and insist on a regular file.
    fn open_file(dir: &OwnedFd, name: &str, flags: libc::c_int) -> io::Result<File> {
        let name = c(name)?;
        // Non-blocking, so a planted FIFO can't hold the open; regular files ignore it.
        let flags = flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
        // SAFETY: an open directory descriptor and a valid C string.
        let fd = check(unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags, 0o644) })?;
        // SAFETY: a descriptor openat just returned.
        let file = unsafe { File::from_raw_fd(fd) };
        if !file.metadata()?.is_file() {
            return Err(super::refused("not a regular file"));
        }
        Ok(file)
    }

    pub fn write(root: &Path, parts: &[String], name: &str, data: &[u8]) -> io::Result<()> {
        let dir = open_dirs(root, parts, true)?;
        let tmp = format!(".{name}.runo-tmp");
        let _ = remove_in(&dir, &tmp);
        let mut file = open_file(&dir, &tmp, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
        file.write_all(data)?;
        drop(file);
        let (from, to) = (c(&tmp)?, c(name)?);
        // A rename replaces a link at `name` itself; it never writes through it.
        // SAFETY: open directory descriptors and valid C strings.
        check(unsafe {
            libc::renameat(dir.as_raw_fd(), from.as_ptr(), dir.as_raw_fd(), to.as_ptr())
        })
        .map(|_| ())
    }

    pub fn read(root: &Path, parts: &[String], name: &str) -> io::Result<Vec<u8>> {
        let dir = open_dirs(root, parts, false)?;
        let file = open_file(&dir, name, libc::O_RDONLY)?;
        let mut data = Vec::new();
        file.take(super::READ_LIMIT + 1).read_to_end(&mut data)?;
        if data.len() as u64 > super::READ_LIMIT {
            return Err(super::refused("the file is too large"));
        }
        Ok(data)
    }

    fn remove_in(dir: &OwnedFd, name: &str) -> io::Result<()> {
        let name = c(name)?;
        // SAFETY: an open directory descriptor and a valid C string.
        check(unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) }).map(|_| ())
    }

    pub fn remove(root: &Path, parts: &[String], name: &str) -> io::Result<()> {
        remove_in(&open_dirs(root, parts, false)?, name)
    }
}

/// Without `openat`, each step is checked for a link before it is used. A link swapped
/// in between the check and the use can still win, which is why Cove's shared
/// folders, the case this guards, are only on macOS and Linux.
#[cfg(not(unix))]
mod imp {
    use std::io::{self, Read, Write};
    use std::path::{Path, PathBuf};

    fn real(path: &Path, want_dir: bool) -> io::Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() || meta.is_dir() != want_dir {
            return Err(super::refused("a link or the wrong kind of file"));
        }
        Ok(())
    }

    pub fn open_dirs(root: &Path, parts: &[String], create: bool) -> io::Result<PathBuf> {
        let mut path = root.to_path_buf();
        for part in parts {
            path.push(part);
            match real(&path, true) {
                Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                    std::fs::create_dir(&path)?
                }
                other => other?,
            }
        }
        Ok(path)
    }

    pub fn write(root: &Path, parts: &[String], name: &str, data: &[u8]) -> io::Result<()> {
        let dir = open_dirs(root, parts, true)?;
        let tmp = dir.join(format!(".{name}.runo-tmp"));
        let _ = std::fs::remove_file(&tmp);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(data)?;
        drop(file);
        std::fs::rename(&tmp, dir.join(name))
    }

    pub fn read(root: &Path, parts: &[String], name: &str) -> io::Result<Vec<u8>> {
        let path = open_dirs(root, parts, false)?.join(name);
        real(&path, false)?;
        let mut data = Vec::new();
        std::fs::File::open(&path)?
            .take(super::READ_LIMIT + 1)
            .read_to_end(&mut data)?;
        if data.len() as u64 > super::READ_LIMIT {
            return Err(super::refused("the file is too large"));
        }
        Ok(data)
    }

    pub fn remove(root: &Path, parts: &[String], name: &str) -> io::Result<()> {
        std::fs::remove_file(open_dirs(root, parts, false)?.join(name))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn writes_stay_in_the_folder_and_never_follow_links() {
        let base = std::env::temp_dir().join(format!("runo-safefs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, outside) = (base.join("root"), base.join("outside"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let victim = outside.join("zshrc");
        std::fs::write(&victim, "mine").unwrap();

        write(&root, ".runo/mcp/out/a", b"reply").unwrap();
        assert_eq!(read(&root, ".runo/mcp/out/a").unwrap(), b"reply");
        assert!(list(&root, ".runo/mcp/out")
            .unwrap()
            .contains(&"a".to_string()));

        // Paths that leave the folder are refused.
        for bad in ["../x", "/etc/passwd", "", ".runo/../../x"] {
            assert!(write(&root, bad, b"x").is_err(), "{bad}");
        }

        // A link in place of the file is replaced, not written through.
        std::os::unix::fs::symlink(&victim, root.join(".runo/mcp/alive")).unwrap();
        write(&root, ".runo/mcp/alive", b"").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "mine");
        assert!(!std::fs::symlink_metadata(root.join(".runo/mcp/alive"))
            .unwrap()
            .file_type()
            .is_symlink());

        // A linked file isn't read, and a linked folder isn't entered.
        std::os::unix::fs::symlink(&victim, root.join(".runo/mcp/out/b")).unwrap();
        assert!(read(&root, ".runo/mcp/out/b").is_err());
        remove(&root, ".runo/mcp/out/b").unwrap();
        assert!(victim.exists());
        std::fs::remove_dir_all(root.join(".runo/mcp/out")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".runo/mcp/out")).unwrap();
        assert!(write(&root, ".runo/mcp/out/zshrc", b"evil").is_err());
        assert!(list(&root, ".runo/mcp/out").is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "mine");
        let _ = std::fs::remove_dir_all(base);
    }
}
