//! The processes on this machine that can be looked at (the person's own): each one's
//! parent and working folder, read straight from the kernel rather than by running
//! `lsof` or `ps`, which scan every process and take a good part of a second.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    /// Its working folder; `None` when it can't be read (another user's process).
    pub cwd: Option<PathBuf>,
}

#[cfg(target_os = "macos")]
pub fn all() -> Vec<Proc> {
    use std::ffi::CStr;
    use std::mem::size_of;
    // SAFETY: the buffers are sized and typed as libproc expects; each call writes at
    // most the byte count it is given and says how much it wrote.
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return Vec::new();
        }
        // Room for processes started since the count.
        let mut pids = vec![0 as libc::pid_t; n as usize + 64];
        let bytes = (pids.len() * size_of::<libc::pid_t>()) as libc::c_int;
        let got = libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes);
        if got <= 0 {
            return Vec::new();
        }
        pids.truncate(got as usize);
        let mut out = Vec::with_capacity(pids.len());
        for pid in pids.into_iter().filter(|p| *p > 0) {
            let mut bsd: libc::proc_bsdinfo = std::mem::zeroed();
            let size = size_of::<libc::proc_bsdinfo>() as libc::c_int;
            if libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                (&mut bsd as *mut libc::proc_bsdinfo).cast(),
                size,
            ) != size
            {
                continue;
            }
            let mut vnode: libc::proc_vnodepathinfo = std::mem::zeroed();
            let size = size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
            let cwd = (libc::proc_pidinfo(
                pid,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut vnode as *mut libc::proc_vnodepathinfo).cast(),
                size,
            ) == size)
                .then(|| {
                    let raw = vnode.pvi_cdir.vip_path.as_ptr().cast::<libc::c_char>();
                    CStr::from_ptr(raw).to_string_lossy().into_owned()
                })
                .filter(|p| p.starts_with('/'))
                .map(PathBuf::from);
            out.push(Proc {
                pid: pid as u32,
                ppid: bsd.pbi_ppid,
                cwd,
            });
        }
        out
    }
}

#[cfg(target_os = "linux")]
pub fn all() -> Vec<Proc> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
            // `pid (comm) state ppid ...`; comm may hold spaces and parentheses.
            let ppid = stat
                .rsplit_once(')')?
                .1
                .split_whitespace()
                .nth(1)?
                .parse()
                .ok()?;
            let cwd = std::fs::read_link(e.path().join("cwd")).ok();
            Some(Proc { pid, ppid, cwd })
        })
        .collect()
}

/// Elsewhere (Windows) there is no cheap way to tell.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn all() -> Vec<Proc> {
    Vec::new()
}

/// `root` and every process below it.
pub fn descendants(procs: &[Proc], root: u32) -> Vec<u32> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        out.extend(
            procs
                .iter()
                .filter(|p| p.ppid == parent && p.pid != parent && !out.contains(&p.pid))
                .map(|p| p.pid)
                .collect::<Vec<_>>(),
        );
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, ppid: u32) -> Proc {
        Proc {
            pid,
            ppid,
            cwd: None,
        }
    }

    #[test]
    fn descendants_walk_the_whole_tree() {
        let procs = [p(1, 0), p(10, 1), p(11, 10), p(12, 11), p(20, 1), p(13, 10)];
        let mut found = descendants(&procs, 10);
        found.sort_unstable();
        assert_eq!(found, vec![10, 11, 12, 13]);
        assert_eq!(descendants(&procs, 99), vec![99]);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn finds_this_process_and_its_folder() {
        let procs = all();
        let me = procs
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("this process is listed");
        let here = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(
            me.cwd.as_ref().and_then(|c| c.canonicalize().ok()),
            Some(here)
        );
    }
}
