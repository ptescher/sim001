# The launcher

The launcher is the window you start SIM001 from. It starts the sim alone, or the whole ground station. It also sets up your controllers and opens this manual. In the app bundle, you open it as `SIM001.app`.

From source, start it with `cd domains/teleoperation/launcher && cargo run --release`.

## What it does

- **Launch.** Start the sim alone, or the whole station (the sim, ArduPilot SITL and the console). Choose *With ArduPilot* to fly the T1-S VTOL. Choose *Without* to fly the plain T1-S Ranger. In the station, *Without* starts the sim with `--direct`, so your sticks fly the plane through the sim's own modes. You can also pick the place, ray tracing, full screen, and whether to skip the build.
- **Controllers.** The launcher lists the USB joysticks and gamepads that are plugged in, and what `controls.cfg` maps. Press *Set up controller* to start the sim with the radio wizard, which opens on its own when it sees a controller.
- **Docs.** Open this manual. From source, the launcher can also draw it again, or run the tests first.
- **Log.** The output of the program that is running. Press Stop, or close the launcher, to end that program and all that it started.

## A radio on the serial line

A radio with USB Serial (VCP) set to LUA shows up as a serial device, not as a joystick. The launcher finds it and lists it under *Controllers*. With ArduPilot, *TX15 on serial* starts the station on that device. The sticks, the telemetry and the console's MAVLink then go through the radio, as on the real plane. See [[The flight computer]].

## Next

Set up your controls: [[Controls]]. Then fly your first throw with [[The simulator]], or your first hover with [[Flying with a flight controller]].
