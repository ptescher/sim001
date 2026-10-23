//! `--serve`: the app starts a game server of its own (`towers_server`, built from the workspace)
//! and keeps it as long as it runs. The server stops its drones and exits when the app closes
//! its input (`--stop-with-stdin`): when the app quits, or starts a new game. With the cameras on,
//! it starts the 3D view of the game (`towers_view`) too, which sends the camera of each drone.

use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

pub struct Server {
    child: Child,
    /// Closing it stops the server.
    stdin: Option<ChildStdin>,
    /// The 3D view with the drones' cameras, and its input: closing that stops it.
    view: Option<(Child, ChildStdin)>,
}

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().expect("the repo root")
}

/// Builds the game server, and the 3D view with the cameras of the drones if `cameras` (a no-op
/// when they are up to date).
pub fn build(cameras: bool) -> Result<(), String> {
    let mut args = vec!["build", "--release", "-p", "recreation_towers_server"];
    if cameras {
        args.extend(["-p", "recreation_towers_view"]);
    }
    let ok = Command::new("cargo")
        .args(args)
        .current_dir(root())
        .status()
        .map_err(|e| format!("can't run cargo: {e}"))?
        .success();
    if ok { Ok(()) } else { Err("the game server did not build".into()) }
}

/// The ArduPlane with the AGENT mode: `SIM001_BIN`, else the checkout's AGENT build.
fn agent_bin() -> PathBuf {
    std::env::var_os("SIM001_BIN").map(PathBuf::from).unwrap_or_else(|| root().join(".ardupilot-agent/build/sitl/bin"))
}

impl Server {
    /// Starts a game with `args` (`--blue person --red computer --speedup 1` ...), and the 3D view
    /// of it if `cameras`. What they print goes to `logs/towers/server.log` and `view.log`.
    pub fn start(args: &[String], cameras: bool) -> Result<Server, String> {
        let program = root().join("target/release/towers_server");
        let logs = root().join("logs/towers");
        let _ = std::fs::create_dir_all(&logs);
        let log = std::fs::File::create(logs.join("server.log")).map_err(|e| e.to_string())?;
        let err = log.try_clone().map_err(|e| e.to_string())?;
        let mut child = Command::new(&program)
            .args(args)
            .arg("--stop-with-stdin")
            .env("SIM001_BIN", agent_bin())
            .stdin(Stdio::piped())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(err))
            .spawn()
            .map_err(|e| format!("can't start {}: {e}", program.display()))?;
        let stdin = child.stdin.take();
        let view = if cameras { Some(start_view(&logs)?) } else { None };
        Ok(Server { child, stdin, view })
    }

    /// Stops the game: the server stops its drones and exits.
    pub fn stop(&mut self) {
        self.stdin = None;
        if let Some((mut view, stdin)) = self.view.take() {
            drop(stdin);
            for _ in 0..30 {
                if let Ok(Some(_)) = view.try_wait() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            let _ = view.kill();
            let _ = view.wait();
        }
        for _ in 0..100 {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the 3D view of the game, which takes a spectator's seat on the server and serves the
/// drones' cameras (it stops when its input closes).
fn start_view(logs: &Path) -> Result<(Child, ChildStdin), String> {
    let program = root().join("target/release/towers_view");
    let log = std::fs::File::create(logs.join("view.log")).map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let mut child = Command::new(&program)
        .arg("--stop-with-stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .spawn()
        .map_err(|e| format!("can't start {}: {e}", program.display()))?;
    let stdin = child.stdin.take().ok_or("no input to the 3D view")?;
    Ok((child, stdin))
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}
