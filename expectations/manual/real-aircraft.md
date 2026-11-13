# Connecting to a real aircraft

The ground control console also talks to a real aircraft. It uses [[MAVLink]], as Mission Planner and QGroundControl do, so it works with an aircraft that runs ArduPilot. Use this page after you can plan and fly a mission in the sim: see [[From plan to flight]].

Do the first flights with a tool that you know, and with a safety pilot on the radio. The console is built for this sim and for the T1-S. It is not tested on other aircraft.

## What you need

- An aircraft with ArduPilot, and a telemetry link from it to your computer.
- The link as a network address. The console connects by TCP or UDP. It does not open a serial port.
- A build from source, or the app. In the app on macOS, the console is the program `SIM001.app/Contents/MacOS/sim001_gcs`.

If your telemetry radio is a serial device, let MAVProxy pass it on. For example, `mavproxy.py --master=/dev/tty.usbserial-0001 --baud=57600 --out=udp:127.0.0.1:14550` sends the telemetry to UDP port 14550 of your computer.

## Start the console

Start the console alone, without `--station`. The sim and ArduPilot SITL do not start.

From source:

`cd domains/teleoperation/console && cargo run --release -- --connect udpin:0.0.0.0:14550`

`--connect` takes one of these addresses:

| Address | Use it when |
| --- | --- |
| `udpin:0.0.0.0:14550` | The aircraft, or MAVProxy with `--out`, sends telemetry to your computer on UDP port 14550 |
| `udpout:HOST:PORT` | The console must send the first packet, for example to a telemetry bridge on Wi-Fi |
| `tcpout:HOST:PORT` | The link is a TCP server, for example ArduPilot SITL on `tcpout:127.0.0.1:5760` |

The console shows LINK DOWN, in red, until it hears the aircraft. If the link drops, the console connects again by itself.

## Put the map in the right place

The tactical map measures positions in metres from one point, the origin. Without an origin, the console uses the pilot box of the Alpine Valley, and your aircraft is not on the map.

- For a place that has a map pack, add `--map NAME`. The map shows the aerial photo of the place. [[Places and scenery]] tells you how to build a pack for your field.
- For any other place, add `--origin LAT,LON,ALT` with the position of your launch point: degrees, degrees and metres above sea level. The map then shows a plain grid, 100 m to a square.

## Plan in the sim, fly at the field

1. Plan the mission in the sim, on the map pack of your field, and fly it there first.
2. Press SAVE. The console writes `mission.wpl`. See [[Mission files]].
3. At the field, connect to the aircraft. Press LOAD, and then UPLOAD.
4. Press DOWNLOAD, and compare the solid line of the mission on the aircraft with your plan.

Mission Planner and QGroundControl read `mission.wpl` too.

## What is different from the sim

- There is no sim window and no sensor feed.
- The console shows the mode buttons of the T1-S VTOL. ArduPilot refuses a mode that your aircraft does not have.
- The console does not change the parameters of the aircraft. Set them up with Mission Planner or QGroundControl.
- To watch the aircraft in the 3D scenery, start `sim001_view`: see [[Watching a real plane]].
