//! The commander app of Towers (see plans/towers.md): a person's seat on the game server.
//!
//!   cargo run --release -- --serve                   a game of your own: you command Blue against the computer
//!   cargo run --release -- --serve --red nobody      a game to practise in: nobody commands Red
//!   cargo run --release -- --connect HOST:5955       take the Blue seat of a game someone else runs
//!   cargo run --release -- --team red --watch        watch Red's map, with no orders
//!   cargo run --release -- --serve --no-cameras      the same, with no 3D view and no pictures from the drones

mod bench;
mod cameras;
mod draw;
mod expect;
mod ground;
mod seat;
mod server;
mod snapshot;
mod station;
mod theme;
mod ui;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions,
    actions, px, size,
};
use recreation_towers::Team;
use recreation_towers::seat::{Join, PORT};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

actions!(sim001_command, [Quit]);

const HELP: &str = "\
sim001_command [options]
  --serve            start a game server (towers_server, with six ArduPlane SITL drones) and take your seat
  --team blue|red    the team you command or watch (default blue)
  --watch            watch the team's map, with no orders
  --red X, --blue X  with --serve: who commands the other team: computer (default) or nobody
  --speedup N        with --serve: how fast the drones' time runs (default 1)
  --no-cameras       no pictures from the drones' cameras (and with --serve, no 3D view of the game)
  --cameras PORT     the port of the first drone's camera on the 3D view (default 5601)
  --connect ADDR     the game server (default 127.0.0.1:5955)
  --expect [SUITE]   run the acceptance tests in SUITE (default ../../../expectations) against this app,
                     each on a fresh game of its own server (needs the AGENT build of ArduPlane), and exit
  --platform field   with --expect: read the suite as the field and run it on the bench (a ground radio and a
                     flight controller: SIM001_FIELD_RADIO, SIM001_FIELD_FC); without them, nothing is supported yet
  --only TEXT        with --expect: run only the tests whose title contains TEXT
  --docs DIR         with --expect: write the results and screenshots of the window to DIR (merged with the others')";

fn main() {
    suite_budget::begin();
    let args: Vec<String> = std::env::args().collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if flag("-h") || flag("--help") {
        println!("{HELP}");
        return;
    }
    let team = if arg("--team").is_some_and(|t| t.eq_ignore_ascii_case("red")) { Team::Red } else { Team::Blue };
    let join = if flag("--watch") { Join::Watch(team) } else { Join::Command(team) };
    let expect = flag("--expect");
    if expect && arg("--platform").is_some_and(|p| p == "field") {
        let suite = args
            .iter()
            .position(|a| a == "--expect")
            .and_then(|i| args.get(i + 1))
            .filter(|a| !a.starts_with("--"))
            .map(PathBuf::from)
            .unwrap_or_else(|| server::root().join("expectations"));
        std::process::exit(bench::run(&suite, arg("--only").as_deref()));
    }
    let serve = flag("--serve") || expect;
    // A server of its own takes a free port for its seats (the usual one may be in use), which
    // the app, the 3D view and any restarted game all use.
    let own_port = if serve && arg("--connect").is_none() { Some(server::free_port(PORT)) } else { None };
    let addr = arg("--connect").unwrap_or_else(|| format!("127.0.0.1:{}", own_port.unwrap_or(PORT)));
    let cameras_on = !flag("--no-cameras");

    // The server's arguments: this app's team is a person's; the other team's is the computer's,
    // or nobody's.
    let other = team.other();
    let other_flag = if other == Team::Red { "--red" } else { "--blue" };
    let mut server_args = vec![
        if team == Team::Red { "--red" } else { "--blue" }.to_string(),
        "person".to_string(),
        other_flag.to_string(),
        arg(other_flag).unwrap_or_else(|| "computer".into()),
        "--speedup".to_string(),
        arg("--speedup").unwrap_or_else(|| if expect { "10".into() } else { "1".into() }),
    ];
    if let Some(port) = own_port {
        server_args.extend(["--seat-port".to_string(), port.to_string()]);
    }
    if expect {
        // The tests play a practice game: nothing of Red's gets in the way of what they check.
        server_args[3] = "nobody".into();
    }
    let server = serve.then(|| {
        if let Err(e) = server::build(cameras_on) {
            eprintln!("{e}");
            std::process::exit(2);
        }
        if cameras_on && let Err(e) = server::ensure_map(&ground::assets_dir()) {
            // The map still works, on the plain ground.
            eprintln!("{e}");
        }
        match server::Server::start(&server_args, cameras_on) {
            Ok(s) => Arc::new(Mutex::new(Some(s))),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
    });
    if serve {
        println!("game server started ({}); its log is logs/towers/server.log", server_args.join(" "));
    }
    let seat = seat::Seat::connect(&addr, join);
    // A seat that commands is the team's ground station: it flies the team's drones itself.
    if matches!(join, Join::Command(_)) {
        station::start(seat.clone(), addr.rsplit_once(':').map_or("127.0.0.1", |(h, _)| h).to_string());
    }
    // The pictures from the team's drones: the 3D view of the game is on the server's machine.
    let cameras = match join {
        Join::Command(t) | Join::Watch(t) if cameras_on => cameras::Cameras::connect(
            addr.rsplit_once(':').map_or("127.0.0.1", |(h, _)| h),
            arg("--cameras").and_then(|p| p.parse().ok()).unwrap_or(cameras::FIRST_PORT),
            t,
            recreation_towers::Rules::default().drones,
        ),
        _ => cameras::Cameras::none(),
    };

    let expect_config = expect.then(|| {
        let suite = args
            .iter()
            .position(|a| a == "--expect")
            .and_then(|i| args.get(i + 1))
            .filter(|a| !a.starts_with("--"))
            .map(PathBuf::from)
            .unwrap_or_else(|| server::root().join("expectations"));
        expect::Config {
            suite,
            only: arg("--only"),
            docs: arg("--docs").map(PathBuf::from),
            server: server.clone().unwrap(),
            server_args: server_args.clone(),
            cameras: cameras_on,
        }
    });

    // The picture of the valley under the map, if it has been exported.
    let ground = ground::Ground::load(&ground::assets_dir()).map(Arc::new);
    let stop = server.clone();
    Application::new().run(move |cx: &mut App| {
        // The server (and its drones) stop with the app.
        cx.on_action(move |_: &Quit, cx| {
            if let Some(s) = &stop {
                s.lock().unwrap().take();
            }
            cx.quit()
        });
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu { name: "SIM001 Towers".into(), items: vec![MenuItem::action("Quit", Quit)] }]);
        let bounds = Bounds::centered(None, size(px(1400.0), px(900.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // Below this the map and the 340 px side column would not fit.
            window_min_size: Some(size(px(900.0), px(600.0))),
            titlebar: Some(TitlebarOptions {
                title: Some("SIM001 Towers".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            focus: true,
            ..Default::default()
        };
        let (seat, cameras, ground) = (seat.clone(), cameras.clone(), ground.clone());
        let window = cx
            .open_window(options, |window, cx| cx.new(|cx| ui::CommanderView::new(seat, cameras, ground, window, cx)))
            .expect("open the window");
        if let Some(config) = expect_config {
            expect::start(cx, window, config);
        }
        cx.activate(true);
    });
    drop(server);
    std::process::exit(expect::EXIT.load(std::sync::atomic::Ordering::SeqCst));
}
