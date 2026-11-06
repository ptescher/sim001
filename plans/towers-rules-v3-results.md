# Towers v3: what the arena says

## Step 1: on-board policies against "fly the order on" (headroom, criteria 3 and 4)

`cargo run --release -p cognition_towers --example edge 400`: each policy plays `persist` (the
default commander, every order with the standing order "fly on") on both sides of 400 seeds, 800
games. A policy decides for a drone that is out of contact, once a second, from `EdgeObs` only.
Margin is the policy's mean score over persist's (0 is level).

| Policy | Margin | Won / lost | Drones down, per game |
| --- | --- | --- | --- |
| keep the order (the baseline) | +0.000 ± 0.044 | 400 / 400 | 1.70 |
| home when 1 enemy is near | −0.992 ± 0.036 | 130 / 669 | 0.37 |
| home when 2 enemies are near | −0.428 ± 0.044 | 279 / 521 | 0.73 |
| home when 3 enemies are near | −0.204 ± 0.045 | 340 / 460 | 1.20 |
| finish the capture, then return | +0.000 ± 0.044 | 400 / 400 | 1.70 |

Reading it:

- **No headroom yet.** Every retreat policy loses to persist, and the more cautious it is, the
  more it loses. Pressing on beats leaving, also when the enemy is in sight. Leaving saves
  drones (1.70 down a game to 0.37) and loses the game.
- **"Finish, then return" never runs.** A drone is out of contact only inside the shadow of an
  enemy tower, and a tower that the drone's team holds casts no shadow. By the time its tower is
  held, the drone is in contact again, so the policy has no cut-off drone to decide for.
- **What this says about the rules**, not about the policies: a cut-off drone has one choice that
  matters (press on or leave) and pressing on wins. The on-board information that the v3 plan
  wants to matter does not yet change the right answer. The rules need a cost to pressing on that
  the camera can see (sections 5 and 6 of `plans/towers-rules-v3.md`), before a model has
  anything to learn.
- `EdgeObs` has no friends and no own class: "outnumbered" counts enemies only. A policy that
  compares the enemy's weight with its own side's in the circle needs both.

## Step 1c: allocations of drones to towers (formations)

The decision that matters is the captain's: which drone goes to which tower. A *formation* is a
fixed allocation (a pure strategy): each drone flies to its post and presses on, and goes home to
recharge when its battery is low. `cargo run --release -p cognition_towers --example formations 60`
plays each pair on both sides of 60 seeds (120 games a cell). The cell is the row's mean score
over the column's. Squad: three standard, a heavy, a light.

| Row against column | commander | centre rush | wings | balanced | no centre | raid | turtle | heavy centre | mean |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| commander | - | +0.31 | +1.30 | −0.80 | +1.54 | −0.68 | +1.58 | +0.51 | +0.54 |
| centre rush (all five at the centre) | −0.31 | - | +0.86 | −0.70 | +0.88 | +0.11 | +1.07 | +0.82 | +0.39 |
| wings (2 west, 2 east, 1 centre) | −1.30 | −0.86 | - | −0.99 | −0.20 | −1.40 | +1.56 | −1.29 | −0.64 |
| **balanced** (west, east, 3 at the centre) | +0.80 | +0.70 | +0.99 | - | +0.99 | +1.06 | +1.55 | +0.70 | **+0.97** |
| no centre (both flanks only) | −1.54 | −0.88 | +0.20 | −0.99 | - | −1.42 | +1.56 | −1.29 | −0.62 |
| raid (two at the enemy home) | +0.68 | −0.11 | +1.40 | −1.06 | +1.42 | - | +1.55 | +0.09 | +0.57 |
| turtle (guards its home and the flanks) | −1.58 | −1.07 | −1.56 | −1.55 | −1.56 | −1.55 | - | −1.67 | −1.51 |
| heavy centre | −0.51 | −0.82 | +1.29 | −0.70 | +1.29 | −0.09 | +1.67 | - | +0.30 |

Reading it:

- **There is no cycle: one allocation, balanced, beats every other**, and the more of the squad
  that is at the centre, the better a team does (turtle and no centre are last). The spire is
  worth two points a second, and the team that holds it is ahead for the whole game.
- **"Both sides give up the centre" is not an equilibrium.** Against a team that goes to the
  centre, a team with no centre loses by 0.9 to 1.5. For that trade to be a real choice, the
  centre has to cost something, or pay less than it does now.
- Numbers to try (next): the spire's points (2 a second), its shadow (200 m), a contest drain
  that grows with the enemy weight there; each as a field in `Rules`, then this table again.
  The target is no row that beats the rest, and some cycle between the rows.
- These are pure strategies with no reaction. The default commander reacts (it defends, gives
  up a lost tower) and still loses to balanced.

## Step 1d: making the centre cost something

The same round robin with the spire worth less (`formations 40 spire_score=X`; 80 games a cell).
`beats every other` is a row that wins against each other row by at least 2 standard errors.

| Spire points a second | Best row (mean) | Rows that beat every other | Cycles (a beats b beats c beats a) |
| --- | --- | --- | --- |
| 2 (now) | balanced (+0.97) | **1** (balanced) | none |
| 1.5 | heavy centre (+0.72), raid (+0.69) | 0 | wings > balanced > raid > wings; no centre > balanced > raid > no centre |
| 1.5, contest_drain 3 | commander (+0.68) | 0 | the same two, and heavy centre beats raid |
| 1 | raid (+0.59), commander (+0.58), heavy centre (+0.58) | 0 | wings > balanced > raid > wings; no centre > balanced > raid > no centre |

- **Making the spire worth 1.5 points a second is enough to end the dominance.** Rows beat each
  other in cycles: a team that spreads to both flanks beats one that masses at the centre, the
  mass beats a raid on the enemy home, and the raid beats the spread.
- "centre rush" and "turtle" are last in every column: they are not strategies, only mistakes.
  "no centre" now holds its own, which is the sacrifice of the centre on both sides.
- `Rules::default()` is not changed: the specs and the commander's tuning say two points. Next:
  set `spire_score` 1.5, regenerate the acceptance numbers, and re-tune the commander.

## Step 2: layouts as data, and the chain (11 towers)

`Rules::layout` picks the layout: `Valley` (five towers, as before: the allocation matrix is
identical) or `Chain` (the valley's five, and an *outpost* halfway along each of the three lanes
on each side: Blue west/mid/east, Red west/mid/east). An outpost is a new kind of tower: one
point a second, no recharge, a radio shadow of 50 m (`outpost_jam_radius`). The halfway places
are 215 m from a fort and 215 m from the centre, so an outpost's shadow clears the fort's (160 m)
but overlaps the spire's (200 m) by a ring of 35 m. No smaller shadow fixes that on a 430 m lane,
so the chain does not keep "no two shadows overlap" of the v2 plan.

`cargo run --release -p cognition_towers --example formations 40 layout=1` (80 games a cell):

| Row against column | commander | mass centre | balanced | own line | far line | raid | spread | snipe | no centre | mean |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| commander | - | +1.42 | +1.28 | +1.02 | +0.91 | +0.93 | +1.12 | +1.39 | +1.31 | **+1.17** |
| mass centre (all five) | −1.42 | - | −0.78 | −1.12 | −0.97 | −0.56 | −0.96 | −1.33 | −1.27 | −1.05 |
| balanced (west, east, three at the centre) | −1.28 | +0.78 | - | +0.80 | −0.46 | +0.28 | −0.60 | −0.21 | −1.17 | −0.23 |
| own line (the three own-side outposts, two at the centre) | −1.02 | +1.12 | −0.80 | - | +1.31 | +1.21 | +0.25 | +1.35 | +1.12 | +0.57 |
| far line (the three enemy-side outposts, two at the centre) | −0.91 | +0.97 | +0.46 | −1.31 | - | +0.69 | −0.80 | +1.22 | −0.43 | −0.01 |
| raid (two at the enemy home) | −0.93 | +0.56 | −0.28 | −1.21 | −0.69 | - | −1.44 | +1.01 | −0.04 | −0.38 |
| spread (own west and east outposts, the enemy's mid, centre, west) | −1.12 | +0.96 | +0.60 | −0.25 | +0.80 | +1.44 | - | +1.49 | +1.14 | +0.63 |
| snipe (all five at the enemy-side outposts) | −1.39 | +1.33 | +0.21 | −1.35 | −1.22 | −1.01 | −1.49 | - | −1.08 | −0.75 |
| no centre | −1.31 | +1.27 | +1.17 | −1.12 | +0.43 | +0.04 | −1.14 | +1.08 | - | +0.05 |

Reading it:

- **The fixed allocations cycle, none dominates.** A closed 3-cycle: balanced beats own line
  (+0.80), own line beats far line (+1.31), far line beats balanced (+0.46). Mass centre and
  snipe lose to nearly everything. Spread beats raid (+1.44), snipe (+1.49), no centre (+1.14)
  and far line (+0.80), and loses only to the commander and, narrowly, to own line (−0.25).
- **A reactive commander beats every fixed allocation** (+0.9 to +1.4). On the chain there are
  enough towers that a team that holds what it takes and sends the next drone to the nearest
  free tower out-scores any plan that does not react. The reaction is where the skill is.
  Against the valley's five towers the default commander only broke even (+0.54 mean).
- So the next question is not whether to add towers (it does what the plan wanted: no single
  allocation wins) but whether a *captain* can beat the default commander. That is a job for the
  captain league on the chain layout.

## Step 3: the first captain that beats the commander on the chain

`SIM001_LAYOUT=chain cognition_league run --game captain --strategies claude-tuner,claude-engineer
--seeds 64` (one generation, $2.26 of Claude). The three numbers-only challengers were kept out
(−0.05, −0.51, −0.54); the engineer's code change was promoted (+0.69, lower bound +0.52).

The change (now in `commander.rs`, spec in `05-commander.expect`): **the heavy drone, which counts
double at a tower, goes to the spire** when the team does not hold it and no drone is on it,
instead of joining the nearest flank as a spare; and a drone recharges earlier when enemies are
in its circle, at the contest drain's rate (without it the new rule lost 2.75 drones a game).

Checked on 256 seeds that the league never saw (`judge`, challenger against the old build, both
sides of each seed, 512 games):

| Layout | Mean difference | Lower bound | Games won / lost | Points a game |
| --- | --- | --- | --- | --- |
| chain | **+0.754** | +0.698 | 409 / 102 (1 drawn) | 584 against 491 |
| valley | −0.010 | −0.100 | 261 / 251 | 526 against 544 |

- The idea is worth a lot on the chain (a team with five drones can use a double-weight drone
  where it counts, and the spire is the one tower worth 1.5) and nothing on the valley, where
  the default commander's flank spare did as well.
- The tuner's numbers (never give up a contested tower) looked good on the seeds Claude tried
  (+0.29) and lost on the league's: a pick among several tries on one set of seeds. The league's
  fresh seeds are the check that catches it.

## Step 4: an Opus league run (chain, two generations), and what was kept

`SIM001_LAYOUT=chain cognition_league run --game captain --generations 2 --strategies
claude-engineer,claude-tuner --claude-model opus --seeds 128` from master's commander. Two
promotions: generation 1 the tuner's numbers (`defend_range` 0, `give_up` 5, `stay_away` 90;
+0.51 by the league), generation 2 the engineer's allocation code (+0.49 on top of them).
Opus's own estimates were within 0.06 of the league's (the earlier run's were far more
optimistic). The engineer proposed the same code in both generations; it was kept out in
generation 1 (+0.16) because the opponent had not yet dropped its defence.

An Opus subagent re-judged them on 256 seeds the league never saw (3,000,000 and up; 512 games a
row), against master's commander:

| Challenger | Chain | Valley |
| --- | --- | --- |
| The final champion (allocation code + the tuner's numbers) | +0.756 (+0.698) | **−0.110** (−0.198) |
| The tuner's numbers alone | +0.496 (+0.438) | **−0.303** (−0.391) |
| The allocation code alone, with master's numbers | +0.181 (+0.124) | +0.157 (+0.073) |

- **The numbers overfit the layout (and the opponent):** turning the defence off pays on the chain
  and loses on the valley. They are not made the defaults.
- **The allocation code is kept.** Drones and unclaimed towers are matched greedily by the time
  the drone needs to win the tower (the flight, and the capture at its weight) per point the tower
  scores; a drone is not sent where its battery cannot last the flight, the capture and the way
  home. On a third set of fresh seeds (4,000,000 and up), merged with master's numbers:
  chain +0.180 (lower bound +0.128), valley +0.129 (lower bound +0.043).
- **Not tested on its own:** the battery condition. A drone that the pairing skips falls through to
  the existing step that sends the leftover drones to the nearest tower it does not hold, so the
  condition cannot change a simple order; the greedy pairing has a unit test.
- **Next for the league:** judge on the chain and the valley together (no valley regression), and
  against a pool of past champions, not only the default commander.

## Step 5: a belief about where the enemy is (`recreation_towers::Belief`)

A particle filter: 128 guesses for each enemy drone. Each guess flies to a tower (those the enemy
does not hold count double), circles it while it is unheld, and goes on with a chance of 0.01 a
second once the team holds it. A drone the team sees collapses its cloud onto it. A place that the
team looks at (its linked drones 150 m, its held towers 200 or 250 m, its base 150 m) and sees no
drone in loses its guesses; if all the guesses are in such places the cloud starts again on the
ring that the drone can have reached from its last sighting. The unit tests were written first
(they failed against stubs); three expectations in `16-belief.expect`.

`cargo run --release -p cognition_towers --example belief 40 [layout=1]`: in games of the default
commanders, each second, for each Red drone in the air: how far Blue's belief (its average) and
Blue's last sighting are from it, and how much of the belief is within 100 m of the drone (a
guess that knows nothing puts 3 % there).

| Chain, 40 games | Samples | Belief error | Last-seen error | Belief within 100 m | Last-seen within 100 m |
| --- | --- | --- | --- | --- | --- |
| seen now | 10645 | 1 m | 0 m | 100 % | 100 % |
| unseen 0 to 10 s | 3155 | **41 m** | 64 m | 86 % | 82 % |
| unseen 10 to 30 s | 3230 | **128 m** | 157 m | 49 % | 42 % |
| unseen 30 s or more | 1790 | 182 m | **145 m** | 38 % | **47 %** |
| never seen | 9049 | **158 m** | 291 m | 21 % | 15 % |

| Valley, 40 games | Samples | Belief error | Last-seen error | Belief within 100 m | Last-seen within 100 m |
| --- | --- | --- | --- | --- | --- |
| unseen 0 to 10 s | 7722 | **40 m** | 66 m | 86 % | 83 % |
| unseen 10 to 30 s | 10469 | **123 m** | 178 m | 57 % | 37 % |
| unseen 30 s or more | 14452 | **217 m** | 287 m | 38 % | 22 % |
| never seen | 6074 | **141 m** | 251 m | 19 % | 20 % |

- The belief beats "stay where last seen" in every row but one. The exception is the chain, 30 s
  or more out of sight, where the default commander's drones mostly stay at their tower; the
  belief puts 8 points less on it.
- The first model (a drone keeps flying between towers) lost to last-seen after 10 s: the real
  drones are at a tower, not on the way. The setting 0.01 for the chance of leaving a held tower was
  chosen by running the example at 0.3, 0.1, 0.03, 0.01 and 0 (0.01 and 0 were best and nearly
  the same); it is a fit to the default commander's play, and a captain that plays differently
  needs a different motion model.
- The belief is not used by the commander yet. Next: a captain that looks ahead with it.

## Step 6: a captain that thinks ahead (`lookahead.rs`)

Every 5 s a commander with `SIM001_LOOKAHEAD=1` (or `Commander::with_lookahead(true)`) rolls a
small model of the game forward 45 s and asks whether changing one drone's order would be worth
more than 2 points. The model (`Sim`): our drones fly to their towers at 16 m/s; a tower's bar
moves 5 % a second for each unit of weight more in its circle; a held tower scores its points;
the value of a future is its points plus 10 points for each full bar (so progress counts). The
enemy drones are guesses drawn from the `Belief` of step 5: each flies to its tower and, if the
enemy holds it, to the nearest one it does not. The search tries every `Take(tower)` for every
drone that is free (not charging, in contact), against 8 futures with the same guesses for the
base orders and each change, and takes the change with the best average, if it is above the
margin. The decision is made for the commander's own orders, which it then keeps as usual.

Judged by the league's `judge` against master's commander on 256 seeds neither side had seen
(6,000,000 and up; both sides of each seed, 512 games):

| Challenger | Chain | Valley |
| --- | --- | --- |
| Thinks ahead, with the belief | **+0.319** (lower bound +0.251) | **+0.327** (+0.241) |
| Thinks ahead, belief never updated (the enemy stays at its start) | +0.073 (+0.007) | +0.186 (+0.095) |
| For comparison: the allocation by time to win (PR 69) | +0.180 (+0.128) | +0.129 (+0.043) |

- **The belief carries most of the gain on the chain** (+0.25 of +0.32) and about half on the
  valley (+0.14 of +0.33). Planning with the model alone still helps on the valley.
- It takes about 4 s to play the 256 seeds (the model is cheap), so the search can be made bigger:
  more futures, two changes at a time, a longer horizon.
- Off by default. Turning it on in the champion is a decision for a person (the spec and the
  numbers of `05-commander.expect` are about the commander without it).
- Not tested: against a different opponent than master's commander; with a beaten opponent the
  belief's motion model (fitted to that commander's play) may be worse.
