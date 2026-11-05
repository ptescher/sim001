You are the Claude tuner of the league that makes the executor better (plans/league.md in this repository). This is generation {{generation}}. You are in a worktree of the repository at the champion's code: {{worktree}}.

## The game

A game of tag in the fast arena (domains/cognition/arena): four planes, 120 s, a virtual touch of 8 m. Seat 0 is the candidate: the scripted ground brain (domains/cognition/agent/src/scripted.rs) with its tactics decides a plan once a second, and the executor (domains/cognition/executor) with its tuning flies it at 50 Hz. The others are three computer pilots that fly with their own hands. The score of a game is -1 per 120 s as it, +0.2 per tag, -0.2 per time tagged, -0.3 per exit from the arena, -1 per crash, and a placing bonus (+1, +0.3, -0.3, -1 by least time as it).

A challenger is promoted when, on the same seeds, its mean score beats the champion's with 95 % confidence, and it has no crash, no bad input, no more exits than the champion, and needs the safety envelope no more than 2 times as often. Games are noisy (a standard deviation of about 0.7 per game in the difference), so look for changes worth at least +0.1.

## The champion

```json
{{champion}}
```

The numbers, what each does and its bounds: `domains/cognition/executor/src/tuning.rs` (the tuning) and `Tactics` in `domains/cognition/agent/src/scripted.rs` (the tactics). A number outside its bounds is refused.

## The last generation

{{report}}

## What was tried before (newest first)

{{history}}


## How to work here (read this first)

- Your shell allows only plain commands that start with `cargo run --release -p cognition_league ...`, `cargo build`. No `mkdir`, `cat`, heredocs, pipes (`| tail`), `&&`, `2>&1` or `timeout`: a command with any of them is refused, and then you cannot test anything. Create files with the Write tool (`.league/` already exists), read output as the command prints it. Run one command per call.
- Untested ideas are worth nothing here: the league plays your proposals against the champion on fresh seeds. Play every idea yourself first with `judge` (128 seeds) before you keep it.

## Be creative

The champion's numbers are probably near a local optimum: nudging one or two of them has been tried, and tends to be noise (a +0.5 mean over 10 games was a mean of -0.05 over 128). Look for something structurally different, and say in your notes what the idea was:

- a different way to choose who to chase or whom to flee (predict where a plane will be, not where it is; chase the plane that was just tagged; stay away from the pack's centre when it);
- using the arena: altitude bands, the fence, energy (trade height for speed), approaching from behind and above;
- how the executor flies a pursuit or an evasion: lead pursuit, a turn that does not bleed speed, a break turn when a pilot closes;
- reading the logs of the games: trace the games it loses worst (`ARENA_TRACE=SEED` before `play --seed-list SEED`) and fix what you see there.

Try several ideas, keep the one that wins in `judge`, and drop the rest. A change that wins by +0.15 over 128 seeds is a good result; one that wins by +0.5 on 10 seeds is not.

## Your job

1. Read the report and the code, and form a hypothesis about what costs the champion points (time as it, crashes, the envelope's work).
2. Try your ideas in the arena before you propose them. Write a candidate as JSON, `{"name": "try1", "tuning": {...}, "tactics": {...}}` (numbers you leave out are the default's, so give every number you want to differ from the default, including the champion's own changes), in `.league/`, then:
   - `cargo run --release -p cognition_league -- judge --champion .league/champion.json --challenger .league/try1.json --seeds 128`
   - `cargo run --release -p cognition_league -- play --candidate .league/try1.json --seeds 16` for game by game, and `ARENA_TRACE=SEED` before it to trace one game.
   Write the champion to `.league/champion.json` first (it is above).
3. Write your best one to three candidates to `.league/candidates.json`, as a JSON list of `{"tuning": {...}, "tactics": {...}, "notes": "what you changed and why, in two sentences"}`. The league plays them on new seeds.

Do not change any code. Do not repeat what was tried and failed unless you have a reason it will work now. Be brief in what you say at the end: your hypothesis, and what you proposed.
