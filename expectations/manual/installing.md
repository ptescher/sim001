# Requirements and installation

This page tells you what a computer needs to run SIM001, and how to install it. Read it first. When you can start the sim, go on to [[The launcher]].

## What you need

- A Mac with Apple Silicon is the main platform. Ray tracing needs hardware ray tracing: an M3 or M4 chip, or other hardware that supports it. Without it, use the default rasterized view.
- Windows and Linux builds exist but are untested.
- Nothing else for the app. To build from source: Rust (the file `rust-toolchain.toml` pins the version, and `rustup` installs it for you), and Docker Desktop if you have no native ArduPlane build.
- For a radio: a RadioMaster TX15 (or another EdgeTX radio). A Nintendo Switch Pro Controller and the keyboard also work. See [[Controls]].

## Download the app

1. Download the latest release from [GitHub Releases](https://github.com/ptescher/sim001/releases/latest): `SIM001-…-macos-arm64.zip` for a Mac with Apple Silicon, or `SIM001-…-linux-x86_64.tar.gz` for Linux.
2. On macOS, unzip it and open `SIM001.app`. The app is not notarized, so the first time macOS says that it cannot check it. Open System Settings, then Privacy & Security, and click Open Anyway.
3. On Linux, unpack the archive and run `sim001_launcher` in its folder.

The app opens [[The launcher]]. The app holds the launcher, the sim, the ground station, ArduPlane SITL, the maps and this manual, so you need nothing else.

Your radio settings (`controls.cfg`) and the flight logs go to your data folder. On macOS this is `~/Library/Application Support/SIM001`. The first run copies the example radio settings there.

## Run from source

Build from source to use the latest version, or on Windows.

1. Install [Rust](https://rustup.rs) and Git.
2. Clone the repository: `git clone https://github.com/ptescher/sim001` and `cd sim001`.
3. Build ArduPlane SITL once, with no Docker: `ardupilot/build_native.sh`. It builds into `.ardupilot/`. Without a native ArduPlane build, the ground station uses the Docker image `sim001-ardupilot-sitl` and needs Docker Desktop. The first time, it builds the image. This takes about 15 minutes. After that it starts in a few seconds.
4. Start the launcher: `cargo run --release`. The first build takes several minutes.
5. To start the sim without the launcher: `cargo run --release --bin sim001`. It flies the T1-S VTOL. For the plain T1-S Ranger, add `-- --aircraft t1s-ranger`. [[Command-line options]] lists the rest.

To make an app of your own, as the release does: `tools/bundle.sh`. It writes `target/bundle/SIM001.app` on macOS, and a folder and an archive on Linux and Windows.

## Ray tracing

Add `--raytrace` to any start command to use ray-traced shadows, global illumination and reflections. The 3D view then draws at a lower internal size and scales up, and the HUD stays sharp. Press F5 to lower the render size and F6 to raise it. If the frame rate is too low, set `SIM001_TREE_FRAC=0.5` to draw half of the forest.

## Sleep and full screen

While the sim runs, it keeps the computer awake. `--fullscreen` starts in borderless full screen, and F11 switches full screen on and off. To let macOS use Game Mode, run the app bundle with `tools/macos-app.sh` and start it with `--fullscreen`.
