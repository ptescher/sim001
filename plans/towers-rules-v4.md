# Towers v4: a game of decisions

Status: a design, not built. Against v3 (`plans/towers-rules-v3.md`, measured in
`plans/towers-rules-v3-results.md`). Numbers are targets to check in the arena, not measurements.

The purpose: a game that **tests our models**, the commander and each pilot. A good test game has
many decisions, every option in a decision beats something and loses to something, and no plan
made before the game wins it. The game stays a real one: the commander commands real drones over
real radios, and every rule says how it is done on SITL and in the field (section 7).

## 1. What the arena says today

| Finding (v3 results) | What it means for a test |
| --- | --- |
| On the valley with the spire at 2 points, one allocation (balanced) beat every other | One dimension, and it had a dominant answer |
| At 1.5 points, and on the chain, the allocations cycle (spread > mass > raid > spread) | A cycle is possible, but only in *where* drones go |
| A reactive commander beats every fixed allocation by +0.9 to +1.4 | Good: skill exists, and it is in the reaction |
| Lookahead with the belief +0.3 over without | Information is worth something to the commander |
| No on-board policy beats "press on" (every retreat loses, up to −0.99) | **The pilot has no decision.** Nothing a drone can see changes the right answer |
| The only thing that hurts a drone is battery drain in a contested circle | Drones cannot fight; they only stand in circles |

So the commander plays one dimension and the pilot none. v4 adds dimensions, each a choice with a
counter, and puts costs where the pilot's own camera can see them.

## 2. The goal, as tests

Every criterion is a number from the arena (section 8), not an argument.

| # | Criterion | Target |
| --- | --- | --- |
| C1 | No dominance: no strategy in the population beats every other | none at 3 standard errors |
| C2 | A wide equilibrium: the Nash mixture of the population is spread out | at least 8 strategies with 5 % or more, differing in at least 3 dimensions |
| C3 | Cycles: the share of the payoff matrix that is cyclic (rock-paper-scissors), not a ranking | 30 % to 70 %: some of it must be a ranking, or a model cannot be rated |
| C4 | Every mechanic pulls its weight: without it, C2 or C3 gets worse | each mechanic, by ablation |
| C5 | Reaction beats preparation: a commander that scouts and adapts beats the best fixed mixture | +0.3 or more |
| C6 | The pilot matters: the best on-board policy beats "fly the order" | +0.15 or more (today 0) |
| C7 | No snowball: P(win given the first tower, or the first drone down) | below 0.75; games last 60 % of the clock or more |
| C8 | Real: each rule has a way to run on SITL and in the field | section 7 |

## 3. The decisions of one game

| When | Commander decides | Pilot decides (on board) |
| --- | --- | --- |
| Before the game | **Loadout**: airframes, payloads and spares from a budget (4.2), given the **forecast** (4.9) | - |
| Opening | Where each drone goes (4.1), high or low (4.3), loud or quiet (4.5), how much to script (4.7) | Route leg by leg: lane low or over the trees high |
| Middle | Which tower to take, defend or give up; when to rotate batteries (4.8); when to jam, hunt, scout | Press on, hold at the edge of a shadow, or leave; evade or keep capturing when a hunter is seen; chase or let go; break silence to report or stay quiet |
| End | All in, or keep what is held | Spend the last of the battery, or bring the airframe home |

## 4. The dimensions

Each dimension is a cycle or a trade on its own. The game is the product of them, so a strategy is a
choice in each, and a choice that is right in one is often wrong in another.

### 4.1 Where (allocation): kept

As now. The spire at 1.5 points (v3 step 1d showed 2 is a dominant prize). The chain and the valley
both stay; the layout is drawn per game (4.9), so an allocation learned on one does not carry over.

### 4.2 What you bring (loadout): new

Before the game each commander picks airframes and payloads from a **budget of 20**, hidden from
the other team. Five fly at a time; the rest are **spares** (4.8). A sighted enemy drone shows its
airframe; its payload shows only when it is used (it jams, tags or relays).

| Airframe | Cost | Weight in a circle | Battery | What it is |
| --- | --- | --- | --- | --- |
| Light (Seeker 3 quad) | 2 | 0.5 | 150 s | Fast to accelerate, hovers, short battery, hurt most by wind |
| Standard (T1-S VTOL) | 3 | 1 | 240 s | The workhorse |
| Heavy (T2 Cruza) | 4 | 2 | 300 s | Holds a tower alone; slow, wide turns, the easiest to tag |

| Payload | Extra cost | On | Does |
| --- | --- | --- | --- |
| Camera (every drone) | 0 | any | Sees 150 m (as now) |
| Tagger | 1 | light, standard | Can tag (4.6); counts 0 in a circle |
| Relay | 1 | standard, heavy | Relays 200 m and for up to 3 drones (4.4) |
| Jammer | 2 | standard, heavy | A shadow that moves with it, when switched on (4.7) |

Today's squad (three standard, a heavy, a light) costs 15, so a budget of 20 is today's squad and
some payloads, or a bigger fleet with none. There are about 1,400 loadouts within the budget; with
each drone's first tower, height and emission, more than 10^8 openings.

### 4.3 How you fly (height): sharpened

High and low exist (only a low drone takes a tower). v4 makes height a trade in radio too:

| | High (55 to 75 m) | Low (16 to 20 m, lanes and clearings) |
| --- | --- | --- |
| Way | Straight over the forest | Along the lanes, 1.3 to 2 times longer |
| Takes a tower | No | Yes |
| Seen by | Anything within its sight, and DF (4.5) | Only along open ground, or from above within 75 m |
| Radio | Line of sight to its base mast, the whole field | **New:** 250 m to a friendly node (base mast, held tower, high drone in contact); the canopy cuts the rest |
| Can be tagged | Anywhere | Only over open ground (lane, clearing, lake) |
| Risk | Hunters | Trees |

### 4.4 Towers are radio towers: new

A tower its team holds is a **node**: it relays its holder's low drones within 250 m. A relay
payload makes its drone a node of 200 m for up to three drones. So holding towers builds a network,
and a strike deep in enemy ground needs a relay overhead (visible, and a target) or must go dark
on a script. Snowball check (C7): a held tower's node does not reach into an enemy shadow, so the
network stops where the other team's towers start.

### 4.5 Who you talk to (emission and direction finding): from v3, revised

A drone is **loud** (telemetry and video down, the default) or **quiet** (receive only). A quiet
drone still hears orders, so the commander can steer it, but sends nothing: the commander has only
its last position and age, and its camera adds nothing to the team's picture. It is invisible to
direction finding.

**DF**: a fort or the spire that a team holds takes a **bearing** (a direction, not a place) on each
loud enemy drone within 400 m. Two towers bearing on one drone give a fix of about 30 m. Bearings
go into the team's `Belief` (v3 step 5), which already takes sightings.

The choice is matching pennies between attacker and defender:

| Attacker \ Defender | Hunts by DF (taggers wait for a bearing) | Watches (a high drone over its tower) |
| --- | --- | --- |
| Loud | Found early, tagged on the way | Can call for help and adapt: about even |
| Quiet | Not found: reaches the clearing | Seen in the clearing, and cannot report or be warned |

And the pilot gets a decision of its own: a quiet drone that sees something worth telling **breaks
silence** for one report (its picture reaches the team; a DF bearing reaches the enemy).

### 4.6 The fight (interception): new

A drone with a **tagger** tags an enemy drone within 8 m (the tag of the executor game,
`plans/league.md`), if the target is high or over open ground. Below the treetops in the forest a
drone cannot be tagged, but nothing can fly there either, so a low drone's cover is **not being
seen**, not immunity. The tagged drone is **down**: the airframe is lost (4.8).

- A tagger needs a **track**: its own camera, or the team's picture while it is in contact.
- A tagger has **two tags**, then it lands at its base to re-arm (as a battery swap).
- A tagger chasing (faster than cruise) uses battery twice as fast.
- A tagger can tag a tagger.

The roles that come out of loadout, height and emission, as a cycle:

| Role | Beats | Loses to | Why |
| --- | --- | --- | --- |
| **Holder** (heavy, low, on a tower) | Striker | Hunter | Weight 2 against 1; slow and wide turns, so it is tagged |
| **Striker** (standard, low, quiet) | Hunter by DF | Overwatch, Holder | Not found; but seen in the clearing from above |
| **Overwatch** (standard, high over its tower) | Striker | Hunter, Jammer | Sees the clearing and cues its taggers; it is high and steady, and its cue goes by radio |
| **Hunter** (light, tagger) | Holder, Overwatch, Relay, a jammer that is on | Striker, Decoy | Needs a track; two tags, then home |
| **Jammer** (4.7) | Overwatch's cue, Relay, orders | Hunter (homes on the jam), quiet scripts | Loud by nature |
| **Decoy / scout** (light, loud, high) | Hunter's tags; gives the picture | Anything that fights for a tower | Holds nothing |

### 4.7 How much you plan (scripts, orders, jamming): from v3

A **jammer**, switched on, is a shadow of 100 m round itself that moves with it: enemy drones in it
are out of contact, and an enemy tagger in it has only its own camera. On, it is loud to DF from
800 m (a fix, not a bearing), uses battery 1.5 times as fast, and any tagger can **home on it**
without a track.

Orders take **airtime**: a short order (go home, hold) gets through a jammer's edge where a full
script of steps does not, and a script upload needs contact for a time that grows with its length
(v3 section 4: steps and interrupts). The cycle:

| | Beats | Loses to |
| --- | --- | --- |
| **Short orders** (commanded, adaptive) | Scripts: the enemy's script is predictable, and the belief learns it | Jammer: no orders get through |
| **Scripts** (planned ahead, flown alone) | Jammer: they need no link | Short orders: they cannot react |
| **Jammer** | Short orders | Scripts, and hunters |

### 4.8 Tempo (logistics): changed

- **Land and swap.** No recharging in flight over the base. A drone lands at its base (or on the
  pad of a depot its team holds), is on the ground 30 s, and takes off with a full battery. On the
  pad of a depot it can be tagged.
- **Airframes are finite.** A drone that is down (battery empty, tagged, crashed) is lost. The next
  spare takes off from the base after 30 s. With no spares left, the team flies fewer drones.
- **Battery empty is down**, as now, but now it costs an airframe, so stretching a battery is a bet.

The tempo trade: launching together gives a big push and a hole when all land together; staggering
never has a hole and never a push. A scout that sees the hole punishes it, which is the
information dimension again.

### 4.9 The conditions: new

Drawn per game from its seed, and told to both commanders before the loadout as a **forecast**
(with some error):

| Condition | Range | What it changes |
| --- | --- | --- |
| Wind | 0 to 10 m/s, any direction, may shift once | Leg times up and down wind; the quad most |
| Cloud base | 50 m to none | Below it, "high" sees nothing of the ground: low play is favoured |
| Visibility | 100 to 250 m | Every camera's range |
| Layout | valley, chain, lopsided (v3: kinds swapped on the valley) | Which allocations are good |

A strategy fixed before the game meets a different field each time, so a model has to read the
forecast and plan for it (C5).

## 5. The pilot

Today a pilot is the executor flying an order (or a person flying sticks with the order as a
mission). v4 gives the pilot **intent** instead of a bare order: a task (take, hold, watch, hunt,
relay, scout, escort), the limits the commander sets (height, emission, abort conditions), and a
deadline. How to fly it is the pilot's.

The pilot's decisions, each settled by something only the drone knows:

| Decision | On-board information that settles it |
| --- | --- |
| Route: lane low or over the trees high, leg by leg | Its picture of hunters, its battery, wind on its own airspeed |
| In a shadow: press on, hold at the edge as a picket, or leave | The bar, the enemy in its camera, its battery, time out of contact |
| A hunter in sight: keep capturing, break to the lane, climb and run | Distance and closing speed of the hunter, the bar, battery |
| As a hunter: chase, or let it go (a decoy? into a jammer?) | The target's airframe, tags left, battery, whether it is being led into a shadow |
| Quiet: break silence to report | What it saw, how far from enemy towers (DF) |
| Battery: land now, or stretch | Battery, wind home, the bar |

This is the "cost to pressing on that the camera can see" that v3 step 1 found missing: a hunter
in the camera makes pressing on wrong some of the time, and only the drone knows when.

On a flight controller this stays small: the pilot policy is the `Edge` interface of `plans/edge.md`
(`EdgeObs` in, the instruction queue out), which a single-board computer could later run, as
`plans/actors.md` allows.

## 6. What is cut, and why

| Cut | Why |
| --- | --- |
| Guns, hit points, dogfighting | A captain never flies a dogfight; a tag at 8 m is the real counter-drone (an interceptor) and the executor already has it |
| v3's "EW escort beats the interceptor" | A jammer is loud: a hunter homes on it. The jammer's job is against links, not against hunters |
| Recharging while flying over the base | Not real. Batteries are swapped on the ground |
| Points for kills | A kill is worth the airframe it costs the enemy; points for it would snowball |

## 7. Real on SITL and in the field

| Rule | Fast arena | ArduPlane SITL | Field |
| --- | --- | --- | --- |
| Loadout, payloads | Referee | Referee; airframe = its SITL model | Real airframes; payloads are a rule (see jammer) |
| Low radio range | Referee | The server drops the frames, as for a shadow | The link filter drops them, as for a shadow |
| Towers and relays as nodes | Referee | Server | Link filter |
| Quiet | A flag | Stream rates to 0 (`SRn_*`); the drone still takes `COMMAND_INT` | The same |
| DF | Referee, from who is loud | Referee | Referee first; a real direction finder later |
| Tag | Referee, 8 m | The executor's tag; its envelope keeps 4 m | GPS positions; the envelope keeps 4 m, nothing touches |
| Jammer | Referee | Frames dropped, and GPS degraded (`SIM_GPS*`) | **A rule only**: the link filter drops frames. No real jamming (illegal); no GPS effect |
| Land and swap | Referee | VTOL landing at the base, disarm, re-launch | A person swaps the pack |
| Spares | A count | A fresh SITL instance | Real airframes |
| Wind | Per-class speed | `SIM_WIND_*` | Real; the forecast is the real one |
| Cloud base, visibility | Referee | Referee | Declared for the game |

## 8. How we test the models

### 8.1 The measuring stick (built first)

`cognition_league` grows a `nash` command: play a population of strategies against each other on
both sides of many seeds, and report the matrix, its Nash mixture (a linear program), the support,
and the cyclic share (the matrix split into a ranking part and a cyclic part, as in "Real world
games look like spinning tops", Czarnecki et al. 2020). It runs on today's game first, as the
baseline every later step must improve (C4).

The population: parametrised scripted strategies (a loadout, an allocation, a height and emission
per role, a rotation plan), the league's past champions, and the Claude strategies.

### 8.2 Testing a commander

| Test | What it measures |
| --- | --- |
| Nash-averaged score against the population | Strength that a pile of similar opponents cannot inflate |
| Exploitability: a league of exploiters trained against the frozen commander | How far a best response gets; a good model is hard to exploit, not only good on average |
| Series: best of five against one opponent drawn from the Nash mixture, loadout hidden | Does it read the opponent? The second game should go better than the first |
| The forecast varied with everything else held | Does it plan for the conditions? |
| A latency budget (advice 2 s late cost −0.16 in v3) | Can it decide in time |

### 8.3 Testing a pilot

| Test | What it measures |
| --- | --- |
| **Vignettes**: a bank of situations (a drone, what it sees on board, the rest of the game frozen), each action valued by many arena rollouts | Regret: the value of the best action less the value of the pilot's. Exact, cheap, and a model sees only `EdgeObs` |
| Pilot swap: the same commander with different pilot policies, in the league | What a pilot is worth in whole games (C6) |
| Intent kept: the pilot's flight against the commander's limits | A pilot that wins by ignoring the commander fails |
| SITL: the top vignettes flown on ArduPlane | That the arena's answer is the right one on a real airframe |

A person can play every seat (the commander app, the cockpit), so a model can be compared with people.

## 9. Order of work

Each step starts with its expectations (`tdd`, `writing-specs`), then the arena, then the server,
and ends with its numbers in `plans/towers-rules-v4-results.md`. A mechanic that does not improve
C2 or C3 is cut, not kept.

1. **The measuring stick** (8.1) on today's game: the baseline.
2. **Logistics and loadout**, airframes only: land and swap, spares, the budget. Rules only.
3. **Height and radio**: the canopy range, towers as nodes, the relay payload.
4. **Emission and DF**, into the `Belief`.
5. **The fight**: tagger, hunter, overwatch. The vignette bank starts here: this is where the pilot
   first has a decision.
6. **Jammer and airtime**: scripts against orders (needs the instruction queue of `plans/actors.md`).
7. **Conditions and layouts per game**, with the forecast.
8. **SITL**: measure each airframe first (v3 section 7: speed, turn, climb, endurance, the landing
   and re-launch), then the top rows of the matrix on SITL.

## 10. Open questions for the owner

- **Hidden loadout?** Hidden makes the draft a guessing game (good for testing reading the
  opponent); shown makes it a counter-pick. The plan says hidden until seen.
- **Land and swap on SITL**: 9 of 33 crashes in the 14 games of 2026-11-14 were re-launches. Swapping
  needs the take-off to be reliable first, or the arena and SITL will disagree.
- **Tags among real drones**: 8 m with the envelope's 4 m separation, as in the executor game. Is
  that close enough to fly in the field?
- **Towers as nodes**: the most likely to snowball. If C7 fails, the first to go.
