//! Starting ArduPlane SITL: a native build (`ardupilot/build_native.sh`) or the Docker image.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub const DOCKER_IMAGE: &str = "sim001-ardupilot-sitl";
pub const DOCKER_NAME: &str = "sim001-sitl";
/// Where SITL instance 0 listens for a ground station.
pub const MAVLINK_PORT: u16 = 5760;
/// ardupilot/start.sh's default home: the CMAC flying field in Canberra.
pub const HOME: &str = "-35.363261,149.165230,584,353";

/// How SITL is to be started.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Native { program: PathBuf, args: Vec<String> },
    Docker { args: Vec<String> },
    /// Neither is there: what to do about it.
    Missing(String),
}

/// Where a native build puts ArduPlane under the checkout `root`; in an app bundle it sits
/// beside the programs (`SIM001_BIN`).
pub fn native_binary(root: &Path) -> PathBuf {
    if let Some(bin) = std::env::var_os("SIM001_BIN") {
        let bundled = PathBuf::from(bin).join(if cfg!(windows) { "arduplane.exe" } else { "arduplane" });
        if bundled.exists() {
            return bundled;
        }
    }
    root.join(".ardupilot/build/sitl/bin/arduplane")
}

/// The command line of a native ArduPlane for the plane `aircraft`: instance 0, at the default
/// home, in real time.
pub fn native_args(root: &Path, aircraft: &str) -> Vec<String> {
    native_instance_args(root, aircraft, 0, HOME, 1.0)
}

/// The same for instance `n`, at `home` ("lat,lon,alt,heading"), `speedup` times real time. Its
/// physics comes from UDP 9002 + 10·n, its MAVLink goes to TCP 5760 + 10·n, and it never waits
/// for a ground station.
pub fn native_instance_args(root: &Path, aircraft: &str, n: u16, home: &str, speedup: f32) -> Vec<String> {
    let parm = |name: &str| root.join("ardupilot").join(name).display().to_string();
    let mut defaults = vec![root.join(".ardupilot/Tools/autotest/models/plane.parm").display().to_string(), parm("t1s.parm")];
    if root.join("ardupilot").join(format!("{aircraft}.parm")).exists() {
        defaults.push(parm(&format!("{aircraft}.parm")));
    }
    vec![
        "--model".into(),
        "JSON:127.0.0.1".into(),
        "--speedup".into(),
        format!("{speedup}"),
        "--home".into(),
        home.into(),
        "--defaults".into(),
        defaults.join(","),
        "--serial0".into(),
        "tcp:0".into(),
        format!("-I{n}"),
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

/// Whether the Docker image of the station is built.
pub fn docker_image_exists() -> bool {
    Command::new("docker").args(["image", "inspect", DOCKER_IMAGE]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Starts what the plan says, with `dir` as its working directory (SITL keeps its files there).
/// Returns the child and whether it is a Docker one.
pub fn spawn(dir: &Path, plan: Plan) -> Result<(Child, bool), String> {
    let _ = std::fs::create_dir_all(dir);
    let (mut cmd, docker) = match plan {
        Plan::Native { program, args } => {
            let mut c = Command::new(program);
            c.args(args).current_dir(dir);
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

/// Native ArduPlane instances (SITL `-I`), each in its own working directory under `dir`, stopped when
/// this is dropped: the planes of a headless game.
pub struct Fleet {
    children: Vec<Child>,
}

impl Fleet {
    pub fn start(root: &Path, dir: &Path, aircraft: &str, instances: std::ops::Range<u16>, home: &str, speedup: f32) -> Result<Fleet, String> {
        let program = native_binary(root);
        if !program.exists() {
            return Err(format!("no native ArduPlane at {}: run ardupilot/build_native.sh", program.display()));
        }
        let mut fleet = Fleet { children: vec![] };
        for i in instances {
            let plan = Plan::Native { program: program.clone(), args: native_instance_args(root, aircraft, i, home, speedup) };
            fleet.children.push(spawn(&dir.join(format!("instance{i}")), plan)?.0);
        }
        Ok(fleet)
    }

    pub fn stop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.children.clear();
    }
}

impl Drop for Fleet {
    fn drop(&mut self) {
        self.stop();
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
        AGENT_MODE => "AGENT",
        _ => return format!("MODE {n}"),
    }
    .into()
}

/// The custom mode number of the AGENT mode in the fork (`ardupilot/patches`).
pub const AGENT_MODE: u32 = 50;

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
    }

    #[test]
    fn a_native_build_is_preferred_over_docker() {
        assert!(matches!(plan(&root(), "t1s-vtol", true, true), Plan::Native { .. }));
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
    fn instances_take_their_number_and_speed() {
        let args = native_instance_args(&root(), "t1s-vtol", 2, "1,2,3,4", 5.0).join(" ");
        assert!(args.contains("-I2") && args.contains("--speedup 5") && args.contains("--home 1,2,3,4"), "{args}");
    }

    #[test]
    fn docker_gets_the_plane_and_does_not_wait() {
        let args = docker_args("t1s-vtol").join(" ");
        assert!(args.contains("AIRCRAFT=t1s-vtol") && args.contains("NOWAIT=1") && args.contains("5760:5760"), "{args}");
    }

    #[test]
    fn the_modes_are_named() {
        assert_eq!(plane_mode_name(18), "QHOVER");
        assert_eq!(plane_mode_name(5), "FBWA");
        assert_eq!(plane_mode_name(21), "QRTL");
        assert_eq!(plane_mode_name(AGENT_MODE), "AGENT");
        assert_eq!(plane_mode_name(99), "MODE 99");
    }
}
