//! The one program the launcher runs at a time (the sim, the station, the docs build), with its
//! output kept for the log pane. It runs in its own process group, so stopping it also stops what
//! it started (cargo's child, the sim, the console).

use std::io::{BufRead, BufReader, Read};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

const MAX_LINES: usize = 400;

pub struct Job {
    pub title: String,
    child: Child,
    pub log: Arc<Mutex<Vec<String>>>,
    pub exit: Option<std::process::ExitStatus>,
    pub stopping: bool,
    stop_at: Option<std::time::Instant>,
}

/// Where everything is: in a checkout (`cargo run`), or in an app bundle made by tools/bundle.sh.
pub struct Layout {
    /// Resources: maps, docs, ArduPilot's files, the terrain map.
    pub home: PathBuf,
    /// The programs (sim001, sim001_gcs, arduplane). In a checkout they are built by cargo instead.
    pub bin: Option<PathBuf>,
    /// controls.cfg and the flight logs: the checkout, or the user's data folder.
    pub data: PathBuf,
}

fn user_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/SIM001")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(home).join("SIM001")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("sim001")
    }
}

impl Layout {
    pub fn detect() -> Layout {
        let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_default();
        // A bundle has a `sim001-bundle` marker in its resources: beside the launcher, or in
        // Contents/Resources of a macOS .app.
        for home in [exe.clone(), exe.join("../Resources")] {
            if home.join("sim001-bundle").exists() {
                let data = user_data_dir();
                let _ = std::fs::create_dir_all(&data);
                // The first time, the radio is mapped as in controls.cfg.example (a TX15 in joystick
                // mode); the wizard replaces it.
                let cfg = data.join("controls.cfg");
                if !cfg.exists() {
                    let _ = std::fs::copy(home.join("controls.cfg.example"), &cfg);
                }
                return Layout { home, bin: Some(exe), data };
            }
        }
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).map(Path::to_path_buf).unwrap_or_default();
        // `SIM001_DATA` moves controls.cfg and the logs elsewhere (`--expect` gives each run its own).
        let data = std::env::var_os("SIM001_DATA").map(PathBuf::from).unwrap_or_else(|| root.clone());
        Layout { home: root, bin: None, data }
    }

    pub fn bundled(&self) -> bool {
        self.bin.is_some()
    }

    /// The program `name` of the bundle.
    pub fn program(&self, name: &str) -> Option<String> {
        self.bin
            .as_ref()
            .map(|b| b.join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }).display().to_string())
    }
}

/// In `--expect`, the launcher starts itself in place of each program (`--stand-in`), which waits
/// until it is stopped. The log and the job are real; nothing flies.
static STAND_IN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn use_stand_in(exe: PathBuf) {
    let _ = STAND_IN.set(exe);
}

/// The stand-in's part (`sim001_launcher --stand-in PROGRAM ARGS...`): wait to be stopped, as the
/// sim and the station do. It says nothing: the log's first line already has the command.
pub fn stand_in(_program: &[String]) -> ! {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// What to spawn for `program args`: the program, or the stand-in for it.
fn spawned(program: &str, args: &[String]) -> (String, Vec<String>) {
    match STAND_IN.get() {
        Some(exe) => {
            let mut a = vec!["--stand-in".to_string(), program.to_string()];
            a.extend(args.iter().cloned());
            (exe.display().to_string(), a)
        }
        None => (program.to_string(), args.to_vec()),
    }
}

pub fn layout() -> &'static Layout {
    static L: std::sync::OnceLock<Layout> = std::sync::OnceLock::new();
    L.get_or_init(Layout::detect)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            if chars.next_if_eq(&'[').is_some() {
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if ch != '\r' {
            out.push(ch);
        }
    }
    out
}

fn pump(reader: impl Read + Send + 'static, log: Arc<Mutex<Vec<String>>>) {
    std::thread::spawn(move || {
        // Cargo redraws its progress with \r; treat both as line ends.
        for chunk in BufReader::new(reader).split(b'\n') {
            let Ok(chunk) = chunk else { break };
            let text = String::from_utf8_lossy(&chunk).into_owned();
            for part in text.split('\r') {
                let line = strip_ansi(part);
                if line.trim().is_empty() {
                    continue;
                }
                let mut log = log.lock().unwrap();
                log.push(line);
                if log.len() > MAX_LINES {
                    log.remove(0);
                }
            }
        }
    });
}

impl Job {
    /// Ctrl-C (or, with `force`, a kill) for the whole process group: the program and what it started.
    fn signal_group(&self, force: bool) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), if force { libc::SIGKILL } else { libc::SIGINT });
        }
        #[cfg(windows)]
        {
            // taskkill /T takes the children too; there is no gentler signal for a console-less child.
            let _ = force;
            let _ = Command::new("taskkill").args(["/PID", &self.child.id().to_string(), "/T", "/F"]).status();
        }
    }

    /// Runs `program args` in `dir`.
    pub fn start(title: impl Into<String>, program: &str, args: &[String], dir: &Path) -> Result<Job, String> {
        let title = title.into();
        let (exe, exe_args) = spawned(program, args);
        let mut cmd = Command::new(&exe);
        cmd.args(&exe_args).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        // Where the programs find their files (see sitl::resource_root in the sim).
        let l = layout();
        cmd.env("SIM001_HOME", &l.home).env("SIM001_DATA", &l.data);
        if let Some(bin) = &l.bin {
            cmd.env("SIM001_BIN", bin);
        }
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd.spawn().map_err(|e| format!("{program}: {e}"))?;
        let log = Arc::new(Mutex::new(vec![format!("$ {program} {}", args.join(" "))]));
        pump(child.stdout.take().unwrap(), log.clone());
        pump(child.stderr.take().unwrap(), log.clone());
        Ok(Job { title, child, log, exit: None, stopping: false, stop_at: None })
    }

    /// Notes whether it has finished.
    pub fn poll(&mut self) {
        if self.exit.is_none() {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.exit = Some(status);
                // Whatever it started and left behind goes too.
                self.signal_group(true);
            }
            // The sim and ArduPilot may not leave on Ctrl-C: after a few seconds, end the group.
            if self.stop_at.is_some_and(|t| t.elapsed() > std::time::Duration::from_secs(3)) {
                self.signal_group(true);
                self.stop_at = None;
            }
        }
    }

    pub fn running(&self) -> bool {
        self.exit.is_none()
    }

    /// Asks the whole group to stop (Ctrl-C, as in a terminal).
    pub fn stop(&mut self) {
        if self.running() {
            self.stopping = true;
            self.stop_at.get_or_insert_with(std::time::Instant::now);
            self.signal_group(false);
        }
    }

    /// The command line it was started with, as the first line of its log shows it.
    pub fn command(&self) -> String {
        self.log.lock().unwrap().first().map(|l| l.trim_start_matches("$ ").to_string()).unwrap_or_default()
    }

    /// The process id, for `--expect` to end a stand-in as a program ends on its own.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Ends the whole group now.
    pub fn kill(&mut self) {
        if self.running() {
            self.signal_group(true);
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_drops_colours_and_carriage_returns() {
        assert_eq!(strip_ansi("\x1b[1m\x1b[32m   Compiling\x1b[0m sim001\r"), "   Compiling sim001");
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn a_program_runs_with_its_output_in_the_log_and_its_command_first() {
        let mut j = Job::start("echo", "echo", &["hello".to_string(), "there".to_string()], Path::new(".")).unwrap();
        let t = std::time::Instant::now();
        while j.running() && t.elapsed().as_secs() < 5 {
            j.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(j.exit.is_some_and(|s| s.success()));
        assert_eq!(j.command(), "echo hello there");
        assert!(j.log.lock().unwrap().iter().any(|l| l == "hello there"), "{:?}", j.log.lock().unwrap());
    }

    #[test]
    fn stop_ends_a_program_that_waits() {
        let mut j = Job::start("sleep", "sleep", &["30".to_string()], Path::new(".")).unwrap();
        j.poll();
        assert!(j.running());
        j.stop();
        let t = std::time::Instant::now();
        while j.running() && t.elapsed().as_secs() < 5 {
            j.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!j.running() && j.stopping);
        assert!(!j.exit.unwrap().success());
    }
}
