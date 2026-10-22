//! The commander app of Towers (see plans/towers.md): a person's seat on the game server.
//!
//!   cargo run --release -- --serve                   a game of your own: you command Blue against the computer
//!   cargo run --release -- --serve --red nobody      a game to practise in: nobody commands Red
//!   cargo run --release -- --connect HOST:5955       take the Blue seat of a game someone else runs
//!   cargo run --release -- --team red --watch        watch Red's map, with no orders

mod draw;
mod expect;
mod seat;
mod server;
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
  --connect ADDR     the game server (default 127.0.0.1:5955)
  --expect [SUITE]   run the acceptance tests in SUITE (default ../../../expectations) against this app,
                     each on a fresh game of its own server (needs the AGENT build of ArduPlane), and exit
  --only TEXT        with --expect: run only the tests whose title contains TEXT";

fn main() {
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
    let serve = flag("--serve") || expect;
    let addr = arg("--connect").unwrap_or_else(|| format!("127.0.0.1:{PORT}"));

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
    if expect {
        // The tests play a practice game: nothing of Red's gets in the way of what they check.
        server_args[3] = "nobody".into();
    }
    let server = serve.then(|| {
        if let Err(e) = server::build() {
            eprintln!("{e}");
            std::process::exit(2);
        }
        match server::Server::start(&server_args) {
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

    let expect_config = expect.then(|| {
        let suite = args
            .iter()
            .position(|a| a == "--expect")
            .and_then(|i| args.get(i + 1))
            .filter(|a| !a.starts_with("--"))
            .map(PathBuf::from)
            .unwrap_or_else(|| server::root().join("expectations"));
        expect::Config { suite, only: arg("--only"), server: server.clone().unwrap(), server_args: server_args.clone() }
    });

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
            titlebar: Some(TitlebarOptions {
                title: Some("SIM001 Towers".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            focus: true,
            ..Default::default()
        };
        let seat = seat.clone();
        let window = cx
            .open_window(options, |window, cx| cx.new(|cx| ui::CommanderView::new(seat, window, cx)))
            .expect("open the window");
        if let Some(config) = expect_config {
            expect::start(cx, window, config);
        }
        cx.activate(true);
    });
    drop(server);
    std::process::exit(expect::EXIT.load(std::sync::atomic::Ordering::SeqCst));
}
