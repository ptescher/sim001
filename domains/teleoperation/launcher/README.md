# SIM001 Launcher

A [GPUI](https://gpui.rs) window, in the console's theme, that starts everything else:

```bash
cd domains/teleoperation/launcher && cargo run --release
```

- **Launch:** the simulator alone, or the whole ground station (sim, ArduPilot SITL and the console). *With ArduPilot* flies the T1-S VTOL (the station, or the sim, which starts its own ArduPilot). *Without* flies the plain T1-S Ranger in the sim, and in the station runs `--direct`: your sticks fly the plane through the sim's own modes while ArduPilot only reports. Pick the place, ray tracing, full screen or skipping the rebuild.
- **Controllers:** lists the USB joysticks and gamepads that are plugged in and what `controls.cfg` maps. *Set up controller* starts the sim with `--calibrate`, which opens the radio wizard (F2) as soon as it sees a controller, and the wizard saves `controls.cfg`.
- **Docs:** renders the manual from the specs and the last saved results (`expectation docs`), opens it, or re-runs the tests first (`cargo run --release -p sim001_tasks -- docs --no-station`).
- **Log:** the output of the one program running. Stop (or closing the launcher) ends it and everything it started. The SITL Docker container, if the station started one, is left to the station.

A radio with USB Serial (VCP) set to LUA shows up as a serial device, not a joystick. The launcher finds it (`/dev/cu.usbmodem…`, named by its USB product name) and lists it under Controllers. With ArduPilot, **TX15 on serial** (on by default when one is found) starts the station with `--tx15-serial`, or the sim alone with `--flight-computer`, on that device, so the sticks, the telemetry and the console's MAVLink go through the radio.

## In an app bundle

`tools/bundle.sh` (from the repository root) builds `target/bundle/SIM001.app` (macOS; Linux and Windows get a folder with the launcher beside the programs). The launcher finds the bundle by a `sim001-bundle` marker in its resources, starts the bundled `sim001` and `sim001_gcs` directly instead of through cargo, and hands them `SIM001_HOME` (maps, docs, ArduPilot's files, the terrain map), `SIM001_BIN` and `SIM001_DATA`. `controls.cfg` and the flight logs live in the user's data folder, so the app can stay read-only. In a bundle there is nothing to build, so the docs buttons that render or re-run tests are hidden.
