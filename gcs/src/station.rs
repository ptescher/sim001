//! `--station`: bring the whole simulator station up from the ground station.
//!
//! [`prepare`] gets every prerequisite in place first, in the terminal: builds the sim, exports
//! the terrain map if missing, starts Docker Desktop, builds the ArduPlane SITL image if
//! missing and (re)starts the SITL container. Then [`start_sim`] starts t1s_sim as SITL's
//! physics with the sensor feed on, in its pane. The sim stops when the console closes.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use t1s_station::layout::Rect;

const SITL_IMAGE: &str = "t1s-ardupilot-sitl";
const SITL_CONTAINER: &str = "t1s-sitl";

pub struct Options {
    pub raster: bool,
    pub feed_port: u16,
    /// Skip `cargo build` of the sim (use the existing binary).
    pub no_build: bool,
    /// Start the sim with the sticks flying (ArduPilot watching); J / D-pad down toggles.
    pub direct: bool,
    /// A real place from `maps/<name>` (None: the procedural Alpine Valley).
    pub map: Option<String>,
    /// The plane to fly (`t1s_sim --aircraft`); None is the sim's default.
    pub aircraft: Option<String>,
    /// The sim takes its radio from the console (`--test-radio`), for acceptance tests.
    pub test_radio: bool,
    /// The sim stands in for the TX15 on this port, and the console takes its telemetry from
    /// there (None: the console talks to ArduPilot directly).
    pub radio_port: Option<u16>,
    /// With the radio: the TX15 stand-in and ArduPilot talk through the sim's flight computer,
    /// on a UART in CRSF, and the console hears only what comes down that line.
    pub flight_computer: bool,
}

/// Where the sim's TX15 stand-in listens.
pub const RADIO_PORT: u16 = 5770;
/// Where the sim's flight computer listens for the TX15 (a virtual UART), with `--flight-computer`.
pub const UART_PORT: u16 = 5771;

/// The map pack's metadata (home position etc.), if `name` has been built.
pub fn pack_meta(name: &str) -> Option<t1s_station::mappack::PackMeta> {
    t1s_station::mappack::PackMeta::read(&repo_root().join("maps").join(name).join("build")).ok()
}

/// Repository root (the sim's crate), next to this crate.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).unwrap_or_default()
}

fn sim_binary() -> PathBuf {
    repo_root().join("target/release/t1s_sim")
}

fn step(n: usize, total: usize, what: &str) {
    println!("\x1b[1mstation [{n}/{total}]\x1b[0m {what}");
}

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what} failed ({status})")) }
}

/// Everything the station needs before it opens windows. Prints progress; long steps (the sim
/// build, the one-time SITL image build) stream their own output.
pub fn prepare(opts: &Options, assets: &Path) -> Result<(), String> {
    let root = repo_root();
    let total = 5;

    step(1, total, "building the sim (cargo build --release)");
    if opts.no_build {
        println!("  skipped (--no-build)");
    } else {
        // Under `cargo run`, $CARGO is the cargo running us; the repo's rust-toolchain.toml
        // pins the same toolchain for the sim.
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        run(Command::new(cargo).args(["build", "--release"]).current_dir(&root), "building the sim")?;
    }
    if !sim_binary().exists() {
        return Err(format!("{} missing after the build", sim_binary().display()));
    }

    if let Some(name) = &opts.map {
        step(2, total, &format!("map pack for {name}"));
        if pack_meta(name).is_some() && assets.join("terrain_map.txt").exists() {
            println!("  maps/{name}/build (present)");
        } else {
            // Downloads USGS lidar + NAIP photos + OSM (a few minutes the first time).
            println!("  building maps/{name} (USGS 3DEP lidar, NAIP photos, OpenStreetMap): a few minutes the first time");
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            run(
                Command::new(cargo).args(["run", "--release", "--manifest-path", "tools/mapbuild/Cargo.toml", "--", name]).current_dir(&root),
                "building the map pack",
            )?;
        }
    } else {
        step(2, total, "terrain map for the tactical display");
        if assets.join("terrain_map.png").exists() && assets.join("terrain_map.txt").exists() {
            println!("  {} (present)", assets.display());
        } else {
            run(Command::new(sim_binary()).arg("--export-map").arg(assets).current_dir(&root), "exporting the terrain map")?;
        }
    }

    step(3, total, "Docker");
    ensure_docker()?;

    step(4, total, "ArduPlane SITL image");
    let have_image = image_id().is_some();
    if have_image {
        // Cached layers make this a few seconds; it picks up changes to t1s.parm / start.sh.
        run(Command::new("docker").args(["build", "-q", "-t", SITL_IMAGE, "ardupilot"]).current_dir(&root).stdout(Stdio::null()), "updating the SITL image")?;
        println!("  {SITL_IMAGE} (up to date)");
    } else {
        println!("  building {SITL_IMAGE} from ardupilot/Dockerfile: first time only, about 15 minutes");
        run(Command::new("docker").args(["build", "-t", SITL_IMAGE, "ardupilot"]).current_dir(&root), "building the SITL image")?;
    }

    step(5, total, "ArduPlane SITL containers: yours, and one for each computer pilot");
    // ArduPilot's home must be the sim's pilot box: the map's home, else start.sh's default.
    let home = opts.map.as_deref().and_then(pack_meta).map(|m| {
        (format!("{:.7},{:.7},{:.1},{:.0}", m.home_lat, m.home_lon, m.home_alt, m.launch_heading), m.title)
    });
    // The computer pilots fly planes like yours, each with an autopilot of its own, so that a
    // game is fair: they take as long to get into the air as you do.
    for instance in 0..=recreation_games::PILOTS.len() {
        println!("  {}", start_sitl(
                instance,
                opts.map.as_deref().unwrap_or("alpine"),
                opts.aircraft.as_deref().unwrap_or(simulation_aircraft::default().id),
                home.as_ref().map(|h| h.0.as_str()),
            )?
        );
    }
    if let Some((loc, title)) = home {
        println!("  home {loc} ({title})");
    }
    Ok(())
}

fn image_id() -> Option<String> {
    let out = Command::new("docker").args(["image", "inspect", "-f", "{{.Id}}", SITL_IMAGE]).output().ok()?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !id.is_empty()).then_some(id)
}

fn docker_up() -> bool {
    Command::new("docker").arg("info").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn ensure_docker() -> Result<(), String> {
    if Command::new("docker").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        return Err("the docker command isn't installed: install Docker Desktop (https://www.docker.com/products/docker-desktop/)".into());
    }
    if docker_up() {
        println!("  running");
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        println!("  starting Docker Desktop...");
        let _ = Command::new("open").args(["-ga", "Docker"]).status();
        let deadline = Instant::now() + Duration::from_secs(180);
        while !docker_up() {
            if Instant::now() > deadline {
                return Err("Docker Desktop didn't come up within 3 minutes".into());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        println!("  running");
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err("the Docker daemon isn't running: start it and try again".into())
}

/// The container of SITL instance `instance`: 0 is yours, 1 to 3 the computer pilots'.
fn sitl_container(instance: usize) -> String {
    if instance == 0 { SITL_CONTAINER.to_string() } else { format!("{SITL_CONTAINER}-{instance}") }
}

/// The `docker run` arguments for SITL instance `instance` (before the image). Yours puts its
/// MAVLink on tcp:5760 for the console; a computer pilot's has nobody to talk to, so it publishes
/// no port. Each keeps its flight logs apart.
fn sitl_run_args(instance: usize, map: &str, aircraft: &str, home_loc: Option<&str>, logs: &std::path::Path) -> Vec<String> {
    let name = sitl_container(instance);
    let mut a: Vec<String> = ["run", "-d", "--rm", "--name", &name].iter().map(|s| s.to_string()).collect();
    if instance == 0 {
        a.extend(["-p".into(), "5760:5760".into()]);
    }
    a.extend(["--label".into(), format!("t1s.map={map}"), "--label".into(), format!("t1s.aircraft={aircraft}")]);
    a.extend(["-e".into(), format!("AIRCRAFT={aircraft}"), "-e".into(), format!("INSTANCE={instance}")]);
    if let Some(h) = home_loc {
        a.extend(["-e".into(), format!("HOME_LOC={h}")]);
    }
    a.extend(["-v".into(), format!("{}:/sitl/logs", logs.display())]);
    a
}

/// Starts SITL instance `instance`, or reboots it if it's already running: it was flying a
/// previous sim's aircraft, and ArduPilot's EKF would see the new one jump back to the pilot box.
/// A container from an older image is replaced, so parameter changes take effect.
fn start_sitl(instance: usize, map: &str, aircraft: &str, home_loc: Option<&str>) -> Result<String, String> {
    let name = sitl_container(instance);
    let name = name.as_str();
    let running = Command::new("docker")
        .args(["ps", "--filter", &format!("name=^{name}$"), "--format", "{{.Image}}"])
        .output()
        .map_err(|e| format!("docker ps: {e}"))?;
    if !String::from_utf8_lossy(&running.stdout).trim().is_empty() {
        let inspect = |fmt: &str| {
            Command::new("docker")
                .args(["inspect", "-f", fmt, name])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        };
        let same_image = image_id().is_some_and(|id| id == inspect("{{.Image}}"));
        // Its home position belongs to the map it was started for.
        let same_map = inspect("{{index .Config.Labels \"t1s.map\"}}") == map;
        // Its parameters belong to the plane it was started for.
        let same_aircraft = inspect("{{index .Config.Labels \"t1s.aircraft\"}}") == aircraft;
        if same_image && same_map && same_aircraft {
            run(Command::new("docker").args(["restart", "-t", "1", name]).stdout(Stdio::null()), "restarting SITL")?;
            return Ok(format!("restarted {name}"));
        }
        println!(
            "  {}: replacing the container",
            if !same_image { "the SITL image changed" } else if !same_map { "different map" } else { "different aircraft" }
        );
    }
    // Gone, stopped, or from an older image.
    let _ = Command::new("docker").args(["rm", "-f", name]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    // ArduPilot's flight logs (.BIN) land in logs/sitl/ in the repo, so they outlive the container.
    let logs = if instance == 0 { repo_root().join("logs/sitl") } else { repo_root().join(format!("logs/sitl/opponent-{instance}")) };
    std::fs::create_dir_all(&logs).map_err(|e| format!("creating {}: {e}", logs.display()))?;
    let out = Command::new("docker")
        .args(sitl_run_args(instance, map, aircraft, home_loc, &logs))
        .arg(SITL_IMAGE)
        .output()
        .map_err(|e| format!("docker run: {e}"))?;
    if out.status.success() {
        Ok(if instance == 0 {
            format!("started {name} (MAVLink on tcp:5760, flight logs in {})", logs.display())
        } else {
            format!("started {name} (a computer pilot's autopilot, flight logs in {})", logs.display())
        })
    } else {
        Err(format!("docker run failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Starts the sim (release build) in its pane. Its output goes to the terminal.
pub fn start_sim(opts: &Options, rect: Option<Rect>, home: (f64, f64, f32)) -> Result<Child, String> {
    let root = repo_root();
    let bin = sim_binary();
    let mut cmd = Command::new(&bin);
    cmd.current_dir(&root).args(["--ardupilot", "--feed", &opts.feed_port.to_string(), "--exit-with-parent"]);
    if opts.raster {
        cmd.arg("--raster");
    }
    if opts.direct {
        cmd.arg("--direct");
    }
    if let Some(m) = &opts.map {
        cmd.args(["--map", m]);
    }
    if let Some(a) = &opts.aircraft {
        cmd.args(["--aircraft", a]);
    }
    if opts.test_radio {
        cmd.arg("--test-radio");
    }
    // The console's telemetry comes through the sim's TX15 stand-in, which needs to know where on the earth the pilot box is.
    if let Some(port) = opts.radio_port {
        cmd.args(["--radio", &port.to_string(), "--home", &format!("{},{},{}", home.0, home.1, home.2)]);
        if opts.flight_computer {
            cmd.args(["--flight-computer", &format!("tcp:{UART_PORT}"), "--radio-uart", &format!("tcp:127.0.0.1:{UART_PORT}")]);
        }
    }
    if let Some(r) = rect {
        cmd.args(["--window-rect", &r.to_arg(), "--frameless"]);
    }
    cmd.spawn().map_err(|e| format!("starting {}: {e}", bin.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Your autopilot talks to the console on tcp:5760. Each computer pilot's is an instance of
    /// its own (so its physics port is 9002 + 10·N), publishes no port, and logs on its own.
    #[test]
    fn each_autopilot_is_a_container_of_its_own() {
        let logs = std::path::Path::new("/logs");
        let yours = sitl_run_args(0, "alpine", "t1s-vtol", None, logs);
        assert!(yours.windows(2).any(|w| w == ["--name", "t1s-sitl"]));
        assert!(yours.windows(2).any(|w| w == ["-p", "5760:5760"]));
        assert!(yours.contains(&"INSTANCE=0".to_string()));
        let kestrel = sitl_run_args(1, "alpine", "t1s-vtol", Some("1,2,3,4"), logs);
        assert!(kestrel.windows(2).any(|w| w == ["--name", "t1s-sitl-1"]));
        assert!(!kestrel.contains(&"-p".to_string()), "nobody connects to it");
        assert!(kestrel.contains(&"INSTANCE=1".to_string()) && kestrel.contains(&"HOME_LOC=1,2,3,4".to_string()));
        assert!(kestrel.contains(&"AIRCRAFT=t1s-vtol".to_string()), "a plane like yours");
    }
}
