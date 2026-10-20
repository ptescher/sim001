# Requirements and installation

This page tells you what a computer needs to run SIM001, and how to install it. Read it first. When you can start the sim, go on to [[The launcher]].

## What you need

- A Mac with Apple Silicon is the main platform. Ray tracing needs hardware ray tracing: an M3 or M4 chip, or other hardware that supports it. Without it, use the default rasterized view.
- Windows and Linux builds exist but are untested.
- Rust 1.99 to build from source. The file `rust-toolchain.toml` pins it, and `rustup` installs it for you.
- Docker Desktop, if you use the ground station and have no native ArduPlane build.
- For a radio: a RadioMaster TX15 (or another EdgeTX radio). A Nintendo Switch Pro Controller and the keyboard also work. See [[Controls]].

## Install the app

1. Build the app bundle on the system you want to use it on: `tools/bundle.sh`.
2. On macOS, open `target/bundle/SIM001.app`. The bundle holds the launcher, the sim, the ground station, ArduPlane SITL, the maps and this manual.
3. On Linux and Windows, the script makes a folder and an archive that hold the same files.

Your radio settings (`controls.cfg`) and the flight logs go to your data folder. On macOS this is `~/Library/Application Support/SIM001`. The first run copies the example radio settings there.

## Run from source

1. Clone the repository.
2. Build ArduPlane SITL once, with no Docker: `ardupilot/build_native.sh`. It builds into `.ardupilot/`.
3. Start the T1-S VTOL, which is the default plane: `cargo run --release`.
4. For the plain T1-S Ranger, which has no flight controller: `cargo run --release -- --aircraft t1s-ranger`.
5. For the whole ground station, start it from the console folder: `cd domains/teleoperation/console && cargo run --release -- --station`.

Without a native ArduPlane build, the sim uses the Docker image `sim001-ardupilot-sitl`. The first time, the station builds it. This takes about 15 minutes. After that it starts in a few seconds.

## Ray tracing

Add `--raytrace` to any start command to use ray-traced shadows, global illumination and reflections. The 3D view then draws at a lower internal size and scales up, and the HUD stays sharp. Press F5 to lower the render size and F6 to raise it. If the frame rate is too low, set `SIM001_TREE_FRAC=0.5` to draw half of the forest.

## Sleep and full screen

While the sim runs, it keeps the computer awake. `--fullscreen` starts in borderless full screen, and F11 switches full screen on and off. To let macOS use Game Mode, run the app bundle with `tools/macos-app.sh` and start it with `--fullscreen`.
