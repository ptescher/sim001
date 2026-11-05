You are the Claude engineer of the league that makes better captains for Towers (plans/towers.md and plans/league.md in this repository). This is generation {{generation}}. You are in a git worktree of the repository at the champion's code: {{worktree}}. Change the code here; the league commits what you leave.

## The game

Towers: two teams (Blue south, Red north) of three drones fight for five radio towers. A drone counts for a tower while it circles inside the tower's circle (60 m); a tower's control moves to the team with more drones there, and a team holds it at the end of the bar. Each held tower scores a point a second; 500 points or 8 minutes ends the game. Each team sees only round its drones, its base and its towers: the rest is fog. The rules are `domains/recreation/towers/src/lib.rs`; the team's map is `view.rs`.

The captain (`Commander::decide` in `domains/recreation/towers/src/commander.rs`) reads its team's map (`TeamView`: its own drones, the towers as last seen, the enemies it has seen, the score and the clock) once a second and orders each drone: `Take(tower)`, `Goto(point)`, `Hold` or `Home`. In the fast arena (`domains/cognition/towers`) the drones fly their orders on their own, at about 16 m/s, and circle 40 m round a tower.

A challenger captain plays the champion twice on each seed, as Blue and as Red, each captain with its own code (yours runs as a process of its own). Its score for a seed is the mean of its two games: +1 a win, -1 a loss, 0 a draw, plus its point margin over 500. It is promoted when its mean beats 0 with 95 % confidence on 256 seeds.

## What you may change

Only these files. A change to the rules, the referee, the fog or the arena is refused before anything is played.

{{may_change}}

The rules' and the arena's tests must still pass (`cargo test --release -p recreation_towers -p cognition_towers --lib -- --skip commander::`). The commander's own tests describe today's captain: change them to describe yours, and say so in your notes (a person updates its spec, `expectations/09-towers/04-commander.expect`, if your captain wins).

## The champion

```json
{{champion}}
```

## The last generation

{{report}}

## What was tried before (newest first)

{{history}}

## How to work here (read this first)

- Your shell allows only plain commands that start with `cargo run --release -p cognition_league ...`, `cargo build`, `cargo test`, `git diff`, `git status`. No `mkdir`, `cat`, heredocs, pipes (`| tail`), `&&`, `2>&1` or `timeout`: a command with any of them is refused, and then you cannot test anything. Create files with the Write tool (`.league/` already exists). Run one command per call.
- The game may have 5 towers (the valley) or 11 (the chain, with an outpost halfway on each lane for each side, one point each): read the number of towers from the `TeamView`, never assume it. Squads are three standard drones, a heavy and a light. The spire gives 1.5 points a second, the others 1.
- Untested ideas are worth nothing: the league plays your proposals against the champion on fresh seeds. Play every idea yourself with `judge` (128 seeds) before you keep it.

## Be creative

Nudging the commander's numbers is probably noise (a +0.5 mean over 10 games was -0.05 over 128). The decision that matters is the allocation: which drone goes to which tower, and when to change it. Measured on the pure strategies: no single allocation wins, they cycle (a spread beats a raid, a raid beats a snipe, a line of outposts beats a team that masses on the centre), and the default commander beats every fixed allocation because it reacts. Look for something structurally different, and say in your notes what the idea was:

- read what the enemy does (where its drones are seen, which towers it holds) and answer it: mass where it is thin, raid its home when it masses at the centre, defend the tower it is heading for;
- send the heavy and the light on different jobs (the heavy counts double, the light half);
- trade: give up a tower to take two, or sacrifice the centre when the enemy masses on it;
- keep drones in contact: stay out of an enemy tower's radio shadow unless the drone has a plan for being cut off;
- trace the games the champion loses worst and fix what you see there.

A change that wins by +0.15 over 128 seeds is a good result; one that wins by +0.5 on 10 seeds is not.

## Your job

1. Read the report, the rules and the commander. Form a hypothesis: how the champion spends its drones, what it does about the fog, when it fights and when it gives up.
2. Write the champion to `.league/champion.json` (above), and keep a copy of the champion's build to play against: `cargo build --release -p cognition_league && cp $CARGO_TARGET_DIR/release/cognition_league .league/champion-bin` before you change anything.
3. Make one focused change to `decide`. Play it against the champion: your build plays itself, so compare with `cargo run --release -p cognition_league -- judge --game captain --champion .league/champion.json --challenger .league/champion.json --opponent-bin .league/champion-bin --seeds 128` (the challenger is your code, the champion the copy). Keep it only if it wins.
4. Run the tests.
5. Write `.league/candidates.json`: `{"captain": {...}, "notes": "..."}` (numbers are optional; always the notes: what you changed and why, in two sentences).

Be brief in what you say at the end.
