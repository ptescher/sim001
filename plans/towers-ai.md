# Improving the Towers AI with the game tools

How the computer commander is improved now that games can be played, recorded and read in bulk
(`plans/game-tooling.md`): look at what the records say decides games, make a guess a tuning
number, and play it head to head against the commander as it is.

## The method

- **Measure head to head, both seats.** `towers_games --games 1000 --swap` plays each seed twice,
  the sides changed over: 2000 games, a standard error of about 22 wins (1.1 %).
- **A side can carry a tuning:** `--blue plain+contest_margin=0`, any of the commander's tuning
  numbers, and `+advice_margin=POINTS`. A control (the defaults written out) plays level, 1000 to
  1000.
- **Tune on one range of seeds, check on another.** The numbers below were found on seeds 20001 to
  21000 and checked on 40001 to 41000.
- **Read the records, not only the score.** The questions below were answered by small scripts over
  the JSON-lines records (`Tick`, `Taken`, `Spotted`, `Board`, `Took`).

## What the records showed (1200 games, 2026-11-27)

- **The first capture decides most games.** The side that takes the first tower wins 71 % of
  games. Two first captures in three are the centre spire, at about 68 s.
- **Being seen is mostly a symptom.** The side the enemy sees more loses (98 % when the gap is 20
  points or more), but the side seen less before the first capture wins only 54 %: a side that is
  losing flies into the enemy's towers, and towers see.
- **The centre is given away, not fought for.** In 711 of 788 games where the centre fell first,
  the losing side had no drone in its circle when it fell.
- **The light drone never fought.** It was in a circle low only 11 % of its flying time (the others
  about 28 %) and was sent home 29 % of it. The commander sends a drone home when its battery falls
  to the bingo, the flight home and a margin of 15 s; in a contested circle the margin was counted
  at the contested drain (5.5 times the plain rate against a heavy and a standard), 82 of the light's
  150. It went home the moment it saw an enemy in the circle.
- **One drone a game goes at the enemy's home tower**, which is never taken (the enemy charges
  beside it).

## The measurements

Each variant against the commander as it was (both sides with no advisor unless named), 2000 games:

| Variant | Won |
| --- | --- |
| control (the defaults written out) | 1000 (50 %) |
| contested margin 5 s | 1368 (68 %) |
| **contested margin 0 s** | **1556 (78 %)** |
| no drone sent at the enemy's home tower | 1018 (51 %) |
| contested margin 5 s and no drone at the enemy's home tower | 1151 (58 %) |

The fort attacker earns its keep: without it, the enemy's defenders stay at the middle towers.
Drones down a game did not change (0.1 a side; flat batteries 139 against 130).

With the contested margin at 0, **the lookahead became a handicap**: the commander with it lost to
the commander without it, 726 to 1274. Its taken proposals (14 a game) mostly redirect drones in
flight (83 %), few pull a drone out of a contested circle (4 %). Its small model has no batteries:
nobody goes home in its 45 s, so it cannot see that a contest is won by staying. The bar a proposal
must clear (`ADVICE_MARGIN`, 5 points) was too low:

| Advice margin (points) | Lookahead against none, both at margin 0 |
| --- | --- |
| 5 | 726 (36 %) |
| 10 | 833 (42 %) |
| 20 | 1193 (60 %) |
| 30 | 1311 (66 %) |
| **40** | **1430 (72 %)** |
| 50 | 1362 (68 %) |
| 60 | 1317 (66 %) |
| 80 and more | 1000 (50 %: nothing is taken) |

The bar was too low before the margin change as well: on the old margin, 40 beat 5 by 1327 to 672.

**Checked on fresh seeds (40001 to 41000):** the commander with the lookahead, the contested
margin 0 and the advice margin 40 beat the commander with the lookahead as it was by **1570 to
429 (79 %)**. These are now the defaults (`CONTEST_MARGIN` 0, `ADVICE_MARGIN` 40).

## Not settled

- **The seat balance moved.** With both sides the new commander, the Blue seat wins 527 of 1000
  (about 1.7 standard errors); with both the plain commander on the new margin, 556 of 1000. The
  war of batteries at the centre may favour whichever team's heavy comes down first.
- **The lookahead's model has no batteries.** A model that sends drones home when their batteries
  run low (ours, and the enemy's as guessed) could see what a contest is worth, and its small gains
  could be trusted again.
- **SITL.** The arena's results have not been played on SITL yet; the battery is the game's, so the
  rule is the same there, but the flight is not.
- **The light drone** still waits at 87 to 91 m (its cruise height) and takes about 20 s to come
  down into a circle.
