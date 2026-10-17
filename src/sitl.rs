//! The ArduPilot the sim starts for itself, so that a plane with a flight controller flies with
//! no ground station: `sim001` alone, with the T1-S VTOL. A native ArduPlane SITL build (see
//! ardupilot/build_native.sh) is the light way; the Docker image the station uses is the fallback.

use crate::sim::SimCore;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where SITL listens for a ground station (and where the sim listens to its heartbeat).
pub const MAVLINK_PORT: u16 = 5760;
const DOCKER_IMAGE: &str = "sim001-ardupilot-sitl";
/// MAV_AUTOPILOT_ARDUPILOTMEGA.
const ARDUPILOTMEGA: u32 = 3;
const DOCKER_NAME: &str = "sim001-sitl";
/// ardupilot/start.sh's default home: the CMAC flying field in Canberra.
const HOME: &str = "-35.363261,149.165230,584,353";

/// How SITL is to be started.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Native { program: PathBuf, args: Vec<String> },
    Docker { args: Vec<String> },
    /// Neither is there: what to do about it.
    Missing(String),
}

/// Where the sim's files are: the checkout, or the app bundle's resources (`SIM001_HOME`, set by the launcher).
pub fn resource_root() -> PathBuf {
    std::env::var_os("SIM001_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

/// Where what the sim writes goes (flight logs): the checkout, or the user's data folder (`SIM001_DATA`).
pub fn data_dir(root: &Path) -> PathBuf {
    std::env::var_os("SIM001_DATA").map(PathBuf::from).unwrap_or_else(|| root.to_path_buf())
}

/// Where a native build puts ArduPlane, under the checkout `root`; in an app bundle it sits beside the programs.
pub fn native_binary(root: &Path) -> PathBuf {
    if let Some(bin) = std::env::var_os("SIM001_BIN") {
        let bundled = PathBuf::from(bin).join(if cfg!(windows) { "arduplane.exe" } else { "arduplane" });
        if bundled.exists() {
            return bundled;
        }
    }
    root.join(".ardupilot/build/sitl/bin/arduplane")
}

/// The command line of a native ArduPlane for the plane `aircraft`, with its physics at `physics_port`.
pub fn native_args(root: &Path, aircraft: &str) -> Vec<String> {
    let parm = |name: &str| root.join("ardupilot").join(name).display().to_string();
    let mut defaults = vec![root.join(".ardupilot/Tools/autotest/models/plane.parm").display().to_string(), parm("t1s.parm")];
    if root.join("ardupilot").join(format!("{aircraft}.parm")).exists() {
        defaults.push(parm(&format!("{aircraft}.parm")));
    }
    vec![
        "--model".into(),
        "JSON:127.0.0.1".into(),
        "--speedup".into(),
        "1".into(),
        "--home".into(),
        HOME.into(),
        "--defaults".into(),
        defaults.join(","),
        "--serial0".into(),
        "tcp:0".into(),
        "-I0".into(),
    ]
}

/// The `docker run` arguments of the station's image for `aircraft`.
pub fn docker_args(aircraft: &str) -> Vec<String> {
    let a = |s: &str| s.to_string();
    vec![a("run"), a("--rm"), a("--name"), a(DOCKER_NAME), a("-p"), format!("{MAVLINK_PORT}:{MAVLINK_PORT}"), a("-e"), format!("AIRCRAFT={aircraft}"), a("-e"), a("NOWAIT=1"), a(DOCKER_IMAGE)]
}

/// Native first, then Docker, else say how to get one.
pub fn plan(root: &Path, aircraft: &str, native: bool, docker_image: bool) -> Plan {
    if native {
        Plan::Native { program: native_binary(root), args: native_args(root, aircraft) }
    } else if docker_image {
        Plan::Docker { args: docker_args(aircraft) }
    } else {
        Plan::Missing("this plane has a flight controller, and ArduPilot flies it. Build ArduPilot once with `ardupilot/build_native.sh` (or `docker build -t sim001-ardupilot-sitl ardupilot/`), or start the whole station with `cd domains/teleoperation/console && cargo run --release -- --station`".into())
    }
}

/// The name of an ArduPlane flight mode by its MAVLink custom mode number.
pub fn plane_mode_name(n: u32) -> String {
    match n {
        0 => "MANUAL",
        1 => "CIRCLE",
        2 => "STABILIZE",
        3 => "TRAINING",
        4 => "ACRO",
        5 => "FBWA",
        6 => "FBWB",
        7 => "CRUISE",
        8 => "AUTOTUNE",
        10 => "AUTO",
        11 => "RTL",
        12 => "LOITER",
        13 => "TAKEOFF",
        15 => "GUIDED",
        17 => "QSTABILIZE",
        18 => "QHOVER",
        19 => "QLOITER",
        20 => "QLAND",
        21 => "QRTL",
        _ => return format!("MODE {n}"),
    }
    .into()
}

/// A started SITL, stopped when this is dropped.
pub struct Running {
    child: Child,
    docker: bool,
    root: PathBuf,
    plan: Plan,
}

impl Running {
    fn halt(&mut self) {
        if self.docker {
            let _ = Command::new("docker").args(["rm", "-f", DOCKER_NAME]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// A fresh ArduPilot: it is the only way back to a plane on the ground, disarmed, with its
    /// estimators at rest (the tests do this between one test and the next).
    pub fn restart(&mut self) -> Result<(), String> {
        self.halt();
        let (child, docker) = spawn(&self.root, self.plan.clone())?;
        self.child = child;
        self.docker = docker;
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.halt();
    }
}

/// The SITL the sim started for itself, kept here so that the tests can restart it.
static SITL: std::sync::Mutex<Option<Running>> = std::sync::Mutex::new(None);

pub fn keep(running: Running) {
    *SITL.lock().unwrap() = Some(running);
}

pub fn restart() -> Result<(), String> {
    SITL.lock().unwrap().as_mut().map_or(Err("the sim did not start this ArduPilot".into()), Running::restart)
}

/// Stops it (at exit).
pub fn stop() {
    SITL.lock().unwrap().take();
}

/// Start what the plan says. SITL runs in `logs/sitl` under `root`, where it keeps its files.
pub fn start(root: &Path, plan: Plan) -> Result<Running, String> {
    let (child, docker) = spawn(root, plan.clone())?;
    Ok(Running { child, docker, root: root.to_path_buf(), plan })
}

fn spawn(root: &Path, plan: Plan) -> Result<(Child, bool), String> {
    let dir = data_dir(root).join("logs/sitl");
    let _ = std::fs::create_dir_all(&dir);
    let (mut cmd, docker) = match plan {
        Plan::Native { program, args } => {
            let mut c = Command::new(program);
            c.args(args).current_dir(&dir);
            (c, false)
        }
        Plan::Docker { args } => {
            let _ = Command::new("docker").args(["rm", "-f", DOCKER_NAME]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            let mut c = Command::new("docker");
            c.args(args);
            (c, true)
        }
        Plan::Missing(why) => return Err(why),
    };
    let child = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| format!("cannot start ArduPilot: {e}"))?;
    Ok((child, docker))
}

/// Whether the Docker image of the station is built.
pub fn docker_image_exists() -> bool {
    Command::new("docker").args(["image", "inspect", DOCKER_IMAGE]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Reads SITL's heartbeat over MAVLink and tells the core whether the plane is armed and in which
/// mode, as the ground station does when there is one (the throttle stick and the HUD need it).
pub fn watch_heartbeat(core: Arc<Mutex<SimCore>>) {
    std::thread::Builder::new()
        .name("sitl-heartbeat".into())
        .spawn(move || loop {
            let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", MAVLINK_PORT)) else {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            };
            let mut parser = communication_telemetry::Parser::default();
            let mut buf = [0u8; 512];
            while let Ok(n) = std::io::Read::read(&mut stream, &mut buf) {
                if n == 0 {
                    break;
                }
                parser.push(&buf[..n]);
                while let Some((_, msg)) = parser.next_message() {
                    // What ArduPilot says (why it won't arm, for one) goes to the terminal.
                    if let communication_telemetry::MavMessage::STATUSTEXT(t) = &msg {
                        let text: Vec<u8> = t.text.iter().copied().take_while(|b| *b != 0).collect();
                        println!("ArduPilot: {}", String::from_utf8_lossy(&text));
                    }
                    // MAV_SYS_STATUS_PREARM_CHECK: ArduPilot's own pre-arm checks pass.
                    if let communication_telemetry::MavMessage::SYS_STATUS(st) = &msg {
                        core.lock().unwrap().ap.prearm_ok = st.onboard_control_sensors_health.bits() & 0x1000_0000 != 0;
                    }
                    if let communication_telemetry::MavMessage::HEARTBEAT(h) = msg {
                        // The autopilot's own heartbeat, not a ground station's.
                        if h.autopilot as u32 == ARDUPILOTMEGA {
                            let armed = h.base_mode.bits() & 128 != 0;
                            core.lock().unwrap().ap.gcs = Some((Instant::now(), armed, plane_mode_name(h.custom_mode)));
                        }
                    }
                }
            }
        })
        .expect("spawn heartbeat thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn a_native_build_is_preferred_over_docker() {
        let p = plan(&root(), "t1s-vtol", true, true);
        assert!(matches!(p, Plan::Native { .. }), "{p:?}");
        assert!(matches!(plan(&root(), "t1s-vtol", false, true), Plan::Docker { .. }));
    }

    #[test]
    fn with_neither_it_says_how_to_get_one() {
        let Plan::Missing(why) = plan(&root(), "t1s-vtol", false, false) else { panic!("expected Missing") };
        assert!(why.contains("ardupilot/build_native.sh") && why.contains("--station"), "{why}");
    }

    #[test]
    fn native_sitl_gets_its_physics_from_the_sim_on_this_machine() {
        let args = native_args(&root(), "t1s-vtol").join(" ");
        assert!(args.contains("--model JSON:127.0.0.1"), "{args}");
        assert!(args.contains("t1s.parm") && args.contains("t1s-vtol.parm"), "the VTOL's parameters go over the T1-S defaults: {args}");
        assert!(args.contains("-I0") && args.contains("--serial0 tcp:0"), "it does not wait for a ground station: {args}");
    }

    #[test]
    fn docker_gets_the_plane_and_does_not_wait() {
        let args = docker_args("t1s-vtol").join(" ");
        assert!(args.contains("AIRCRAFT=t1s-vtol") && args.contains("NOWAIT=1") && args.contains("5760:5760"), "{args}");
    }

    #[test]
    fn the_modes_of_the_vtol_are_named() {
        assert_eq!(plane_mode_name(18), "QHOVER");
        assert_eq!(plane_mode_name(5), "FBWA");
        assert_eq!(plane_mode_name(21), "QRTL");
        assert_eq!(plane_mode_name(99), "MODE 99");
    }
}
