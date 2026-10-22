# Cognition

AI control of planes, from the sim to the real thing: see [plans/sim2real-tag.md](../../plans/sim2real-tag.md). The league that keeps making the executor better: [plans/league.md](../../plans/league.md).

| Crate | Path | Role |
| --- | --- | --- |
| `cognition_plan` | [plan/](plan) | The plan the ground sends a plane, its MAVLink encoding, and the C types the executor takes |
| `cognition_executor` | [executor/](executor) | What flies a plan on board: `no_std`, with a C ABI, linked into the ArduPlane fork's AGENT mode and used by the arena, so both run the same code. Its guidance and energy numbers are a `Tuning` (`src/tuning.rs`); the safety envelope's are fixed (`safety` in `src/lib.rs`) |
| `cognition_agent` | [agent/](agent) | The ground control agent: the scripted brain (with `Tactics`), and the MAVLink link to each plane |
| `cognition_arena` | [arena/](arena) | The fast arena (F0): a game of tag with no window and no SITL, the executor against the three computer pilots, about 7 000 times faster than real time on all cores |
| `cognition_towers` | [towers/](towers) | The fast Towers arena: two captains (in this process, or each in a process of its own) play a seeded game of Towers with drones that fly their orders on their own, about 200 000 times faster than real time |
| `cognition_league` | [league/](league) | The league: strategies (an evolution strategy, Claude agents) give challengers, the arena plays them on the champion's games, and a challenger that is clearly better and safe becomes the champion. Two contests: the executor at tag (`--game executor`) and a captain at Towers (`--game captain`) |
| `cognition_runner` | [runner/](runner) | A game of tag between planes flown by real ArduPlane SITL instances (F1), with no window and faster than real time: `cargo run --release -p cognition_runner -- --speedup 10` (needs `ardupilot/build_native.sh`) |

## The league

```bash
cargo run --release -p cognition_league -- play --seeds 64                     # the default executor's games
cargo run --release -p cognition_league -- judge --champion slow --challenger default
cargo run --release -p cognition_league -- run --generations 10 --strategies es,claude-tuner,claude-engineer
cargo run --release -p cognition_league -- status
cargo run --release -p cognition_league -- judge --game captain --champion timid --challenger default
cargo run --release -p cognition_league -- run --game captain --generations 5 --strategies es,claude-tuner,claude-engineer
```

The league keeps its state in `league/` at the top of the repository: the champion, every challenger (`history.jsonl`) and a report per generation (`generations/NNN.md`). The Claude strategies run `claude -p` headless, each in a git worktree at the champion's code, with a budget of $5 a run (`SIM001_CLAUDE` names another command). A Claude engineer may change only `executor/src/primitives.rs`, `executor/src/tuning.rs` and `agent/src/scripted.rs`; the league refuses a change to the envelope, and a change that fails `cargo test -p cognition_executor -p cognition_agent`. A promoted code change goes on the branch `league/champion`, never into `master`; every code challenger's commit is kept as `refs/league/NAME`. The league works from commits: changes not committed in this checkout are not in the Claude strategies' worktrees.

`ARENA_TRACE=SEED cargo run --release -p cognition_league -- play --candidate FILE --seed-list SEED` traces the candidate's flight in one game, ten times a second.

## The headless game in SITL

The runner is the host for three SITL instances (the T1-S VTOL, instances 1 to 3, as in the sim's games). It answers their servo packets with the flight model, flat ground and no wind, and the computer pilots of `recreation_games` fly them through radio channels: arm, climb in QHOVER, FBWA at 15 m, then tag. The game starts after 70 s of the planes' own time, which ArduPilot needs before it arms (`--warmup`).

At `--speedup 10` a two-minute game takes 12 s of wall time: 9.5 times real time, the speed SITL was asked to run at. Higher speedups have not been tried.

Player 0 (you, in the sim) is not playing, so there are three planes, not four.
