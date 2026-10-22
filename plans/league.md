# Plan: a league that keeps making the executor better

A loop that runs unattended. Each generation, several **strategies** (an evolution strategy and
Claude agents, later trained networks) each propose **challengers** to the executor's
**champion**. Every challenger flies the same seeded games in a **fast arena**. A challenger
that clearly beats the champion, and breaks none of the safety gates, becomes the new
champion. The champion is then checked in SITL. The loop goes on from there.

It sits on top of [sim2real-tag.md](sim2real-tag.md). The arena is the start of the F0 level
there, and the league is a first form of its "league of snapshots", before any RL.

## What a model is

A **candidate** is everything that decides how a plane plays:

| Part | What it is | Who changes it |
| --- | --- | --- |
| `Tuning` | The executor's guidance and energy numbers: L1, the circle field, the height and speed controllers, pursue's lead and closing, evade's height and speed (`cognition_executor::Tuning`, 37 numbers, each with its bounds) | The evolution strategy, Claude agents |
| `Tactics` | The ground brain's numbers: awareness, target stickiness, lead, edge, height bands, plan ttl | The evolution strategy, Claude agents |
| Code | A patch to `cognition_executor` or `cognition_agent` (a git commit on a branch) | Claude agents only, and only if the executor's tests still pass |

**The safety envelope is not in `Tuning`.** Separation, the fence, the floor, the ceiling,
the stall margin and the attitude limits keep their constants, so nothing the league does can
trade safety for points. A code patch that touches the envelope is refused before it is
played (`league` checks the diff).

`Tuning::DEFAULT` is exactly today's constants, so the executor flies as it did before. A
tuning has a hash (FNV-1a over its bits), which the reports quote. Later the plane will
report it too, when the C ABI loads a tuning from the SD card.

## The arena (F0)

`cognition_arena`: a game of tag with four planes, no window, many games at once (rayon).

- **Seat 0 is the candidate.** Its brain (`ScriptedBrain` with the candidate's tactics) gives
  a plan once a second. The executor (with the candidate's tuning) flies it at 50 Hz. Its
  demand goes through **ap-lite**, a small model of ArduPlane's roll and pitch controllers
  and its throttle, into `simulation_physics`.
- **Seats 1 to 3 are the computer pilots** (Kestrel, Merlin, Swift), from
  `recreation_games::Session`. They fly with their own stick logic, so they are the same
  opponents for every candidate, whatever code the candidate brings. This is what lets a code
  patch be compared fairly with the champion.
- **The referee** is `recreation_tag` with a **virtual touch** of 8 m. At the sim's 1.5 m
  touch, an executor that keeps 4 m of separation can never tag anyone. At the plan's 15 m
  for real planes (without its height bands), three planes in this 90 m arena pass "it" round a
  cluster hundreds of times a game; from 10 m down they do not.
- **Tracks** come from the truth, made late and lossy (a delay and a drop rate per game). The
  `communication_air` tracker comes later.
- **Randomised per seed:** wind (0–6 m/s, any direction), who is it first, the start time
  of each opponent, track delay (0.2–1.5 s) and loss (0–30 %). The same seed gives the same
  game, so a challenger and the champion meet the same games.

**Score of one game for seat 0** (the reward table in sim2real-tag.md, without shaping): −1
per 120 s as it, +0.2 per tag, −0.2 per time tagged, −0.3 per exit from the arena, −1 per crash,
plus a placing bonus (+1, +0.3, −0.3, −1 by least time as it). Also recorded: envelope
interventions per second of flight, which rules fired, fallbacks, the closest approach
while it.

## The league

`cognition_league`: the state of the loop, in `league/` (git-ignored except the champion).

```
league/
  champion.json        the champion: tuning, tactics, commit, hash, its arena results
  history.jsonl        every challenger: strategy, parent, diff, results, verdict
  generations/NNN.md   a readable report per generation
  worktrees/           Claude agents' checkouts (removed after each generation)
```

**Promotion.** A challenger plays the same N seeds (256 by default) as the champion. The
difference in score between two executors has a standard deviation of about 0.7 a game, so
256 games tell about 0.1 from noise; 64 could not tell the default executor from the slow one
reliably. It is
promoted when the mean of the paired differences in score is positive and its 95 % lower
bound is above 0, **and** every gate holds:

| Gate | Limit |
| --- | --- |
| Crashes | 0 across all seeds (a crash in the hand's climb-out, before the executor flies, is the arena's, and is not counted; nor is an exit then) |
| Arena exits | No more than the champion's, plus two standard deviations of chance (each exit already costs score; this catches an executor that leaves clearly more often) |
| Envelope interventions | No more than 2× the champion's rate (a better chaser follows its target to the fence more often: the default executor needs 1.6× the slow one's) |
| Bad input or NaN demands | 0 |

When several challengers in a generation qualify, the best by mean difference wins. The
others are recorded with their results, so strategies learn from near misses too.

**SITL check.** Every promoted champion can be flown in `cognition_runner` (F1). That needs the
C ABI to load a tuning (a follow-up), so for now the report lists it as not checked.

## The strategies

A strategy takes the league's state (the champion, the history, the last report) and gives
challengers. Each one runs on its own. They compete with each other as well: the report
counts how many promotions each strategy has won, and the loop gives more slots to the ones
that win.

| Strategy | What it does | Cost |
| --- | --- | --- |
| `es` | A (μ/μ, λ) evolution strategy (λ = 8, μ = 2) in the bounds-scaled space of tuning and tactics. It keeps a centre of its own and moves it to its best challengers every generation, promoted or not, so small gains add up until one is clear enough to promote; its step grows when its best beat the champion and shrinks when not | About 35 s of CPU a generation on 16 cores |
| `claude-tuner` | `claude -p` with the champion, the last report (scores by seed, which rules fired, near misses) and what was tried before. It writes `candidate.json` (tuning and tactics) and its reasons | One headless Claude run |
| `claude-engineer` | `claude -p` in a git worktree off the champion's commit. It may change the executor's primitives or the brain (not the envelope). The league runs `cargo test -p cognition_executor -p cognition_agent` there, then builds the arena there and plays it | One Claude run plus a build |

Claude runs headless (`claude -p --output-format json`), with its tools limited to reading,
editing inside its worktree, and `cargo`. It has a turn budget and a wall-clock budget. The
prompt is in `domains/cognition/league/prompts/`. What it says it did is not trusted: the
league checks the diff and the tests, and the arena decides.

## The loop

```
cargo run --release -p cognition_league -- run --generations 20 --strategies es,claude-tuner,claude-engineer
```

For each generation:

1. Load the champion, or make one from `Tuning::DEFAULT` and the scripted tactics.
2. Ask each strategy for challengers, in parallel.
3. Build what needs building, and refuse what fails tests or touches the envelope.
4. Play every challenger on the generation's seeds. The champion plays them too.
5. Promote, or not. Write `champion.json`, a line per challenger in `history.jsonl`, and
   `generations/NNN.md`.
6. A promoted code patch is committed on the `league/champion` branch. It is never merged
   into master by the loop: a person merges it.

It can run overnight from cron, or from `/loop` in a Claude Code session.

## Findings so far

- **Accelerated stalls.** The default executor crashed in about 1 game in 200 before the
  arena threw into the wind: pursuing a target above it, it asks for the full bank (50°) and
  the full pitch (25°) at 17 m/s; the wing tips stall in the turn, the plane rolls over, and
  the floor rule's recovery (full pitch up again) keeps the wing stalled to the ground. The
  envelope's stall rule looks only at the airspeed against the 1 g stall speed, not at the load
  of a turn. That is a change to the envelope, for a person to make (the league may not); in
  the meantime the crash gate pushes the league away from tunings that pull this hard.
- **The pursuit cannot tag by design.** Pursue sits `separation + buffer + standoff_extra` =
  10 m behind its target, and the envelope's separation zone is 7 m, so with an 8 m touch the
  executor tags mostly by chance. Something for the league to find (`standoff_extra`, the
  tactics), or for an engineer.
- **The hand launch.** Thrown north whatever the wind, the hand's climb-out crashed in some
  winds; now it throws into the wind, and a crash or an exit before the executor flies is not
  the candidate's.

## Order of work

Each step starts with its expectations (`expectations/07-cognition/05-league.expect`).

1. Done: `Tuning` in the executor, a pure refactor (bit-identical demands); the primitives in
   `primitives.rs`, the tuning in `tuning.rs`, the envelope's numbers in `safety`.
2. Done: `cognition_arena`: ap-lite, a seeded game, the score, many games at once.
3. Done: `cognition_league`: candidates, paired comparison, gates, promotion, persistence, reports.
4. Done: the `es` strategy, and the loop end to end with it alone.
5. Done, but not yet run with a real `claude`: the Claude strategies (the prompts, `claude -p`,
   worktrees, the test gate); the tests use a stand-in.
6. Follow-ups: the C ABI to load a tuning (ArduPilot patch 0003, the hash in `AG_*` status),
   the SITL check, tracks through `communication_air`, MLP policies as a third kind of
   candidate (the training stack in sim2real-tag.md, Phase 3), head-to-head games between
   candidates with the same code.

## A second contest: captains for Towers

The league is generic over a `Contest` (`domains/cognition/league/src/contest.rs`): what is
tuned (groups of numbers with bounds), how a candidate plays, which files a Claude engineer may
change, which ones refuse it, the tests a change must pass, and the prompts. The executor at tag
is one; a captain at Towers (`recreation_towers::Commander`, with its `CommanderTuning`) is the
second. Another game is a third `Contest`.

| | Executor | Captain |
| --- | --- | --- |
| Numbers | `tuning` (37), `tactics` (7) | `captain` (`CommanderTuning`: give up, stay away, defend range, behind by) |
| Arena | `cognition_arena`: seat 0 against the computer pilots, ~7 000x real time | `cognition_towers`: drones fly their orders on their own (take-off, a bank-limited turn, the server's 40 m orbits), the real referee and fog, ~200 000x real time |
| Opponent | None of its own (the computer pilots) | The champion: each seed played twice, as Blue and as Red, so a side's advantage cancels; the champion against itself scores exactly 0 |
| Score | The plan's reward for tag | +1 a win, -1 a loss, 0 a draw, plus the point margin over 500, averaged over the two sides |
| Gates | Crashes, bad input, exits, envelope | None beyond the score (no crashes in this arena) |
| Engineer may change | `primitives.rs`, `tuning.rs`, `scripted.rs` | `commander.rs` |
| Refused at once | The envelope (`lib.rs`, `math.rs`, the plan crate) | The rules, referee and fog (`recreation_towers`), the server, the arena |
| Tests | `cognition_executor`, `cognition_agent` | `recreation_towers` and `cognition_towers`, but not the commander's own (they pin today's captain; a promoted captain's spec, `09-towers/04-commander.expect`, is a person's to update) |
| Folder | `league/` | `league/captain/` |

**Code against code.** In tag the opponents do not depend on the candidate's code, so one
build plays it. In Towers the opponent is the champion: built together, a patched captain would
play itself. So a captain with code of its own plays as a process of its own, built from its
worktree, on the seat's JSON lines (`cognition_league captain-seat`, `cognition_towers::serve`),
and the champion with its own code.

```bash
cargo run --release -p cognition_league -- judge --game captain --champion timid --challenger default
cargo run --release -p cognition_league -- run --game captain --generations 5 --strategies es,claude-tuner,claude-engineer
```

The Claude strategies run `claude` without the variables of the session they were started from
(`ANTHROPIC_BASE_URL`, `CLAUDE*`): under the desktop app those point at its own session, and
`claude` is refused (401). It needs a working `claude` login (`claude auth login`).
