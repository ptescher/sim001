//! SIM001 launcher: set up your controllers, render the docs, and start the simulator on its own
//! or the whole ground station, with or without the ArduPilot flight controller.
//!
//!   cargo run --release                          (from this directory)
//!   cargo run --release -- --expect --docs DIR   its acceptance tests (expectations/10-launcher)

mod app;
mod controllers;
mod expect;
mod job;
mod snapshot;
mod theme;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions,
    actions, px, size,
};
use std::path::PathBuf;

actions!(sim001_launcher, [Quit]);

const HELP: &str = "\
sim001_launcher [options]
  --expect [SUITE]   run the launcher's acceptance tests in SUITE (default ../../../expectations) against
                     this window, with a stand-in for each program it starts, and exit
  --only TEXT        with --expect: run only the tests whose title contains TEXT
  --docs DIR         with --expect: write the results and screenshots of the window to DIR";

fn main() {
    suite_budget::begin();
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return;
    }
    // `--stand-in PROGRAM ARGS...`: what the tests start in place of a program (see job.rs).
    if let Some(i) = args.iter().position(|a| a == "--stand-in") {
        job::stand_in(&args[i + 1..]);
    }
    let expect_config = args.iter().position(|a| a == "--expect").map(|i| {
        let suite = args
            .get(i + 1)
            .filter(|a| !a.starts_with("--"))
            .map(PathBuf::from)
            .unwrap_or_else(|| job::layout().home.join("expectations"));
        expect::Config { suite, only: arg("--only"), docs: arg("--docs").map(PathBuf::from) }
    });
    if expect_config.is_some() {
        // A data folder of the run's own (controls.cfg, the logs), and a stand-in for every program.
        let data = std::env::temp_dir().join(format!("sim001_launcher_expect_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&data);
        // SAFETY: no other thread runs yet.
        unsafe { std::env::set_var("SIM001_DATA", &data) };
        job::use_stand_in(std::env::current_exe().expect("the launcher's own path"));
    }
    Application::new().run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu { name: "SIM001 Launcher".into(), items: vec![MenuItem::action("Quit", Quit)] }]);
        let bounds = Bounds::centered(None, size(px(1100.0), px(840.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    // Below this the panels would run off the window: the left column is 520 wide.
                    window_min_size: Some(size(px(900.0), px(560.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some("SIM001 Launcher".into()),
                        appears_transparent: true,
                        traffic_light_position: None,
                    }),
                    focus: true,
                    ..Default::default()
                },
                |_, cx| cx.new(app::Launcher::new),
            )
            .expect("open launcher window");
        // Closing the launcher stops what it started.
        let view = window.entity(cx).ok();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.on_app_quit(move |cx| {
            if let Some(v) = &view {
                v.update(cx, |l, _| l.shutdown());
            }
            async {}
        })
        .detach();
        if let Some(config) = expect_config {
            expect::start(cx, window, config);
        }
        cx.activate(true);
    });
    std::process::exit(expect::EXIT.load(std::sync::atomic::Ordering::SeqCst));
}
