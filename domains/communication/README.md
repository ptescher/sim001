# Communication

How the pilot's station finds out where the other aircraft are. The station has one source of
telemetry: the radio (the TX15). It has no radio of its own to hear other aircraft, and it
does not read the sim. Other aircraft reach it in the ways they would in the field:

- **Remote ID** (ASTM F3411, OpenDroneID). Every unmanned aircraft near the field broadcasts
  who it is and where it is on Bluetooth or Wi-Fi. A receiver on the pilot's plane hears the
  broadcasts that are in range. It tells the autopilot with the `OPEN_DRONE_ID_*` MAVLink
  messages, and `id_or_mac` set to the address of the sender. The autopilot's own Remote ID
  transmitter uses the same messages with no address, and those are not traffic.
- **A MAVLink network.** Pilots who share a telemetry network see each other's `HEARTBEAT` and
  `GLOBAL_POSITION_INT`. The plane routes them on to the station, with the system ID of the sender.
- **ADS-B.** A receiver on the plane reports crewed aircraft as `ADSB_VEHICLE`.

Everything goes down the radio link with the plane's own telemetry, and the plane's own
telemetry goes first. The station decodes all of it into reports, and the tracker joins the
reports about one aircraft into one marker. See the "Other aircraft" section of the manual
(`expectations/04-communication`).

The domain does not depend on the sim. The sim, or a real radio, supplies the bytes. In the
sim, the only thing it supplies is where the other aircraft are; the radios between them are
simulated here.

## Modules

| Crate | Path | Role |
| --- | --- | --- |
| `communication_remoteid` | [remoteid/](remoteid) | The 25-byte Remote ID messages and message packs, encoded and decoded. No dependencies |
| `communication_traffic` | [traffic/](traffic) | The tracker: reports from any source in, one track for each aircraft out, with stale and dropped tracks. No dependencies |
| `communication_telemetry` | [telemetry/](telemetry) | MAVLink about other aircraft. What a device sends (`Beacon` to `HEARTBEAT`, `GLOBAL_POSITION_INT`, `ADSB_VEHICLE`, Remote ID), what a receiver forwards, and the `Decoder` that turns a stream into reports |
| `communication_air` | [air/](air) | The air between aircraft: emitters, listeners, the range and the losses of each radio, and the rates they send at |
| `communication_radio` | [radio/](radio) | The narrow telemetry link of the TX15 (`Downlink`), and the `Rig`, the whole chain from another plane to a marker. The acceptance test runs here |

Each module depends only on the ones above it in the table.

## In the simulator station

`gcs --station` starts the sim with `--radio 5750 --home LAT,LON,ALT` and connects the console to
port 5750, not to ArduPilot. The sim then stands in for the TX15 (`domains/simulation/sim/src/radio.rs`):

- It connects to ArduPilot at `127.0.0.1:5760`, and relays between that and the console
  (`communication_radio::serve`). What the console sends goes up as it is. What ArduPilot sends
  goes through the `Relay`: replies and missions always arrive, and the streamed telemetry goes
  at the rate of the link (`--radio-rate`, 10 000 bytes a second by default).
- In a game, each computer pilot transmits on its own radios. Kestrel sends Remote ID and is on
  the MAVLink network, so its marker joins the two. Merlin sends Remote ID only. Swift is on the
  MAVLink network only. The receivers on your plane hear them by distance (`Air`).
- The console (`domains/teleoperation/console/src/link.rs`, `Vehicle::hear_traffic`) ignores every message from another
  system for the vehicle's state, hands the traffic to the tracker, and draws a marker for each
  track on the tactical map. `--no-radio` connects the console to ArduPilot directly.

## How a report gets to the map

```text
other plane ──air──▶ receivers on your plane ──▶ Downlink (TX15) ──▶ Decoder ──▶ Tracker ──▶ markers
  Beacon             MAVLink network, Remote ID     your own frames first        reports       one per aircraft
```

1. A device transmits on its radios: MAVLink frames on the network (a heartbeat every second
   and a position twice a second), Remote ID on Bluetooth or Wi-Fi (a position every second,
   the ID every three seconds).
2. `Air` says which receivers hear each transmission. Close in, a transmission is heard
   nearly always. Past half the range, the chance falls to zero at the range.
3. The plane passes what its receivers hear to its link. The link carries a fixed number of
   bytes a second. A newer frame about the same thing takes the place of an older one that is
   still waiting, and a frame that waits more than 3 s is dropped. The plane's own telemetry
   is first in line.
4. The `Decoder` reads the bytes and makes reports. The tracker joins a report to a track by
   name. A report with a new name joins a track from another source that is at the same place
   (60 m, 40 m high, moving at about the same velocity). If two tracks are nearly as near, it
   keeps the report apart, because two markers are better than a swapped one.
5. A track goes stale after 5 s without a report, and goes away after 60 s.

## What is assumed

These are numbers to tune, not measurements.

| Assumption | Where |
| --- | --- |
| Range: MAVLink network 5 km, Bluetooth 4 300 m, Bluetooth 5 Long Range 1.2 km, Wi-Fi NAN 800 m, ADS-B 40 km | `Technology::range` |
| The telemetry link carries 2 600 bytes a second | `Downlink::DEFAULT_RATE` |
| Remote ID positions go out once a second, MAVLink positions twice a second | `communication_air` |

The Remote ID byte layout and scaling follow `opendroneid.h` and `opendroneid.c` of the
`opendroneid-core-c` library. The unit tests check the layout against that code by hand
(the numbers in them come from reading it). They are not the standard's own test vectors.

## Not done yet

- **The whole stack has not been run.** The unit tests, the acceptance tests and a TCP test of
  the relay pass. `--station` with SITL has not been run with the relay: it needs the ArduPlane
  Docker image. Run `cargo run --release -- --station`, press G for a race, and look for the
  Kestrel, Merlin and Swift markers on the tactical map.
- **The real radio.** A TX15 with ExpressLRS carries MAVLink inside CRSF frames. This domain
  starts from MAVLink bytes. Getting them out of the radio (the USB serial port of the radio in
  telemetry mode, or the Wi-Fi backpack) is a separate piece of work, and the ELRS link rate
  may be well under the rate assumed here.
- **A real Remote ID receiver.** Someone has to put a receiver (an ESP32 or a module such as
  the BlueMark) on the plane and feed the autopilot over MAVLink. ArduPilot sends Remote ID
  for its own plane. It does not receive.
- **Two radios on one plane.** An aircraft with Bluetooth and Wi-Fi Remote ID sends from two
  addresses. The tracker joins them only if they are at the same place and the sources
  differ, so they show as two markers at the same place.
- **Splitting.** Once the tracker has joined two names it never splits them.
