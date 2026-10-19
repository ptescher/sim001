# Cognition

AI control of planes, from the sim to the real thing: see [plans/sim2real-tag.md](../../plans/sim2real-tag.md).

| Crate | Path | Role |
| --- | --- | --- |
| `cognition_executor` | [executor/](executor) | What flies a plan on board: `no_std`, with a C ABI, linked into the ArduPlane fork's AGENT mode and used by the training environment, so both run the same code |
| `cognition_runner` | [runner/](runner) | A game of tag between planes flown by real ArduPlane SITL instances, with no window and faster than real time: `cargo run --release -p cognition_runner -- --speedup 10` (needs `ardupilot/build_native.sh`) |

## The headless game

The runner is the host for three SITL instances (the T1-S VTOL, instances 1 to 3, as in the sim's games). It answers their servo packets with the flight model, flat ground and no wind, and the computer pilots of `recreation_games` fly them through radio channels: arm, climb in QHOVER, FBWA at 15 m, then tag. The game starts after 70 s of the planes' own time, which ArduPilot needs before it arms (`--warmup`).

At `--speedup 10` a two-minute game takes 12 s of wall time: 9.5 times real time, the speed SITL was asked to run at. Higher speedups have not been tried. The scripted pilots, flying through FBWA, do not catch each other yet: every hand-over in the first runs was a plane leaving the arena, not a tag. That is the baseline to beat.

Player 0 (you, in the sim) is not playing, so there are three planes, not four.
