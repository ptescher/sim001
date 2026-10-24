//! SIM001 ground control station (GPUI) for ArduPlane SITL driven by the sim001 physics.
//!
//!   cargo run --release -- --station                      whole station: SITL, sim, feed, console
//!   cargo run --release                                   console only, tcp:127.0.0.1:5760
//!   cargo run --release -- --sensor                       console + sensor feed window
//!   cargo run --release -- --connect udpin:0.0.0.0:14550  any rust-mavlink address

mod autotest;
mod expect;
mod feed;
mod screens;
mod station;
mod tx15_screen;
mod ui;
// The screenless half of the station lives in teleoperation_core; keep the old paths.
use teleoperation_core::{debug, geo, link, vehicle};

use geo::{Origin, TerrainMap};
use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions,
    WindowBounds, WindowOptions, actions, point, px, size,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use teleoperation_shared::layout::{self, Mode, Pane, Rect};
use vehicle::Vehicle;

actions!(sim001_gcs, [Quit]);

const HELP: &str = "\
sim001_gcs [options]
  --station          start everything: builds the sim, exports the map if missing, starts Docker
                     Desktop, builds the SITL image if missing, starts SITL and the sim with the
                     sensor feed, and lays out sim view, sensor feed and console over your displays
  --no-build         with --station: don't rebuild the sim first
  --map NAME         fly a real place from maps/NAME (cypress, berkeley): with --station it builds
                     the map pack if needed and starts SITL at that place's home
  --aircraft ID      with --station: the plane to fly (default t1s-vtol: the plain t1s-ranger has no flight controller, so no console); a VTOL adds the Q modes
  --direct           with --station: start with the sticks flying through the sim's own modes
                     (ArduPilot keeps running and reporting, its outputs ignored; J toggles)
  --layout single    with --station: tile all three on the main display even with several
  --raytrace         with --station: run the sim with ray tracing
  --sensor           open the sensor feed window (sim already running with --feed)
  --feed HOST:PORT   sensor feed address (default 127.0.0.1:5600)
  --worker N         with --station: this is station N (default 0), for tests that run several at once:
                     its ports, SITL instances, containers and log folders are its own
  --connect ADDR     MAVLink, any rust-mavlink address (default tcpout:127.0.0.1:5760; with
                     --station the sim's TX15 stand-in, tcpout:127.0.0.1:5750)
  --no-radio         with --station: connect to ArduPilot directly, without the TX15 stand-in
  --flight-computer  with --station: the plane's radio goes through the sim's flight computer, on a UART
                     in CRSF, and the console hears only the telemetry that comes down that line
  --tx15-serial DEV  with --station: a real TX15 on its USB cable (DEV is its serial device, with its
                     USB serial set to LUA and the t1sfc script running): the sticks, the telemetry and
                     the console's MAVLink all go through the radio
  --origin LAT,LON[,ALT]  SITL --home = the sim's pilot box (default matches ardupilot/start.sh)
  --assets DIR       output of `sim001 --export-map` (default domains/teleoperation/console/assets)
  --expect [SUITE]   run the acceptance tests in SUITE (default ../../../expectations) against this console
                     by clicking it, and exit; needs a vehicle, so use it with --station
  --only TEXT        with --expect: run only the tests whose title contains TEXT
  --docs DIR         with --expect: write the manual and screenshots to DIR (merged with the sim's)
  --autotest         SITL only: upload a circuit, arm, fly AUTO, GUIDED goto, RTL";

fn main() {
    suite_budget::begin();
    let args: Vec<String> = std::env::args().collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    if flag("-h") || flag("--help") {
        println!("{HELP}");
        return;
    }
    // Which station this is, when the tests run several at once: it decides the ports.
    if let Some(n) = arg("--worker").and_then(|n| n.parse().ok()) {
        station::set_worker(n);
    }
    let ports = station::ports();
    let station_mode = flag("--station");
    // With --station the console's only source of telemetry is the radio, which the sim stands in for.
    let radio_port = (station_mode && !flag("--no-radio")).then_some(ports.radio());
    let address = arg("--connect").unwrap_or_else(|| match radio_port {
        Some(port) => format!("tcpout:127.0.0.1:{port}"),
        None => format!("tcpout:127.0.0.1:{}", ports.mavlink()),
    });
    let map = arg("--map");
    let mut origin = arg("--origin")
        .and_then(|s| Origin::parse(&s))
        .unwrap_or(Origin::SITL_DEFAULT);
    // A real place's map lives in domains/teleoperation/console/assets/<name>; the Alpine Valley's in domains/teleoperation/console/assets.
    let assets = arg("--assets")
        .map(PathBuf::from)
        .unwrap_or_else(|| match &map {
            Some(m) => default_assets().join(m),
            None => default_assets(),
        });
    let layout_mode = if arg("--layout").as_deref() == Some("single") {
        Mode::Single
    } else {
        Mode::Auto
    };
    // The tests' TX15 is the console's own, on the flight computer's serial line.
    let hosted = (radio_port.is_some() && flag("--expect") && arg("--tx15-serial").is_none()).then(|| {
        aviation_line::Hosted::host(&format!("127.0.0.1:{}", ports.uart())).unwrap_or_else(|e| {
            eprintln!("the TX15 for the tests has no line: {e}");
            std::process::exit(1);
        })
    });
    let feed_addr =
        arg("--feed").unwrap_or_else(|| format!("127.0.0.1:{}", ports.feed()));
    let opts = station::Options {
        raytrace: flag("--raytrace"),
        feed_port: feed_addr
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(ports.feed()),
        no_build: flag("--no-build"),
        direct: flag("--direct"),
        map: map.clone(),
        aircraft: arg("--aircraft"),
        test_radio: flag("--expect") && hosted.is_none(),
        radio_port,
        flight_computer: radio_port.is_some() && flag("--flight-computer"),
        // The simulated TX15 on the serial line writes its screen here, for the tests to read.
        tx15_screen: (radio_port.is_some() && flag("--flight-computer") && flag("--expect"))
            .then(|| std::env::temp_dir().join(format!("sim001_tx15_screen_{}.json", std::process::id()))),
        tx15_serial: radio_port.and(arg("--tx15-serial")),
        hosted_radio: hosted.as_ref().map(|(_, relay)| *relay),
    };
    let aircraft = match &opts.aircraft {
        None => simulation_aircraft::default(),
        Some(id) => simulation_aircraft::find(id).unwrap_or_else(|| {
            eprintln!(
                "--aircraft {id}: no such plane. Available: {}",
                simulation_aircraft::ids().join(", ")
            );
            std::process::exit(1);
        }),
    };
    let vtol = (aircraft.params)().motors.iter().any(|m| m.lift);
    if station_mode && !(aircraft.params)().flight_controller {
        eprintln!(
            "--station: the {} has no flight controller (only a receiver and ESCs), so there is nothing for the console to talk to. Fly it with `cargo run --release` in the sim",
            aircraft.title
        );
        std::process::exit(1);
    }
    if station_mode {
        if let Err(e) = station::prepare(&opts, &assets) {
            eprintln!("\x1b[1;31mstation: {e}\x1b[0m");
            std::process::exit(1);
        }
    }

    // The map's home is ArduPilot's home and the sim's pilot box: everything is relative to it.
    if let Some(m) = &map {
        match station::pack_meta(m) {
            Some(meta) => {
                origin = Origin {
                    lat: meta.home_lat,
                    lon: meta.home_lon,
                    alt: meta.home_alt,
                }
            }
            None => eprintln!("map {m} isn't built yet (run with --station, or domains/visualization/mapbuild)"),
        }
    }
    let terrain = TerrainMap::load(&assets).map(Arc::new);
    match &terrain {
        Some(t) => println!("terrain map {} ({:.0} m square)", t.png.display(), t.size_m),
        None => println!(
            "no terrain map in {} (run `cargo run --release -- --export-map domains/teleoperation/console/assets` in the sim)",
            assets.display()
        ),
    }

    let vehicle = Arc::new(Mutex::new(Vehicle::new(&address)));
    // What happens between the console and the autopilot goes to a file as it happens, where the
    // launcher reads it (and what a test that fails prints).
    if let Err(e) = vehicle.lock().unwrap().debug.to_file(&station::logs_dir().join("debug/console.log")) {
        eprintln!("the debug log has no file: {e}");
    }
    let tx = link::spawn(address, vehicle.clone());
    if flag("--autotest") {
        autotest::spawn(vehicle.clone(), tx.clone(), origin);
    }
    let sensor = (station_mode || flag("--sensor")).then(|| {
        feed::spawn(
            feed_addr,
            vehicle.clone(),
            tx.clone(),
            origin,
            terrain.clone(),
        )
    });

    let expect = flag("--expect").then(|| {
        let suite = args
            .iter()
            .position(|a| a == "--expect")
            .and_then(|i| args.get(i + 1))
            .filter(|a| !a.starts_with("--"));
        expect::Config {
            suite: suite.map(PathBuf::from).unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../expectations")
            }),
            docs: arg("--docs").map(PathBuf::from),
            only: arg("--only"),
            vtol,
            rc_modes: (aircraft.params)().rc_modes,
            radio: hosted.map(|(radio, _)| radio),
            vehicle: vehicle.clone(),
        }
    });
    Application::new().run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "SIM001 Ground Control".into(),
            items: vec![MenuItem::action("Quit", Quit)],
        }]);
        let displays = screens::displays(cx);
        // The windows are frameless (content under a transparent title bar), so a layout
        // rectangle is the content rectangle.
        let gpui_bounds = |r: Rect| Bounds {
            origin: point(px(r.x), px(r.y)),
            size: size(px(r.w), px(r.h)),
        };
        let plan = if station_mode {
            layout::plan(&displays, layout_mode)
        } else {
            vec![]
        };
        let rect_of = |pane: Pane| plan.iter().find(|p| p.pane == pane).map(|p| p.rect);
        if station_mode {
            println!("station: {} display(s)", displays.len());
            // With no display given, GPUI places a frame's top-left relative to the main
            // display, so global coordinates reach every display.
            for p in &plan {
                println!(
                    "  {:?} on display {} at {}",
                    p.pane,
                    p.display,
                    p.rect.to_arg()
                );
            }
            match station::start_sim(
                &opts,
                rect_of(Pane::Sim),
                (origin.lat, origin.lon, origin.alt),
            ) {
                Ok(child) => station::keep_sim(child),
                Err(e) => {
                    eprintln!("station: {e}");
                    vehicle.lock().unwrap().push_log("GCS", 3, e);
                }
            }
        }

        let console_bounds = rect_of(Pane::Console)
            .map(gpui_bounds)
            .unwrap_or_else(|| Bounds::centered(None, size(px(1840.0), px(1080.0)), cx));
        let console = cx
            .open_window(
                window_options("SIM001 Ground Control", console_bounds, (800.0, 540.0)),
                |window, cx| {
                    screens::hide_window_buttons(window);
                    cx.new(|cx| {
                        ui::app::Gcs::new(
                            vehicle.clone(),
                            tx,
                            origin,
                            terrain.clone(),
                            sensor.clone(),
                            vtol,
                            cx,
                        )
                    })
                },
            )
            .expect("open console window");

        if let Some(sensor) = sensor.clone() {
            let b = rect_of(Pane::Feed)
                .map(gpui_bounds)
                .unwrap_or_else(|| Bounds::centered(None, size(px(1280.0), px(760.0)), cx));
            let v = vehicle.clone();
            cx.open_window(window_options("SIM001 Sensor", b, (320.0, 240.0)), |window, cx| {
                screens::hide_window_buttons(window);
                cx.new(|cx| ui::sensor::SensorView::new(sensor, v, origin, cx))
            })
            .expect("open sensor window");
        }

        if let Some(config) = expect {
            expect::start(cx, console, config);
        }

        // Closing the console ends the session (and the sim it started).
        let console_id = console.window_id();
        cx.on_window_closed(move |cx| {
            if !cx.windows().iter().any(|w| w.window_id() == console_id) {
                cx.quit();
            }
        })
        .detach();
        cx.on_app_quit(move |_| {
            station::stop_sim();
            // (The window system can end the process before `main` gets to say how the run went.)
            let code = expect::EXIT.load(std::sync::atomic::Ordering::SeqCst);
            if code != 0 {
                std::process::exit(code);
            }
            async {}
        })
        .detach();
        cx.activate(true);
    });
    std::process::exit(expect::EXIT.load(std::sync::atomic::Ordering::SeqCst));
}

fn window_options(title: &'static str, bounds: Bounds<gpui::Pixels>, min: (f32, f32)) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(min.0), px(min.1))),
        // Content runs under a transparent title bar; our own header strips take its place.
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        is_minimizable: false,
        focus: true,
        ..Default::default()
    }
}

/// `domains/teleoperation/console/assets` whether run from the repo root or from the console's directory.
fn default_assets() -> PathBuf {
    if let Some(home) = std::env::var_os("SIM001_HOME") {
        return PathBuf::from(home).join("assets");
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    if manifest.exists() {
        manifest
    } else {
        PathBuf::from("domains/teleoperation/console/assets")
    }
}
