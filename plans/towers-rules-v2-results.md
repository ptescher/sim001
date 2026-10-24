# Towers v2: what the arena says

Measured with `cargo run --release -p cognition_towers --example ruleset [v1|v2] SEEDS [field=value ...]`:
six fixed standing-order policies (the doc's `failsafe`, `persist`, `hold after 5`, `home after 10`,
`home after 30` and `surface after 5`), each pair on both sides of every seed, the default
commander for both teams. v1 is `Rules::v1()`, v2 is `Rules::default()` (the doc's numbers).
The policies that read on-board information (`EdgeObs`) are not built yet, so this is only the
floor: criteria 3 and 4 of the doc (headroom, information value) are not measured.

## v1 against v2 (200 seeds, 400 games a pair)

| | v1 | v2 (doc numbers) | Doc target |
| --- | --- | --- | --- |
| persist against failsafe | +0.73 ± 0.06 | **+0.34 ± 0.07** | < +0.30 |
| best fixed policy's lead over the second | 0.21 | **0.46** | < 0.15 |
| policies that beat every other at 3 standard errors | 0 | **1** (persist) | 0 |
| drones down, per team and game | 0.00 | 0.30 | 0.5 to 3 |
| a game lasts | 229 s of 480 (48 %) | 250 s of 600 (**42 %**) | at least 60 % |
| Blue wins, default against default | 49 % | 44 % | 45 to 55 % |

v2 as written halves persist's lead over the failsafe, and the failsafe now ties "home after
30 s". But persist still beats every other fixed policy, and the games end early: the spire
(2 points a second) and 600 points end a game in about 250 s.

## Tuning (the doc's order: contest_drain, link_timeout, mesh_range, decay_time)

100 to 200 seeds; "dominates" is the number of policies that beat all others at 3 standard errors.

| Change from v2 | persist vs failsafe | lead | dominates | downs | Blue | game / clock |
| --- | --- | --- | --- | --- | --- | --- |
| contest_drain 1.5 | +0.26 ± 0.09 | 0.35 | 0 | 0.65 | 48 % | 251 / 600 |
| contest_drain 1.5, link_timeout 45 | **+0.10 ± 0.09** | **0.14** | 0 | **0.68** | **54 %** | 266 / 600 |
| contest_drain 3 | +0.16 ± 0.09 | 0.16 | 0 | 1.15 | 46 % | 272 / 600 |
| contest_drain 3, win_score 900 | +0.20 ± 0.09 | 0.19 | 0 | 1.91 | 47 % | 382 / 600 |
| contest_drain 1.5, standard_battery 150, win_score 900 | +0.31 ± 0.09 | 0.23 | 0 | 1.08 | 50 % | 399 / 600 |
| contest_drain 1.5, link_timeout 45, win_score 900 (200 seeds) | +0.21 ± 0.06 | 0.16 | 1 | 1.31 | 52 % | 378 / 600 |
| contest_drain 1.5, link_timeout 45, win_score 1100 (200 seeds) | +0.20 ± 0.06 | 0.17 | 1 | 1.67 | 52 % | 448 / 600 |

Reading it:

- A higher contest drain and a longer link timeout both take the shine off "fly on": a contested
  circle now costs battery, and the failsafe fires only on a stall.
- **No setting meets every target.** Making the games longer (a higher win score) makes persist
  dominant again, because a drone has more time to win the long fight.
- Fixed policies are the wrong judge of the goal. The aim is that the information on the drone
  matters, and a fixed policy cannot read it. The next step is the `Edge` policies; these numbers
  say only that no fixed policy is a landslide under `contest_drain 1.5, link_timeout 45`.

## Adopted (your choice): contest_drain 1.5, link_timeout 45, 600 points

`Rules::default()` is now this. 200 seeds, 400 games a pair:

| Check | Result | Target |
| --- | --- | --- |
| persist against failsafe | +0.11 ± 0.06 | < +0.30: met |
| best fixed policy's lead | 0.11 | < 0.15: met |
| policies that beat every other at 3 standard errors | 0 | 0: met |
| drones down, per team and game | 0.68 | 0.5 to 3: met |
| Blue wins, default against default | 53 % | 45 to 55 %: met |
| a game lasts | 270 s of 600 (45 %) | at least 60 %: **not met** |

Not measured: headroom of an on-board policy over the best fixed one, and the value of its
information (criteria 3 and 4). That needs the `Edge` policies. Not yet built either: the
turtle-captain check and P(win | first to hold the spire).
