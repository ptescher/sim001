# Aviation

What sits between the TX15 and the autopilot. In the sim it is a simulated **flight computer**:
the TX15 talks to it on a serial line (UART) in CRSF, the protocol of ExpressLRS, instead of
being a USB game controller (HID). It turns the TX15's channels into the radio channels that
ArduPilot reads. It sends ArduPilot's state back to the TX15 as CRSF telemetry, and it carries
ArduPilot's MAVLink down the same line, so the ground station's telemetry comes from the line.

```text
TX15 ──CRSF channels──▶ flight computer ──rc channels──▶ ArduPilot (JSON SITL)
TX15 ◀─CRSF telemetry── flight computer ◀──state, MAVLink── ArduPilot
 │  └─ MAVLink envelopes (both ways) ─▶ ground station
```

| Crate | Path | Role |
| --- | --- | --- |
| `aviation_crsf` | [crsf/](crsf) | The CRSF frames: the checksum, a stream framer, packed channels, the telemetry frames and the MAVLink envelope. No dependencies |
| `aviation_computer` | [computer/](computer) | The `FlightComputer` (channels in, failsafe, telemetry out, MAVLink carried both ways within the line's rate) and the `Handset`, the TX15's end of the line |

In the sim: `--flight-computer tcp:PORT|/dev/ttyX` puts the flight computer on a UART (see
`src/flight_computer.rs`). `--radio PORT --radio-uart tcp:HOST:PORT` makes the TX15 stand-in
the other end of that UART, so the ground station only gets what comes down the line.

## A real TX15 on its USB cable

The radio can be the other end of the line too, on its USB cable in serial mode (not as a USB
game controller). The radio's USB serial port (`VCP`) has to be in the `LUA` mode, because only
a Lua script can write to it; `CLI` (the default) answers every byte with `Invalid command`. The script
[the T1SFC widget](../teleoperation/edgetx/WIDGETS/T1SFC/main.lua) does the radio's part (the TX15 is a colour radio, which has no telemetry-script screens, so it is a widget; a widget's background function runs whichever view is up):

- it sends the model's 16 channels as CRSF RC channels, as often as the radio runs it;
- it reads the telemetry that comes down (attitude, battery, speed, climb, link quality and the
  flight mode, with `*` after it when the plane is disarmed), and shows it on a telemetry screen;
- it carries MAVLink for the ground station on its cable: what ArduPilot sends goes up again
  addressed to the radio (CRSF address `0xEA`), and what the ground station sends comes down
  addressed to the radio and goes up addressed to the flight controller (`0xC8`).

Set it up once: copy the folder `WIDGETS/T1SFC` to the SD card, set Hardware, USB Serial (VCP) to
LUA, and put the widget on a main view of the model (Model, Screen setup, a zone, `T1SFC`). When you
plug the radio in, pick VCP.
Then plug the radio in, and start the station with its device:

```bash
cd domains/teleoperation/console && cargo run --release -- --station --tx15-serial /dev/cu.usbmodemXXXX
```

The sim opens the device (CRSF's 420 000 baud, raw), and says every two seconds how the line is
doing (`flight computer: link up (… bytes heard, … sent …, quality …)`). The console and the
sim's ground station port get their MAVLink through the radio, and nothing from ArduPilot directly.
The test `a_tx15_on_a_serial_device_flies_the_plane_and_carries_the_ground_station` runs the same
line on a pseudo-terminal, with a radio that does what the script does.

