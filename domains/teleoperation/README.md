# Teleoperation

The pilot's end of the link: the TX15 and its EdgeTX settings, and the ground control station that flies ArduPlane SITL and the sim over MAVLink.

| Path | What it holds |
| --- | --- |
| [console/](console/README.md) | `teleoperation_console` (binary `sim001_gcs`): the GPUI console with a PFD, a tactical map, mission planning and the sensor feed. Excluded from the workspace: it keeps its own lockfile |
| [command/](command/README.md) | `teleoperation_command` (binary `sim001_command`): the commander app of Towers, a person's seat on the game server (`recreation_towers_server`). Excluded from the workspace: it keeps its own lockfile |
| [shared/](shared) | `teleoperation_shared`: what the sim and the console both use, the sensor feed wire protocol and the window layout planner. No dependencies |
| [edgetx/](edgetx) | The radio's SD card as files: `MODELS/` (the T1S VTOL model), an empty `RADIO/` (the radio's own settings folder; the SD card has one) and `THEMES/` (SIM001 Night and SIM001 Day, generated from the shared theme by `cargo run -p illustration_theme --example edgetx`) |

## Previewing in EdgeTX Companion

`edgetx/` has the same layout as the SD card, so Companion's simulator can use it directly:
open Companion, edit the radio profile (Settings, Radio Profiles) and set its **SD structure path**
to this folder, then press Simulate Radio (the toolbar's "Read from SD path" wants a `RADIO/radio.yml` and complains without one; the simulator doesn't). On the simulated radio, SYS then PAGE> to the Themes tab, long-press a theme and choose Set Active. The model and both themes show up under Radio
Settings, Themes, with no copying. Regenerate the themes and restart the simulator to see a
change.

## The radio in serial mode

For the flight computer (see [aviation](../aviation/README.md)), the radio's USB serial port has
to be in the `LUA` mode and the `SIM001FC` widget (`WIDGETS/SIM001FC`) has to be on the model's main view. The widget is tracked here;
Companion's mirror of the SD card holds the rest, which is not.


## Debugging the autopilot

The station keeps what it knows of ArduPilot as a debug log, so that a plane that won't arm or a test that fails can be worked out from what passed between them.

- **The log** (`teleoperation_core::debug`): every command sent (with its parameters), every acknowledgement (a refusal is a warning), everything the autopilot said, the heartbeat lost and back, what changed in its state (mode, armed, GPS fix, EKF flags, a sensor going unhealthy), and the pings. Repeats are kept: the operator's MESSAGES list collapses them, this does not. The console writes it as it happens to `logs/debug/console.log`.
- **The ping:** a TIMESYNC every 2 s that ArduPilot echoes. The round trip and the loss are in the top bar (`PING 21 ms`) and the report.
- **The report:** link, heartbeat age, mode, armed, sensors that are present but not healthy (by name), whether the pre-arm checks pass and the last reason they didn't, EKF, GPS, last acknowledgement.
- **ArduPlane's own output** is kept in `arduplane.log` in its folder (`logs/sitl/`, `logs/sitl/opponent-N/`), after a line that says when each start was.
- **In the console:** the DEBUG button on the MESSAGES screen shows the report, the end of ArduPlane's output and the events, newest first. `SIM001_DEBUG_VIEW=1` opens on it.
- **In the launcher:** the log pane has a tab for each log file of a run (the console's debug log, ArduPlane's output), read as it grows.
- **When a console test fails** it prints the report, the last 80 events and the end of ArduPlane's output.
