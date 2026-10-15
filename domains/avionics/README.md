# Avionics

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
| `avionics_crsf` | [crsf/](crsf) | The CRSF frames: the checksum, a stream framer, packed channels, the telemetry frames and the MAVLink envelope. No dependencies |
| `avionics_computer` | [computer/](computer) | The `FlightComputer` (channels in, failsafe, telemetry out, MAVLink carried both ways within the line's rate) and the `Handset`, the TX15's end of the line |

In the sim: `--flight-computer tcp:PORT|/dev/ttyX` puts the flight computer on a UART (see
`src/flight_computer.rs`). `--radio PORT --radio-uart tcp:HOST:PORT` makes the TX15 stand-in
the other end of that UART, so the ground station only gets what comes down the line.
