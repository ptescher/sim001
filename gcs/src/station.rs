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
}

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

    step(5, total, "ArduPlane SITL container");
    // ArduPilot's home must be the sim's pilot box: the map's home, else start.sh's default.
    let home = opts.map.as_deref().and_then(pack_meta).map(|m| {
        (format!("{:.7},{:.7},{:.1},{:.0}", m.home_lat, m.home_lon, m.home_alt, m.launch_heading), m.title)
    });
    println!("  {}", start_sitl(opts.map.as_deref().unwrap_or("alpine"), home.as_ref().map(|h| h.0.as_str()))?);
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

/// Starts the SITL container, or reboots it if it's already running: it was flying a previous
/// sim's aircraft, and ArduPilot's EKF would see the new one jump back to the pilot box. A
/// container from an older image is replaced, so parameter changes take effect.
fn start_sitl(map: &str, home_loc: Option<&str>) -> Result<String, String> {
    let running = Command::new("docker")
        .args(["ps", "--filter", &format!("name=^{SITL_CONTAINER}$"), "--format", "{{.Image}}"])
        .output()
        .map_err(|e| format!("docker ps: {e}"))?;
    if !String::from_utf8_lossy(&running.stdout).trim().is_empty() {
        let inspect = |fmt: &str| {
            Command::new("docker")
                .args(["inspect", "-f", fmt, SITL_CONTAINER])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default()
        };
        let same_image = image_id().is_some_and(|id| id == inspect("{{.Image}}"));
        // Its home position belongs to the map it was started for.
        let same_map = inspect("{{index .Config.Labels \"t1s.map\"}}") == map;
        if same_image && same_map {
            run(Command::new("docker").args(["restart", "-t", "1", SITL_CONTAINER]).stdout(Stdio::null()), "restarting SITL")?;
            return Ok(format!("restarted {SITL_CONTAINER}"));
        }
        println!("  {}: replacing the container", if same_image { "different map" } else { "the SITL image changed" });
    }
    // Gone, stopped, or from an older image.
    let _ = Command::new("docker").args(["rm", "-f", SITL_CONTAINER]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    // ArduPilot's flight logs (.BIN) land in logs/sitl/ in the repo, so they outlive the container.
    let logs = repo_root().join("logs/sitl");
    std::fs::create_dir_all(&logs).map_err(|e| format!("creating {}: {e}", logs.display()))?;
    let out = Command::new("docker")
        .args(["run", "-d", "--rm", "--name", SITL_CONTAINER, "-p", "5760:5760", "--label", &format!("t1s.map={map}")])
        .args(home_loc.map(|h| vec!["-e".to_string(), format!("HOME_LOC={h}")]).unwrap_or_default())
        .arg("-v")
        .arg(format!("{}:/sitl/logs", logs.display()))
        .arg(SITL_IMAGE)
        .output()
        .map_err(|e| format!("docker run: {e}"))?;
    if out.status.success() {
        Ok(format!("started {SITL_CONTAINER} (MAVLink on tcp:5760, flight logs in {})", logs.display()))
    } else {
        Err(format!("docker run failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Starts the sim (release build) in its pane. Its output goes to the terminal.
pub fn start_sim(opts: &Options, rect: Option<Rect>) -> Result<Child, String> {
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
    if let Some(r) = rect {
        cmd.args(["--window-rect", &r.to_arg(), "--frameless"]);
    }
    cmd.spawn().map_err(|e| format!("starting {}: {e}", bin.display()))
}
