//! The size of this computer (logical cores and memory), and the concurrency limits that
//! scale with it when a setting is on Auto.

use std::sync::OnceLock;

const GB: u64 = 1 << 30;

/// Logical cores and total memory, rounded to whole gigabytes (0 when it couldn't be read).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hardware {
    pub cores: usize,
    pub ram_gb: u64,
}

/// This computer, read once.
pub fn here() -> Hardware {
    static HERE: OnceLock<Hardware> = OnceLock::new();
    *HERE.get_or_init(|| {
        let hw = Hardware {
            cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
            ram_gb: total_ram().map_or(0, |bytes| (bytes + GB / 2) / GB),
        };
        if hw.ram_gb == 0 {
            crate::applog::info(
                "hardware",
                "couldn't read this computer's memory, so Auto limits stay at their lowest",
            );
        }
        hw
    })
}

/// Post-turn check sets that run at once on Auto. A check set is usually a full build and
/// test run, which can take every core and a few gigabytes (a Rust or C++ link, a webpack
/// build, a JVM test run), while the agents and the person's own tools keep running
/// beside them. So one set per 4 logical cores and per 6 GB, the smaller of the two, from
/// 1 to 6: 8 GB gives 1, 16 GB with 8 cores gives 2, 32 GB with 12 cores 3, 64 GB with
/// 16 cores 4. Past 6 the disk and memory bandwidth run out before the cores do.
pub fn checks_auto(hw: Hardware) -> usize {
    (hw.cores / 4).min((hw.ram_gb / 6) as usize).clamp(1, 6)
}

/// Crew threads that run at once on Auto. An agent mostly waits on the network and its
/// model, but the builds and tests it runs take cores and memory like anyone's, so one
/// thread per 3 logical cores and per 4 GB, the smaller of the two, from 2 to 8: 8 GB
/// gives 2, 16 GB with 10 cores 3, 32 GB with 12 cores 4, 64 GB with 16 cores 5. Past 8
/// a plan's usage window is the limit, not the computer.
pub fn crew_auto(hw: Hardware) -> usize {
    (hw.cores / 3).min((hw.ram_gb / 4) as usize).clamp(2, 8)
}

/// How Settings names this computer and its size: "this Mac: 16 cores, 64 GB".
pub fn describe(hw: Hardware) -> String {
    let name = if cfg!(target_os = "macos") {
        "this Mac"
    } else if cfg!(windows) {
        "this PC"
    } else {
        "this computer"
    };
    let ram = if hw.ram_gb == 0 {
        "memory unknown".to_string()
    } else {
        format!("{} GB", hw.ram_gb)
    };
    format!("{name}: {} cores, {ram}", hw.cores)
}

/// A concurrency setting's value: "auto" takes `auto`, a number is itself, and 0 is no
/// limit. Anything else is Auto.
pub fn resolve(choice: &str, auto: usize) -> usize {
    choice.trim().parse().unwrap_or(auto)
}

#[cfg(target_os = "macos")]
fn total_ram() -> Option<u64> {
    let mut bytes: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    // SAFETY: hw.memsize is a 64-bit integer; the buffer and its length match it.
    let ok = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&mut bytes as *mut u64).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } == 0;
    (ok && bytes > 0).then_some(bytes)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn total_ram() -> Option<u64> {
    meminfo_total(&std::fs::read_to_string("/proc/meminfo").ok()?)
}

/// MemTotal from the text of /proc/meminfo, in bytes.
#[cfg(any(test, all(unix, not(target_os = "macos"))))]
fn meminfo_total(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    (kb > 0).then_some(kb * 1024)
}

#[cfg(windows)]
fn total_ram() -> Option<u64> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }
    // SAFETY: a zeroed MEMORYSTATUSEX with its length set, as the call asks.
    let mut status: MemoryStatusEx = unsafe { std::mem::zeroed() };
    status.length = std::mem::size_of::<MemoryStatusEx>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) } != 0;
    (ok && status.total_phys > 0).then_some(status.total_phys)
}

#[cfg(not(any(unix, windows)))]
fn total_ram() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(cores: usize, ram_gb: u64) -> Hardware {
        Hardware { cores, ram_gb }
    }

    #[test]
    fn checks_scale_with_the_smaller_of_cores_and_memory() {
        assert_eq!(checks_auto(hw(8, 8)), 1);
        assert_eq!(checks_auto(hw(4, 8)), 1);
        assert_eq!(checks_auto(hw(8, 16)), 2);
        assert_eq!(checks_auto(hw(12, 32)), 3);
        assert_eq!(checks_auto(hw(16, 64)), 4);
        assert_eq!(checks_auto(hw(24, 64)), 6);
        // Lots of cores, little memory: memory decides.
        assert_eq!(checks_auto(hw(32, 16)), 2);
        assert_eq!(checks_auto(hw(128, 512)), 6);
        // Unknown memory, or a tiny machine, still runs one.
        assert_eq!(checks_auto(hw(16, 0)), 1);
        assert_eq!(checks_auto(hw(1, 2)), 1);
    }

    #[test]
    fn crew_threads_scale_from_two_to_eight() {
        assert_eq!(crew_auto(hw(8, 8)), 2);
        assert_eq!(crew_auto(hw(8, 16)), 2);
        assert_eq!(crew_auto(hw(10, 16)), 3);
        assert_eq!(crew_auto(hw(12, 32)), 4);
        assert_eq!(crew_auto(hw(16, 64)), 5);
        assert_eq!(crew_auto(hw(32, 128)), 8);
        assert_eq!(crew_auto(hw(64, 16)), 4);
        assert_eq!(crew_auto(hw(2, 4)), 2);
        assert_eq!(crew_auto(hw(16, 0)), 2);
    }

    #[test]
    fn choices_resolve_to_a_number() {
        assert_eq!(resolve("auto", 3), 3);
        assert_eq!(resolve("4", 3), 4);
        assert_eq!(resolve("0", 3), 0);
        assert_eq!(resolve("", 3), 3);
        assert_eq!(resolve("lots", 3), 3);
    }

    #[test]
    fn the_computer_is_described_plainly() {
        let text = describe(hw(16, 64));
        assert!(text.ends_with(": 16 cores, 64 GB"), "{text}");
        assert!(describe(hw(4, 0)).ends_with("memory unknown"));
    }

    #[test]
    fn meminfo_gives_total_memory() {
        let text = "MemTotal:       16303428 kB\nMemFree:         1234 kB\n";
        let bytes = meminfo_total(text).unwrap();
        assert_eq!((bytes + GB / 2) / GB, 16);
        assert_eq!(meminfo_total("MemFree: 12 kB\n"), None);
        assert_eq!(meminfo_total("MemTotal: lots\n"), None);
    }

    #[test]
    fn this_computer_reads() {
        let hw = here();
        assert!(hw.cores >= 1);
        if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
            assert!(hw.ram_gb > 0, "{hw:?}");
        }
    }
}
