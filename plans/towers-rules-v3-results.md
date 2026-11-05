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
