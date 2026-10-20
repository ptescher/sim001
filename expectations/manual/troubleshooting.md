# Troubleshooting

Find your problem in the first column.

| Problem | What to do |
| --- | --- |
| The ARM button reads *ARM: NOT READY* | ArduPilot needs about a minute after the start. Wait until the button reads ARM. |
| The first start of the station takes a long time | The station builds the ArduPlane SITL image the first time. This takes about 15 minutes. After that it starts in a few seconds. |
| The station says Docker is not running | Start Docker Desktop. The station also starts it when it can. Or build ArduPlane natively with `ardupilot/build_native.sh`. |
| The sim does not see my radio | Choose *USB Joystick (HID)* when EdgeTX asks. Press F3 in the sim to see the raw axes. A radio in serial mode (VCP set to LUA) is not a joystick: see [[The launcher]]. |
| The sticks move the wrong way, or on the wrong axis | Press F2 and calibrate again. To turn one direction round, change its scale to −1 in `controls.cfg`. |
| A switch or a button does nothing | The axis in `controls.cfg` is a guess. Press F3, use the switch, and put the axis that changes on the matching line. |
| The Switch Pro Controller gives phantom presses | Run with `--debug-input` to print what the sim reads. The sim reads this controller directly over HID, so do not use another tool with it at the same time. |
| The picture is slow | Do not use `--raytrace`. Press F5 to lower the render size. Set `SIM001_TREE_FRAC=0.5`. |
| `--raytrace` fails or the view is black | Ray tracing needs hardware ray tracing (Apple M3 or M4). Start without `--raytrace`. |
| The plane will not throw | Hold the throttle above 55 %, or press Space. During a pause (P), the plane waits. |
| I crashed | Press R to pick the plane up again. Press V to fit a fresh battery. |
| The mission does nothing in AUTO | Press UPLOAD first, and arm the plane. A hand launch waits for your throw: see [[Takeoff and return]]. |
| A map or a place is missing | Run the station with `--map NAME`: it builds the map pack the first time. |

If your problem is not here, look in the glossary for the word, or press H in the sim for the keys.
