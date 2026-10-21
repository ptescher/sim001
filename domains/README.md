# Domains

The repo is one Cargo workspace, organised as **workspace → domain → module**:

```
Cargo.toml                    the workspace (and, for now, the sim package)
domains/
  <domain>/                   a coherent area of the product, with its own README
    <module>/                 one crate: <domain>_<module>, depending only "downwards"
```

A domain is a directory. Each module in it is a crate named `<domain>_<module>`. The
workspace picks them up through the `domains/*/*` glob, so a new module needs no edit to the
root `Cargo.toml`. A module depends on the modules beneath it in its own domain, and on
other domains only through their public crates. Keeping every module a separate crate makes
the compiler enforce that.

| Domain | What it is |
| --- | --- |
| [teleoperation](teleoperation/README.md) | The pilot's end of the link: the TX15's EdgeTX models and themes (laid out as its SD card so EdgeTX Companion can simulate them), and the GPUI ground control station that flies ArduPlane SITL and the sim, with the feed protocol and window layout it shares with the sim. |
| [illustration](illustration/README.md) | The look shared by the ground station, the sim's HUD and the docs: palettes, type and corners. |
| [simulation](simulation/README.md) | The flight model and the planes it flies: aircraft descriptions and the physics, headless and independent of any renderer. |
| [recreation](recreation/README.md) | Games and trials against computer pilots who fly the same plane on the same physics: a pylon race and tag. And Towers, a team game for drones flown by plan, with a fog of war. |
| [visualization](visualization/README.md) | The 3D view (terrain, forest, water, sky, the planes and the cameras) and the place it shows. It draws a scene that the simulator or a real plane's telemetry fills in, and it saves pictures for camera-based systems. |
| [communication](communication/README.md) | How the station finds out where other aircraft are, using only what the radio passes on: Remote ID, a MAVLink network and ADS-B, the radios between aircraft, and the tracker that joins them into one picture. |
| [aviation](aviation/README.md) | The flight computer between the TX15 and ArduPilot: the TX15 on a UART in CRSF instead of a USB controller, channels in, failsafe, telemetry and the ground station's MAVLink carried on the same line. |
| [cognition](cognition/README.md) | AI control of planes, from the sim to the real thing: the plan executor that runs on board, the plan protocol, the training environments and the ground control agent (see [plans/sim2real-tag.md](../plans/sim2real-tag.md)). |

The sim (`src/`) predates the layout and will move into a domain as the work goes on.
`teleoperation/console` and `visualization/mapbuild` are excluded from the workspace for now
because they keep their own lockfiles.

The acceptance tests (`expectations/`, `--expect`) use [expectation](https://github.com/ptescher/expectation), its own repo: behaviour
written as expectations, checked for conflicts, run as acceptance tests from user stories, and turned
into documentation. `cargo run -p expectation -- check expectations` runs its command ([tools/expectation](../tools/expectation)).
