# Teleoperation

The pilot's end of the link: the TX15 and its EdgeTX settings.

| Path | What it holds |
| --- | --- |
| [edgetx/](edgetx) | The radio's SD card as files: `MODELS/` (the T1S VTOL model), an empty `RADIO/` (the radio's own settings folder; the SD card has one) and `THEMES/` (T1S Night and T1S Day, generated from the shared theme by `cargo run -p illustration_theme --example edgetx`) |

## Previewing in EdgeTX Companion

`edgetx/` has the same layout as the SD card, so Companion's simulator can use it directly:
open Companion, edit the radio profile (Settings, Radio Profiles) and set its **SD structure path**
to this folder, then press Simulate Radio (the toolbar's "Read from SD path" wants a `RADIO/radio.yml` and complains without one; the simulator doesn't). On the simulated radio, SYS then PAGE> to the Themes tab, long-press a theme and choose Set Active. The model and both themes show up under Radio
Settings, Themes, with no copying. Regenerate the themes and restart the simulator to see a
change.

## The radio in serial mode

For the flight computer (see [avionics](../avionics/README.md)), the radio's USB serial port has
to be in the `LUA` mode and `SCRIPTS/TELEMETRY/t1sfc.lua` has to run. The script is tracked here;
Companion's mirror of the SD card holds the rest, which is not.

