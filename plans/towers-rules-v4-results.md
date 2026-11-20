# Towers v4: what the arena says

## Step 1: the measuring stick, and today's game (2026-11-19)

`cognition_league nash` plays each formation of the layout against each other on both sides of
each seed (`cognition_towers::formations`: the computer commander and the fixed formations of v3
steps 1c and 2), and reports the table, its Nash equilibrium (a linear program, `nash::equilibrium`),
the formations with 5 % or more of it, the cycle share (the table less its best ranking, as a share
of its sum of squares) and the formations that beat every other at 3 standard errors.
`--set rule=number` changes a rule (`layout=1` is the chain). Specs:
`expectations/08-league/05-equilibrium.expect`.

`cargo run --release -p cognition_league -- nash --seeds 100 [--set ...]` (200 games a cell; about
30 s for the valley, 100 s for the chain), on master at fbb8efa:

| Rules | Equilibrium | Formations with 5 % or more (C2: 8 or more) | Cycle share (C3: 30 to 70 %) | Beats every other at 3 errors (C1: none) |
| --- | --- | --- | --- | --- |
| Valley (today's rules, spire 1.5) | raid 100 % | 1 of 8 | 20 % | raid |
| Chain | commander 100 % | 1 of 9 | 15 % | commander |
| Valley, spire 2 | raid 100 % | 1 of 8 | 16 % | raid |
| Chain, spire 2 | own line 100 % | 1 of 9 | 15 % | none |

The valley, today's rules (the cell is the row's mean score over the column's):

| Row against column | commander | centre rush | wings | balanced | no centre | raid | turtle | heavy centre | mean |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| commander | - | +1.24 | +0.73 | −1.00 | +1.18 | −0.89 | +1.46 | −0.85 | +0.27 |
| centre rush | −1.24 | - | −1.12 | −1.32 | −1.12 | −1.38 | −1.09 | −1.11 | −1.20 |
| wings | −0.73 | +1.12 | - | +0.65 | −0.23 | −1.27 | +1.54 | −1.14 | −0.01 |
| balanced | +1.00 | +1.32 | −0.65 | - | −0.65 | −1.31 | +1.54 | −0.63 | +0.09 |
| no centre | −1.18 | +1.12 | +0.23 | +0.65 | - | −1.27 | +1.54 | −1.14 | −0.01 |
| **raid** | +0.89 | +1.38 | +1.27 | +1.31 | +1.27 | - | +1.53 | +0.61 | **+1.18** |
| turtle | −1.46 | +1.09 | −1.54 | −1.54 | −1.54 | −1.53 | - | −1.60 | −1.16 |
| heavy centre | +0.85 | +1.11 | +1.14 | +0.63 | +1.14 | −0.61 | +1.60 | - | +0.84 |

The chain:

| Row against column | commander | mass centre | balanced | own line | far line | raid | spread | snipe | no centre | mean |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| **commander** | - | +1.50 | +1.29 | +0.62 | +1.35 | +1.33 | +1.11 | +1.51 | +1.24 | **+1.24** |
| mass centre | −1.50 | - | −1.32 | −1.44 | −1.36 | −1.36 | −1.31 | −1.26 | −1.41 | −1.37 |
| balanced | −1.29 | +1.32 | - | −1.14 | +0.07 | +0.15 | −0.86 | +1.16 | −1.30 | −0.24 |
| own line | −0.62 | +1.44 | +1.14 | - | +1.60 | +0.80 | +1.10 | +1.52 | +1.13 | +1.01 |
| far line | −1.35 | +1.36 | −0.07 | −1.60 | - | +0.20 | −1.37 | +1.25 | −1.17 | −0.34 |
| raid | −1.33 | +1.36 | −0.15 | −0.80 | −0.20 | - | −0.72 | +1.29 | −0.95 | −0.19 |
| spread | −1.11 | +1.31 | +0.86 | −1.10 | +1.37 | +0.72 | - | +1.52 | +0.96 | +0.57 |
| snipe | −1.51 | +1.26 | −1.16 | −1.52 | −1.25 | −1.29 | −1.52 | - | −1.47 | −1.06 |
| no centre | −1.24 | +1.41 | +1.30 | −1.13 | +1.17 | +0.95 | −0.96 | +1.47 | - | +0.37 |

Reading it:

- **Today's game is one answer per layout.** Every row of the table above has an equilibrium of
  one formation, and the cycle share is 15 to 20 %: the table is mostly a ranking. This is the
  baseline that every later step must widen (criterion C4).
- **The v3 cycles are gone.** At 1.5 points for the spire, v3 (40 seeds, the commander of
  2026-10) found cycles on the valley and the chain (wings > balanced > raid > wings). On today's
  master (low-only capture, the lookahead commander, the tree and escape changes since), raid beats
  every other formation on the valley at 3 standard errors, also at 2 points.
- **Why raid wins is not found out yet.** It sends the light drone and a standard drone to the
  enemy home and the heavy to the centre. A guess to check: the enemy base is 90 m from its home
  tower, inside the fort's 160 m radio shadow, so a team that takes the enemy home cuts the enemy's
  drones off as they take off from their base. That would be a snowball (C7) as well as a dominant
  plan.
- **On the chain the reacting commander beats every fixed formation**, as in v3 step 2: good for
  C5 (reaction beats preparation), but the population has no second strategy that reacts, so the
  equilibrium is the commander alone.
- The equilibrium is one vertex of the linear program. With a formation that beats every other it
  is unique; when the tables get wider, a maximum-entropy equilibrium (Nash averaging) may be
  needed so that the support is not understated.

## Step 2: the battery swap, finite airframes and the loadout (2026-11-20)

`Rules::v4()` turns on what is built of the v4 rules; `Rules::default()` is today's game, so the
specs, the commander's tuning and SITL do not move. Built: a drone on the ground at its base for
30 s gets a full battery (`Order::Land`; the server flies it as `Home` until landing is flown on
ArduPlane), one that circles over its base recharges 4 s a second, a drone that goes down is lost
and the next spare takes its place after 30 s, and a team's `Loadout` (airframes and payloads,
first five fly, the rest spares) costs at most 20 (light 2, standard 3, heavy 4; tagger and relay
1, jammer 2). Specs: `09-towers/rules/19-logistics.expect` and the story «Keep a drone in the air».
A test in the rules' driver checks that every v4 expectation is covered by a plan that passes: the
coverage planner silently left some out when a new rule shared a step with old ones.

`nash --seeds 60 --set v4=1` (and `--set layout=1` first for the chain). The population is
today's formations, each with two light spares and landing for a swap, the commander (which
circles over its base to recharge), and variants: no spares, circling instead of swapping, and a
second heavy drone (18 of the budget).

| Rules | Equilibrium | Formations with 5 % or more | Cycle share | Beats every other |
| --- | --- | --- | --- | --- |
| Valley, v4 step 2 | raid, two heavies 100 % | 1 of 13 | 21 % | raid, two heavies |
| Chain, v4 step 2 | commander 100 % | 1 of 14 | 16 % | none |

| Variant against its plain formation | Valley | Chain |
| --- | --- | --- |
| balanced (swaps) over balanced, circles | +0.66 | +0.66 |
| raid (swaps) over raid, circles | +1.12 | +0.36 |
| raid, two heavies over raid | +1.37 | +0.78 |
| commander (two spares) over commander, no spares | −0.01 | +0.00 |

- Logistics alone makes no cycles, as expected: it adds choices but no counters.
- **The swap pays a lot** in the arena, where a landing never fails. On ArduPlane a landing and a
  re-launch are the risk that makes circling worth having (9 of 33 crashes on 2026-11-14).
- **Spares are worth nothing yet**: nothing takes a drone out but an empty battery, and the
  captains recharge in time. Tags (step 5) are what makes an airframe a stake.
- A second heavy drone is a large gain: weight 2 holds a tower alone. Tags should make the heavy
  drone the target it is meant to be (section 4.6).

## Step 3: the canopy and the radio (2026-11-20)

Built: a drone below the treetops is in contact on its own only within 250 m of its base or a
tower its team holds; a friend above the trees in contact relays it 250 m; a relay (payload)
relays any friend 300 m, into a radio shadow too; the mesh (70 m) is as before. Spec:
`09-towers/rules/20-canopy.expect`. The v4 population adds "commander, relay" and "raid, relay"
(a relay on the second standard drone and one light spare, 18).

| Rules | Equilibrium | Formations with 5 % or more | Cycle share | Beats every other |
| --- | --- | --- | --- | --- |
| Valley, v4 step 3 | raid, two heavies 100 % | 1 of 15 | 11 % | raid, two heavies |
| Chain, v4 step 3 | commander 100 % | 1 of 16 | 24 % | none |

- The relay is worth nothing to these strategies (commander −0.03 and −0.05, raid 0.00 and
  −0.06): a fixed formation never needs contact, and the commander sends the relay drone where it
  sends any drone. Placing a relay is a decision for a commander that knows about the canopy.
- Nothing punishes a drone yet: the counters come with emission and DF (step 4) and tags (step 5).
