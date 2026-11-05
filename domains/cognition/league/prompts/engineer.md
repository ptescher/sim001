You are the Claude engineer of the league that makes the executor better (plans/league.md in this repository). This is generation {{generation}}. You are in a git worktree of the repository at the champion's code: {{worktree}}. Change the code here; the league commits what you leave.

## The game

A game of tag in the fast arena (domains/cognition/arena): four planes, 120 s, a virtual touch of 8 m. Seat 0 is the candidate: the scripted ground brain (domains/cognition/agent/src/scripted.rs) decides a plan once a second, and the executor (domains/cognition/executor) flies it at 50 Hz. The others are three computer pilots that fly with their own hands, the same whatever your code. The score of a game is -1 per 120 s as it, +0.2 per tag, -0.2 per time tagged, -0.3 per exit from the arena, -1 per crash, and a placing bonus (+1, +0.3, -0.3, -1 by least time as it).

A challenger is promoted when, on the same seeds, its mean score beats the champion's with 95 % confidence, and it has no crash, no bad input, no more exits than the champion, and needs the safety envelope no more than 2 times as often. Games are noisy (a standard deviation of about 0.7 per game in the difference), so look for changes worth at least +0.1.

## What you may change

Only these files. A change to any other file is refused; a change to the safety envelope (`executor/src/lib.rs`, `math.rs`, the plan crate) is refused before anything is played.

{{may_change}}

The executor runs on a flight controller: keep it `no_std`, with no allocation and no panics. The executor's and the agent's tests must still pass (`cargo test --release -p cognition_executor -p cognition_agent`); a change that fails them is refused. Add a test for a new behaviour.

## The champion

```json
{{champion}}
```

## The last generation

{{report}}

## What was tried before (newest first)

{{history}}


## How to work here (read this first)

- Your shell allows only plain commands that start with `cargo run --release -p cognition_league ...`, `cargo build`, `cargo test`, `git diff`, `git status`. No `mkdir`, `cat`, heredocs, pipes (`| tail`), `&&`, `2>&1` or `timeout`: a command with any of them is refused, and then you cannot test anything. Create files with the Write tool (`.league/` already exists), read output as the command prints it. Run one command per call.
- Untested ideas are worth nothing here: the league plays your proposals against the champion on fresh seeds. Play every idea yourself first with `judge` (128 seeds) before you keep it.

## Be creative

The champion's numbers are probably near a local optimum: nudging one or two of them has been tried, and tends to be noise (a +0.5 mean over 10 games was a mean of -0.05 over 128). Look for something structurally different, and say in your notes what the idea was:

- a different way to choose who to chase or whom to flee (predict where a plane will be, not where it is; chase the plane that was just tagged; stay away from the pack's centre when it);
- using the arena: altitude bands, the fence, energy (trade height for speed), approaching from behind and above;
- how the executor flies a pursuit or an evasion: lead pursuit, a turn that does not bleed speed, a break turn when a pilot closes;
- reading the logs of the games: trace the games it loses worst (`ARENA_TRACE=SEED` before `play --seed-list SEED`) and fix what you see there.

Try several ideas, keep the one that wins in `judge`, and drop the rest. A change that wins by +0.15 over 128 seeds is a good result; one that wins by +0.5 on 10 seeds is not.

## Your job

1. Read the report and the code. Form a hypothesis about what costs the champion points: how the primitives fly, how the brain chooses plans, why it crashes or leans on the envelope.
2. Write the champion to `.league/champion.json` (it is above). Before you change anything, note its score: `cargo run --release -p cognition_league -- play --candidate .league/champion.json --seeds 128` (the last line). `ARENA_TRACE=SEED` before a `play --seed-list SEED` traces one game.
3. Make one focused change, and play the same command again. Keep the change only if the mean score went up (by more than about 0.1) with no crash.
4. Run the tests.
5. If you also want new numbers with your code, write `.league/candidates.json`: `{"tuning": {...}, "tactics": {...}, "notes": "..."}`. Always write `notes` there, even with no numbers: what you changed and why, in two sentences.

If you find that the champion's trouble is in the envelope, do not change it: say so in your notes, for a person to read. Be brief in what you say at the end.
