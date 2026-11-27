# Plan: tools for Towers games: records, batches, replays, videos, analysis

To work on the advisors (`plans/advisors.md`) and on predicting the enemy, we need to see games:
play many, keep what happened in each, watch one back, show it to someone, and read off what went
right and wrong. Today each tool has a part of that (the arena's seeds and margins, the server's
flight recorder, a replay page for the old rules, the team map as SVG, the 3D view's stills).
This plan puts one **game record** under all of them.

## What the owner decided (2026-11-26)

- The game tells each team its sightings: when one of its drones or sites spots an enemy drone,
  and when an enemy spots one of its drones. The agents learn it from the game's output, not by
  guessing. (The record carries both from the start; the notice in a team's map is a later step.)
- Tools first: many games, replays, videos with overlays, post-game analysis. Opponent modelling
  is built on them later.

## The game record

One file a game, JSON lines (`recreation_towers::record`), the same from the arena and from the
game server:

| Line | When | What |
| --- | --- | --- |
| `Header` | first | the rules, the seed, who plays each side (commander, advisors), the towers (name, kind, where), the field's radius, the bases |
| `Tick` | every 0.5 s of game time | the referee's truth: every drone (where, height, phase, its order, low or high, in contact, battery), every tower (control, holder), the score |
| `Orders` | each decision | the orders each team gave |
| `Spotted` / `Lost` | when it changes | a team starts or stops seeing an enemy drone (so both "we spotted their drone 3" and "they spotted our drone 2") |
| `Taken` | when it changes | a tower changes holder |
| `Down` / `Up` | when it changes | a drone goes down, comes back |
| `Board` | when it arrives | an entry on a team's board (proposal, intent; later assessments and critiques), and when the commander takes a proposal |
| `End` | last | the outcome, the score, the length |

The record is what every other tool reads, so none of them needs the game's code to run.

## Steps

1. **The record, from the arena.** The types, a `Recorder` that turns a game's state into lines
   (it notices the changes itself), and the arena's match writing one when asked. A recorded game
   plays the same as an unrecorded one.
2. **The batch runner.** `towers_games`: N games on seeds from S, a roster for each side (the
   commander alone, with the lookahead, with the deep thinker), in parallel, a record a game and a
   summary table (the outcome, the score, the towers held, the drones down, the advice taken).
3. **The replay page.** One self-contained page a game, made from its record: the map, the drones
   with their trails and orders, the towers with their control, lines for who sees whom, each
   team's fog on or off, the score over time, a timeline to scrub and play, and each team's board
   beside it (what each advisor said, what the commander took).
4. **Videos with overlays.** Overhead frames drawn from the record (as the team map is drawn, with
   the same overlays: names, orders, sighting lines, score, the board's last entries), rasterised
   with `resvg`, made into an MP4 with `ffmpeg`. Later: the 3D view playing a record and capturing
   frames.
5. **Post-game analysis.** A report a game and a batch: score and tower control over time; how
   long each drone was seen and who saw whom first; the proposals made and taken, what each was
   said to gain and what the team's score did over its horizon (the advisors' track record);
   how the drones followed the intents; the drones down.
6. **The record from the game server.** `towers_server --record` writes the same file beside its
   tracks and ArduPilot logs, so SITL games replay, render and analyse the same way.
7. **Sightings in the team's map.** The game tells each team, in its map, which enemy drones it
   sees and which of its drones the enemy sees (the owner's rule above), as the record already
   does.

## Done (2026-11-27)

All seven steps, in the first version:

| Step | Where | Use |
| --- | --- | --- |
| 1. The record | `recreation_towers::record`; the arena's `play_recorded` | a recorded game plays as an unrecorded one |
| 2. The batch runner | `cargo run --release -p cognition_towers --bin towers_games -- --games 20 --blue deep --red commander --swap --out DIR` | 40 arena games in under a second, a record each (about 650 kB) and `summary.tsv` |
| 3. The replay page | `--bin towers_replay -- DIR/1.jsonl` | a page that stands alone; `#t=120` opens it at 120 s |
| 4. Videos | `--bin towers_video -- DIR/1.jsonl [--view blue] [--speed 10]` | an MP4 with overlays; a 250 s game renders in about 30 s |
| 5. The report | `--bin towers_report -- DIR [--out report.md]` | by side, by game, by advisor |
| 6. SITL | `towers_server --record DIR` writes `DIR/game.jsonl` too | the same tools read it (no board lines yet: the computer commanders' boards are at their seats) |
| 7. Sightings | `OwnDrone::spotted`, `Sites::spotted`; the fog spec | each team is told which of its drones the enemy sees |

First things the tools showed: the Blue seat wins about a fifth of arena games (9 of 40, both
sides the same commander); a team's drones are seen about 35 % of their flying time in the arena
and about 60 % on SITL.

## Not settled

- The 3D replay (step 4, later): `towers_view` playing a record and capturing frames.
- The boards of a SITL game: the computer commanders run at their seats; the server needs them
  (or the seats write their own part of the record).
- What a proposal was worth: the report's "lead's move" says how things went over a taken
  proposal's horizon, not what it gained against not taking it. The arena could replay the game
  from that moment without it (a counterfactual), once a match can be snapshotted.
- How long a batch's records are kept (a game's record is a few hundred kB).
