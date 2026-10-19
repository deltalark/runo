//! The background job that starts `runo --run-due-playbooks` while Runo is closed: a
//! launchd agent on macOS, a systemd user timer on Linux and a Task Scheduler task on
//! Windows. Each sits behind [`Scheduler`]; [`sync`] keeps the job in line with the
//! setting.

use super::{runs_dir, INTERVAL_SECS};
use std::path::PathBuf;
use std::sync::Mutex;

/// What each OS needs to keep the runner job in line with the setting.
trait Scheduler {
    /// The job as it is installed now, in the same form as `wanted`, or None when it
    /// isn't installed.
    fn status(&self) -> Option<String>;
    /// The job that runs `exe --run-due-playbooks` every few minutes.
    fn wanted(&self, exe: &str) -> String;
    /// Install `wanted` in place of whatever is there. Err says why in a sentence.
    fn install(&self, wanted: &str) -> Result<(), String>;
    /// Take the job away.
    fn remove(&self);
}

/// Why the job couldn't be set up, for Settings; None when it is fine.
static NOTE: Mutex<Option<String>> = Mutex::new(None);

/// Why the background job couldn't be set up, when it couldn't.
pub fn note() -> Option<String> {
    NOTE.lock().ok().and_then(|n| n.clone())
}

fn set_note(note: Option<String>) {
    if let Some(n) = &note {
        crate::applog::warn("playbooks", format!("background job not set up: {n}"));
    }
    if let Ok(mut slot) = NOTE.lock() {
        *slot = note;
    }
}

/// Keep the background job in line with the setting: installed and pointing at this
/// copy of Runo when on, gone when off. Runs in the background.
pub fn sync(enabled: bool) {
    let Some(job) = scheduler() else {
        return;
    };
    std::thread::spawn(move || {
        let current = job.status();
        if !enabled {
            if current.is_some() {
                job.remove();
            }
            set_note(None);
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        // An AppImage runs from a mount that is gone once it quits; use the file itself.
        #[cfg(target_os = "linux")]
        let exe = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .unwrap_or(exe);
        let wanted = job.wanted(&exe.to_string_lossy());
        if current.as_deref() == Some(wanted.as_str()) {
            set_note(None);
            return;
        }
        set_note(job.install(&wanted).err());
    });
}

#[cfg(target_os = "macos")]
fn scheduler() -> Option<Box<dyn Scheduler + Send>> {
    Some(Box::new(Launchd::new()?))
}

#[cfg(target_os = "linux")]
fn scheduler() -> Option<Box<dyn Scheduler + Send>> {
    Some(Box::new(Systemd::new()?))
}

#[cfg(windows)]
fn scheduler() -> Option<Box<dyn Scheduler + Send>> {
    Some(Box::new(TaskScheduler))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn scheduler() -> Option<Box<dyn Scheduler + Send>> {
    None
}

/// Run a helper quietly; Ok when it exits 0, else its error output.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn quiet(program: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashing up from the app.
        cmd.creation_flags(0x0800_0000);
    }
    match cmd.output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        Err(e) => Err(format!("{program}: {e}")),
    }
}

// ---------- macOS: launchd ----------

/// The launchd job's label.
#[cfg(target_os = "macos")]
const LABEL: &str = "dev.runo.playbooks";

#[cfg(target_os = "macos")]
struct Launchd {
    plist: PathBuf,
    domain: String,
    job: String,
}

#[cfg(target_os = "macos")]
impl Launchd {
    fn new() -> Option<Self> {
        let home = std::env::var_os("HOME")?;
        let plist = PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let job = format!("{domain}/{LABEL}");
        Some(Self { plist, domain, job })
    }

    fn launchctl(args: &[&str]) {
        let _ = std::process::Command::new("launchctl")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(target_os = "macos")]
impl Scheduler for Launchd {
    fn status(&self) -> Option<String> {
        std::fs::read_to_string(&self.plist).ok()
    }

    fn wanted(&self, exe: &str) -> String {
        launch_plist(exe, &runs_dir().join("runner.log").to_string_lossy())
    }

    fn install(&self, wanted: &str) -> Result<(), String> {
        if let Some(parent) = self.plist.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::create_dir_all(runs_dir());
        if std::fs::write(&self.plist, wanted).is_ok() {
            let path = self.plist.to_string_lossy().into_owned();
            Self::launchctl(&["bootout", self.job.as_str()]);
            Self::launchctl(&["bootstrap", self.domain.as_str(), path.as_str()]);
        }
        Ok(())
    }

    fn remove(&self) {
        Self::launchctl(&["bootout", self.job.as_str()]);
        let _ = std::fs::remove_file(&self.plist);
    }
}

/// The launchd job: the runner every few minutes, in the background.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launch_plist(exe: &str, log: &str) -> String {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.runo.playbooks</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>--run-due-playbooks</string>
  </array>
  <key>StartInterval</key><integer>{INTERVAL_SECS}</integer>
  <key>RunAtLoad</key><false/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>{}</string>
  <key>StandardErrorPath</key><string>{}</string>
</dict>
</plist>
"#,
        esc(exe),
        esc(log),
        esc(log)
    )
}

// ---------- Linux: systemd user timer ----------

/// The systemd units' name, without the .service or .timer.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const UNIT: &str = "runo-playbooks";

/// Between the service and the timer in `Systemd::status` and `wanted`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const UNIT_SPLIT: &str = "\n# --- runo-playbooks.timer ---\n";

#[cfg(target_os = "linux")]
struct Systemd {
    dir: PathBuf,
}

#[cfg(target_os = "linux")]
impl Systemd {
    fn new() -> Option<Self> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|c| !c.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(Self {
            dir: config.join("systemd/user"),
        })
    }

    fn service(&self) -> PathBuf {
        self.dir.join(format!("{UNIT}.service"))
    }

    fn timer(&self) -> PathBuf {
        self.dir.join(format!("{UNIT}.timer"))
    }

    fn systemctl(args: &[&str]) -> Result<(), String> {
        let mut all = vec!["--user"];
        all.extend_from_slice(args);
        quiet("systemctl", &all)
    }
}

#[cfg(target_os = "linux")]
impl Scheduler for Systemd {
    fn status(&self) -> Option<String> {
        let service = std::fs::read_to_string(self.service()).ok()?;
        let timer = std::fs::read_to_string(self.timer()).ok()?;
        Some(format!("{service}{UNIT_SPLIT}{timer}"))
    }

    fn wanted(&self, exe: &str) -> String {
        let log = runs_dir().join("runner.log");
        format!(
            "{}{UNIT_SPLIT}{}",
            systemd_service(exe, &log.to_string_lossy()),
            systemd_timer()
        )
    }

    fn install(&self, wanted: &str) -> Result<(), String> {
        let (service, timer) = wanted.split_once(UNIT_SPLIT).unwrap_or((wanted, ""));
        // Without a user session bus nothing can start the timer; say so and leave
        // nothing behind, so the next launch tries again.
        if let Err(e) = Self::systemctl(&["show-environment"]) {
            return Err(format!(
                "systemd's user session isn't available ({}), so scheduled playbooks only run while Runo is open.",
                if e.is_empty() { "no user bus" } else { &e }
            ));
        }
        let _ = std::fs::create_dir_all(runs_dir());
        std::fs::create_dir_all(&self.dir)
            .and_then(|_| std::fs::write(self.service(), service))
            .and_then(|_| std::fs::write(self.timer(), timer))
            .map_err(|e| format!("couldn't write the systemd timer: {e}"))?;
        let timer = format!("{UNIT}.timer");
        let started = Self::systemctl(&["daemon-reload"])
            .and_then(|_| Self::systemctl(&["enable", "--now", timer.as_str()]))
            // A timer already running keeps its old schedule until restarted.
            .and_then(|_| Self::systemctl(&["restart", timer.as_str()]));
        if let Err(e) = started {
            self.remove();
            return Err(format!("systemctl couldn't start the timer: {e}"));
        }
        Ok(())
    }

    fn remove(&self) {
        let timer = format!("{UNIT}.timer");
        let _ = Self::systemctl(&["disable", "--now", timer.as_str()]);
        let _ = std::fs::remove_file(self.timer());
        let _ = std::fs::remove_file(self.service());
        let _ = Self::systemctl(&["daemon-reload"]);
    }
}

/// Quote one word of a systemd command line, with `%` and `$` kept literal.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_quote(s: &str) -> String {
    let inner = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{inner}\"")
}

/// The systemd service: one pass of the runner, its output added to the log.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_service(exe: &str, log: &str) -> String {
    let log = log.replace('%', "%%");
    format!(
        "[Unit]
Description=Runo scheduled playbooks

[Service]
Type=oneshot
ExecStart={} --run-due-playbooks
StandardOutput=append:{log}
StandardError=append:{log}
",
        systemd_quote(exe)
    )
}

/// The systemd timer: the service every few minutes, the first a few minutes after it
/// starts (like launchd's RunAtLoad false).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_timer() -> String {
    format!(
        "[Unit]
Description=Run Runo's scheduled playbooks every few minutes

[Timer]
OnActiveSec={INTERVAL_SECS}s
OnUnitActiveSec={INTERVAL_SECS}s
Unit={UNIT}.service

[Install]
WantedBy=timers.target
"
    )
}

// ---------- Windows: Task Scheduler ----------

/// The scheduled task's name.
#[cfg_attr(not(windows), allow(dead_code))]
const TASK: &str = "Runo playbooks";

#[cfg(windows)]
struct TaskScheduler;

/// What was last installed, since a task's definition can't be read back as given.
#[cfg(windows)]
fn task_marker() -> PathBuf {
    runs_dir().join("scheduled-task.txt")
}

#[cfg(windows)]
impl Scheduler for TaskScheduler {
    fn status(&self) -> Option<String> {
        quiet("schtasks", &["/Query", "/TN", TASK]).ok()?;
        Some(std::fs::read_to_string(task_marker()).unwrap_or_default())
    }

    fn wanted(&self, exe: &str) -> String {
        schtasks_create(exe).join("\n")
    }

    fn install(&self, wanted: &str) -> Result<(), String> {
        let args: Vec<&str> = wanted.lines().collect();
        quiet("schtasks", &args)
            .map_err(|e| format!("Task Scheduler couldn't create the task: {e}"))?;
        let _ = std::fs::create_dir_all(runs_dir());
        let _ = std::fs::write(task_marker(), wanted);
        Ok(())
    }

    fn remove(&self) {
        let _ = quiet("schtasks", &schtasks_delete());
        let _ = std::fs::remove_file(task_marker());
    }
}

/// `schtasks` arguments for the task: the runner every few minutes.
#[cfg_attr(not(windows), allow(dead_code))]
fn schtasks_create(exe: &str) -> Vec<String> {
    vec![
        "/Create".into(),
        "/SC".into(),
        "MINUTE".into(),
        "/MO".into(),
        (INTERVAL_SECS / 60).max(1).to_string(),
        "/TN".into(),
        TASK.into(),
        "/TR".into(),
        format!("\"{exe}\" --run-due-playbooks"),
        "/F".into(),
    ]
}

/// `schtasks` arguments that delete the task.
#[cfg_attr(not(windows), allow(dead_code))]
fn schtasks_delete() -> [&'static str; 4] {
    ["/Delete", "/TN", TASK, "/F"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_agent_runs_the_runner() {
        let plist = launch_plist(
            "/Applications/R&D/Runo.app/Contents/MacOS/Runo",
            "/tmp/l.log",
        );
        assert!(
            plist.contains("<string>/Applications/R&amp;D/Runo.app/Contents/MacOS/Runo</string>")
        );
        assert!(plist.contains("<string>--run-due-playbooks</string>"));
        assert!(plist.contains("<integer>300</integer>"));
    }

    #[test]
    fn the_systemd_service_runs_the_runner() {
        let unit = systemd_service("/opt/My Apps/runo", "/home/a/.local/share/runo/runner.log");
        assert_eq!(
            unit,
            "[Unit]
Description=Runo scheduled playbooks

[Service]
Type=oneshot
ExecStart=\"/opt/My Apps/runo\" --run-due-playbooks
StandardOutput=append:/home/a/.local/share/runo/runner.log
StandardError=append:/home/a/.local/share/runo/runner.log
"
        );
        // Quotes, backslashes, specifiers and variables in the path stay literal.
        let odd = systemd_service("/x/100%\"$HOME\\runo", "/l/50%.log");
        assert!(odd.contains("ExecStart=\"/x/100%%\\\"$$HOME\\\\runo\" --run-due-playbooks\n"));
        assert!(odd.contains("StandardOutput=append:/l/50%%.log\n"));
    }

    #[test]
    fn the_systemd_timer_matches_launchd() {
        let timer = systemd_timer();
        assert_eq!(
            timer,
            "[Unit]
Description=Run Runo's scheduled playbooks every few minutes

[Timer]
OnActiveSec=300s
OnUnitActiveSec=300s
Unit=runo-playbooks.service

[Install]
WantedBy=timers.target
"
        );
        // The service and timer are told apart again on install.
        let both = format!("{}{UNIT_SPLIT}{timer}", systemd_service("/r", "/l"));
        let (service, back) = both.split_once(UNIT_SPLIT).unwrap();
        assert!(service.starts_with("[Unit]") && service.ends_with("/l\n"));
        assert_eq!(back, timer);
    }

    #[test]
    fn the_scheduled_task_runs_the_runner() {
        assert_eq!(
            schtasks_create(r"C:\Program Files\Runo\runo.exe"),
            [
                "/Create",
                "/SC",
                "MINUTE",
                "/MO",
                "5",
                "/TN",
                "Runo playbooks",
                "/TR",
                r#""C:\Program Files\Runo\runo.exe" --run-due-playbooks"#,
                "/F",
            ]
        );
        assert_eq!(
            schtasks_delete(),
            ["/Delete", "/TN", "Runo playbooks", "/F"]
        );
    }
}
