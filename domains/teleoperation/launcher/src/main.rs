//! SIM001 launcher: set up your controllers, render the docs, and start the simulator on its own
//! or the whole ground station, with or without the ArduPilot flight controller.
//!
//!   cargo run --release        (from this directory)

mod app;
mod controllers;
mod job;
mod theme;

use gpui::{
    App, AppContext, Application, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions,
    actions, px, size,
};

actions!(sim001_launcher, [Quit]);

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu { name: "SIM001 Launcher".into(), items: vec![MenuItem::action("Quit", Quit)] }]);
        let bounds = Bounds::centered(None, size(px(1100.0), px(760.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
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
        cx.activate(true);
    });
}
