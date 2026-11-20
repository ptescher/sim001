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

## Step 4: loud and quiet drones, and direction finding (2026-11-20)

Built: a drone is loud or quiet (`Posture`, sent with an order as `Directive::posture`). A quiet
drone hears its orders and sends nothing: no telemetry, no picture, no battery; its team keeps it
where it last heard from it; it goes loud at its bingo battery (its failsafe). A fort or the spire
that a team holds takes a bearing on each loud enemy drone within 400 m (`TeamView::bearings`);
two bearings on one drone put it on the team's map. Spec: `09-towers/rules/21-emission.expect`.
The v4 population adds "raid, quiet" and "balanced, quiet".

| Rules | Equilibrium | Formations with 5 % or more | Cycle share | Beats every other |
| --- | --- | --- | --- | --- |
| Valley, v4 step 4 | raid, two heavies 100 % | 1 of 17 | 11 % | raid, two heavies |
| Chain, v4 step 4 | raid, quiet 50 %, balanced, two heavies 32 %, commander 18 % | **3 of 18** | 26 % | none |

| Cell | Valley | Chain |
| --- | --- | --- |
| commander over raid | −1.14 | **+0.70** |
| commander over raid, quiet | −1.43 | **−0.77** |
| balanced, quiet over balanced | +0.07 | +0.07 |
| raid, quiet over raid | +0.47 | +1.12 |

- **The first counter.** On the chain the commander beats the loud raid and loses to the quiet
  one: it reacts to the raiders that its towers find, and it cannot find quiet ones. The chain's
  equilibrium is a mix of three for the first time.
- The first version let a quiet drone report its battery, and kept its position at the base: a
  formation then thought its quiet drones were near home and left them at their posts longer
  (balanced, quiet +1.20 over balanced). With no battery report and the failsafe at bingo, the
  quiet balanced formation is level with the loud one (+0.07).
- **Not found out**: why a quiet raid still beats a loud raid between two fixed formations (+0.47,
  +1.12), where neither side uses what it finds. The raid's drones are in the enemy's radio shadow
  for much of the game, so their reports are stale either way; the difference is elsewhere.

## Step 5: tags (2026-11-20)

Built: a drone with a tagger tags the nearest enemy drone within 8 m (in the air) that is above
the treetops or over open ground; the tagged drone is down and its airframe lost. A tagger has two
tags, two more at a swap or as a new airframe, counts for nothing in a circle, and uses twice the
battery when it flies faster than 20 m/s (it chases). A team sees an enemy drone's airframe, and
its payload once it has tagged; a tagged drone's last picture shows its team the tagger.
`Order::Intercept(k)`: the arena's hunter flies at its quarry at 22 m/s, to its height, by its own
camera or where its team last saw it; with no track it circles. Spec:
`09-towers/rules/22-fight.expect`. Population: the `Hunt` post (circle high over the centre, go
for the nearest enemy seen now) in "balanced, hunter", "raid, hunter" (a light tagger, two light
spares, 20) and "two hunters" (a standard and a light tagger, one spare, 19).

Today's game is unchanged: `nash --seeds 100` without v4 gives the step 1 table to the last digit.

| Rules | Equilibrium | Formations with 5 % or more | Cycle share | Beats every other |
| --- | --- | --- | --- | --- |
| Valley, v4 step 5 | balanced, hunter 87 %, raid, hunter 8 %, raid, quiet 4 % | 2 of 20 | 19 % | **none** |
| Chain, v4 step 5 | raid, quiet 50 %, two hunters 29 %, commander 21 % | 3 of 21 | **32 %** | none |

| Cell | Valley | Chain |
| --- | --- | --- |
| balanced, hunter over balanced | +0.66 | +0.66 |
| raid, hunter over raid | +1.75 | +1.28 |
| balanced, hunter over raid, two heavies | +0.74 | +0.19 |
| two hunters over raid, two heavies | +0.23 | +1.00 |
| balanced, hunter over raid, quiet | −0.16 | −0.08 |

- **No strategy beats every other on either layout**, for the first time on the valley: the two
  heavy drones that won steps 2 to 4 are what hunters tag (big, slow, and in the open at their
  tower), and the chain reaches the cycle share target (C3, 30 to 70 %).
- The cycle the plan wanted shows: heavy holders lose to hunters, hunters cannot find quiet
  raiders (a hunter needs a track, and a quiet drone is found only by eye), and the commander's
  DF beats loud raiders (step 4).
- The equilibrium is still narrow (C2 wants 8 or more): the population is small, and hunters
  are worth a lot to any formation (+0.66 to +1.75), so most rows are dominated by their hunter
  variant.

## Step 6: jammers (2026-11-20)

Built: a drone with a jammer and `Posture::Jam` cuts the radio of the enemy drones within 100 m
(a shadow that moves with it), is found from 800 m by one enemy fort or spire (a fix, not a
bearing), shows what it carries, and uses 1.5 s of battery a second. `OwnDrone::payload` tells a
captain what each of its drones carries. Spec: `09-towers/rules/23-jammer.expect` (and «A tower
that nobody holds blocks no radio» now says the jammers are off). Population: "raid, jammer" and
"balanced, jammer" (a jammer on the third standard drone, one light spare, 19).

| Rules | Equilibrium | Formations with 5 % or more | Cycle share | Beats every other |
| --- | --- | --- | --- | --- |
| Valley, v4 step 6 | balanced, hunter 87 %, raid, hunter 8 %, raid, quiet 4 % | 2 of 22 | 19 % | none |
| Chain, v4 step 6 | raid, quiet 50 %, two hunters 29 %, commander 21 % | 3 of 23 | 31 % | none |

| Cell | Valley | Chain |
| --- | --- | --- |
| raid, jammer over raid | −0.04 | −0.43 |
| commander over raid, jammer | −0.43 | +1.07 |
| raid, jammer over raid, quiet | −0.47 | −1.39 |

- The jammer changes nothing between fixed formations: they never need the radio. Against the
  commander it costs more than it gives on the chain, where one tower finds it from 800 m.
- The cycle it is for (jammer beats orders, scripts beat the jammer, orders beat scripts) needs a
  captain that scripts its drones and one that orders them second by second. Neither is in the
  population yet: that is the next population work, not a rule.

## Step 7: the weather (2026-11-20)

Built: each game's weather is drawn from its seed (`Rules::weather`): wind up to 10 m/s, low cloud
(a base of 40 to 90 m) in half the games, poor visibility (100 to 250 m) in half. A drone at or
above the cloud base sees nothing and nothing sees it; no eye sees past the visibility; bearings
and the radio are not touched. Both teams get the weather in the `TeamView`. Spec:
`09-towers/rules/24-weather.expect`. Today's game is unchanged (the step 1 table again).

| Rules | Equilibrium | Formations with 5 % or more | Cycle share | Beats every other |
| --- | --- | --- | --- | --- |
| Valley, all v4 rules | raid, hunter 100 % | 1 of 22 | 16 % | none |
| Chain, all v4 rules | raid, quiet 54 %, balanced, two heavies 32 %, commander 14 % | 3 of 23 | 22 % | none |

| Cell | Valley, step 6 | Valley, step 7 | Chain, step 6 | Chain, step 7 |
| --- | --- | --- | --- | --- |
| raid, two heavies over balanced, hunter | −0.78 | **+0.19** | −0.19 | +0.04 |
| balanced, hunter over raid, quiet | −0.16 | −0.75 | −0.08 | −0.12 |

- In cloud and poor visibility a hunter often has no track, so the heavy drones come back against
  hunters: the weather moves the answer, which is what it is for.

## Where the v4 rules stand

| Criterion | Target | Today's game | All v4 rules |
| --- | --- | --- | --- |
| C1 no dominance | none at 3 errors | raid (valley), commander (chain) | **none** on either layout |
| C2 equilibrium support | 8 or more with 5 % | 1 | 1 (valley), 3 (chain) |
| C3 cycle share | 30 to 70 % | 15 to 20 % | 16 to 22 % (32 % at step 5, chain) |
| C7 no snowball | P(win given first tower) under 0.75 | not measured | not measured |

The counters the plan wanted are there, each seen in a cell of the table: hunters beat heavy
holders, quiet raiders beat hunters and the commander's direction finding, the commander's
direction finding beats loud raiders, the weather takes hunters' tracks away. What is narrow is
the **population**: fixed formations with one change each. A fixed formation cannot react, so most
rows are dominated by the variant that adds the best tool (a hunter, quiet drones), and the
equilibrium is a corner. Next, in this order:

1. **A wider population**: parametrised strategies (each drone's post, height, emission, payload
   and rotation chosen independently, sampled), and reactive captains that use the new tools (a
   commander that hunts what its towers find, goes quiet on a raid, places a relay, switches a
   jammer on before an attack). Then C2 and C3 again.
2. **The scripting captain and the ordering one**, for the jammer's cycle (step 6).
3. **The pilot's vignettes** (section 8.3): the rules now give the pilot decisions (evade or keep
   capturing when a hunter is seen, chase or let go, break silence, land or stretch).
4. C7 (snowball) and the game's length, then SITL for the top rows.

## Step 8: a wider population (2026-11-20)

Built: the `Tactician` (`cognition_towers::tactics`): the default commander's towers, with the v4
tools: a tagger hunts (it counts for nothing in a circle), a drone attacking a tower its team does
not hold goes quiet, a jammer is on within 250 m of the tower it attacks, and home is a landing.
Four in the v4 population ("tactician" and "tactician, loud" with a light tagger and two light
spares; "tactician, jammer" with a jammer and a tagger; "tactician, heavies" with two heavy
drones and a tagger). `nash --mixes N [--mix-seed S]` adds N ways to play drawn from a seed: each
drone's airframe, payload and post (a hunter carries a tagger), quiet or loud, landing or
circling, light spares to the budget. The report now also counts the formations within 0.05 of
the best against the equilibrium (the linear program gives one corner, which can understate how
many are nearly as good).

`nash --seeds 40 --set v4=1 --mixes 24` (and `--set layout=1` first for the chain):

| Rules | Formations | Equilibrium | Within 0.05 of the best against it | Cycle share | Beats every other |
| --- | --- | --- | --- | --- | --- |
| Valley, all v4 rules | 50 | raid, hunter 100 % | 2 (two hunters, −0.03) | 22 % | none |
| Chain, all v4 rules | 51 | tactician, heavies 100 % | 1 | 23 % | none |

- **A wider population did not widen the equilibrium.** No mix and no tactician beats the
  equilibrium's formation in its cell, on either layout. The table is mostly a ranking (77 %).
- **The tagger is the ranking.** Every formation near the top has one, and in each step since 5 the
  hunter variant of a formation beat its plain one by +0.66 to +1.75. A tagger costs 1 of the
  budget, takes a drone out for good (the airframe is lost), and its counters (quiet raiders, the
  weather) only hide targets from it; nothing makes it pay for being where it is. That is a rule to
  tune, not a population to widen.
- What to try next, each measured with the same table: a tag that costs the tagger (a net:
  one tag, and the tagger is down too), taggers that are easy to tag (a hunter that chases is
  loud: DF finds it, and the other team's hunters go for it), a higher price (3), or a tag only
  from behind (the executor game's rear cone). The target: the hunter variant of a formation level
  with the plain one on average, winning against some and losing to others.

## Step 9: tuning the tag (2026-11-20)

Four rules to try, each a field of `Rules` that is off by default (none is in `Rules::v4()`), with
unit tests, and set by `nash --set`: `tag_net=1` (a tag downs the tagger too), `chase_found=800` (a
tagger that chases is found by one enemy tower, as a jammer), `tagger_cost=3` (and `budget=N`), and
`tag_cone=60` (a tag only from behind). A loadout the rules' prices make too dear is fitted (its
last spares left out) or, if it still costs too much, left out of the population.

`nash --seeds 40 --set v4=1 --set RULE --mixes 24`:

| Rules (all v4, and) | Layout | Equilibrium | Within 0.05 | Cycle share |
| --- | --- | --- | --- | --- |
| nothing more (step 8) | valley | raid, hunter 100 % | 2 | 22 % |
| `tag_net=1` | valley | tactician, heavies 63 %, raid, hunter 30 %, balanced, hunter 7 % | **4** | 21 % |
| `chase_found=800` | valley | raid, hunter 100 % | 1 | 22 % |
| `tagger_cost=3` | valley | raid, hunter 100 % | 1 | 28 % |
| `tag_cone=60` | valley | raid, hunter 100 % | 1 | 22 % |
| `tag_net=1` | chain | tactician, heavies 100 % | 1 | 21 % |
| `tag_net=1 tagger_cost=3 budget=16` | valley | raid, two heavies 100 % | 1 | 22 % |
| `tag_net=1 jam_radius=80` | valley | tactician, heavies 73 %, raid, quiet 16 %, raid, hunter 11 % | 3 | 20 % |

What a hunter is worth, found out with a throwaway example (80 seeds, both sides): the raid with a
light tagger at the hunt post over the raid **+1.30**; the same post with a camera only over the raid
+0.12; so the tagger is the gain (+1.24). With the net, both sides lose about as many drones (2.51
and 2.63 a game) and the hunter still gains +1.29: a tagger counts for nothing in a circle, so
trading it for a drone that takes towers costs its team nothing. And a price does not bite, because
the budget buys only spares, and spares are worth almost nothing (step 2).

- **The net is the only change that widens the equilibrium** (valley 1 to 3 formations, 4 within
  0.05), and not on the chain.
- **A dearer tagger hands the game back to the heavy drones**: balancing one tool against the other
  moves the winner, not the width.
- **"Raid" is not a snowball of the radio**: with the fort's shadow at 80 m (it no longer covers
  the enemy base), raid still beats balanced by +1.29. It wins because nothing in the population
  defends its home tower: the commander loses to it too (−0.60). That is a weakness of the
  captains, which a better captain should find, and a test should show.

**Reading of steps 1 to 9.** The rules now give each tool a counter, and no formation beats every
other. A population of fixed formations, with or without a tactician, still finds a corner,
because the corner is about the captains (nobody defends a home, nobody hunts hunters) more than
about the rules. The equilibrium of fixed strategies is the wrong yardstick for the last step: the
test of a model is whether it finds and punishes these corners. Next, then:

1. **Exploiters** (section 8.2): the league's Claude strategies, or a search over the mixes'
   parameters, trained against the equilibrium's formation; how far they get is the measure. A
   game with a wide equilibrium is one where every exploiter gets a little, not one where one
   formation wins.
2. A defence for the commander (hold its home when the enemy is seen near it), so that a raid has
   a counter in the population.
3. Keep `tag_net` as a candidate for v4: decide with the exploiters, not the formations.
