# Plan: advisors that think fast and slow, and read each other

A Towers commander has advisors beside it (`plans/actors.md`). Today each one looks at the team's
map on its own and proposes orders; the commander takes the best one that gains enough. This
plan gives a team a mixture of **fast thinking** (tactics, every few seconds, on the map as it is
now) and **slow thinking** (strategy, over minutes), and lets the advisors **see each other's
outputs** on a shared board.

## What the owner decided (2026-11-24)

- Fast and slow thinking, and advisors that see each other's outputs.
- The slow thinker is a **small local model**, not Claude: it runs on the machine, costs nothing
  per game and sends the game nowhere.
- Slow thinkers do not give orders. They post intents, assessments and critiques; orders come
  from fast thinkers' proposals and the commander's own loop.
- The idea is measured first with a slow stand-in that costs nothing (a deep lookahead), then
  with the local model.
- A person at a seat sees the whole board: proposals, intents, assessments and critiques.

## Why slow thinking must not give orders

Late advice loses its value fast. A lookahead's proposal taken 2 s after the map it was made
from gained +0.03 against no advisor; 5 s late, -0.09; 30 s late, -0.84 (`plans/actors.md`, the
threshold test of PR #138). A model that takes 10 to 30 s to answer would propose moves for a
map that is gone. What stays true for minutes is the shape of a plan and what the enemy is
like: which towers to fight for, which to give up, where the enemy masses, what it does when it
loses a tower. That is what slow thinking is for; the fast thinkers turn it into orders on the
map of the moment.

## What exists

| Piece | Where | What it does now |
| --- | --- | --- |
| `Advise` | `recreation_towers::advice` | `advise(view, hand) -> Vec<Proposal>`: one order for one drone, its gain, the map's time, how long it holds, why |
| `Lookahead` | `recreation_towers::advice`, `lookahead` | The fast advisor: every 5 s it plays 45 s forward in a small model and proposes the one change that gains most |
| `Commander` | `recreation_towers::commander` | Takes the best proposal that gains over `ADVICE_MARGIN` (5); an advised order is kept, the defence does not take it back |
| Seat side | `recreation_towers_ground::advisors` | Runs advisors on threads or as processes (`{"Look":{view,hand}}` → a list of proposals); `towers_advisor`, the Claude advisor (`claude.rs`, never run against real `claude`) |
| App | `teleoperation_command` | The ADVISOR rows over the battlefield: proposals, accepted with a click or Y |
| Arena | `cognition_towers` | `Roster::with_advisor(team, advisor, delay)`: an advisor's answer reaches the commander `delay` s of game time late; examples `advice`, `lookahead`, `margin` |

## The design

### The board

Each team has one board. Every advisor posts entries to it, and at each look an advisor is
shown the entries that have arrived since its last look (and those still in force). An entry:

| Field | Meaning |
| --- | --- |
| `id` | Unique in the game |
| `author` | The advisor's name (`lookahead`, `deep`, `local:qwen3.5`) |
| `made` | The game time of the map it was made from |
| `until` | The game time after which it no longer holds |
| `confidence` | 0 to 1, the author's |
| `kind` | One of the four below |

The four kinds:

- **Proposal**: what there is today (an order for a drone, its queue, its gain, why). Short-lived.
  Fast thinkers post them.
- **Intent**: a strategy for the next 30 to 120 s, in a fixed form a program can use, with words
  for a person: for each tower, *take*, *hold*, *leave* or *no view*, and a weight; optionally a
  drone kept for a role (the heavy to the spire, a scout). Slow thinkers post them. One intent per
  author is in force at a time; a new one replaces the old.
- **Assessment**: what the author believes about the enemy, in a fixed form with words: where it
  masses (a tower or a lane, with its weight), what it is about to do, how it plays (gives up
  contested towers, raids bases). Slow thinkers post them; a fast thinker may too.
- **Critique**: *for* or *against* another entry (by its id), with a reason and a confidence. Any
  advisor may post one; a critique of a critique is not taken.

### Fast thinkers follow the intent

A fast thinker reads the intent in force and works within it. The lookahead's search scores an
order that follows the intent with a bonus (its weight times a tuning factor), searches the
intent's *take* towers first, and leaves *leave* towers out unless a move there gains a lot.
Its proposals say which intent they follow. A fast thinker that sees an assessment may use it in
its model of the enemy (the lookahead's `Belief`).

### Slow thinkers reflect on the fast ones

A slow thinker is shown what happened since its last look: the proposals made, which the
commander took, the towers won and lost, and its own last intent. It answers with a new intent,
an assessment, or critiques. It does not wait to be asked: it looks when it is ready, and its
answer is posted when it comes (seconds or tens of seconds later), with `made` the time of the
map it looked at.

### The commander stays the arbiter

The commander's loop does not change in kind: it gives orders by its own rules and takes the
best proposal over its margin. The board changes what a proposal is worth:

- **Critiques**: each *for* adds, each *against* takes off, its confidence times the critic's
  weight times a factor.
- **Track record**: each advisor has a weight, from how its taken proposals did against what it
  said they would gain (points over the proposal's horizon). An advisor whose proposals do not
  gain what it says is listened to less. A slow thinker's weight comes from how the games went
  while its intents were followed.

Advisors that read each other can talk each other into the same mistake; the track record and
the commander's own loop are what keep that in check.

### The wire

The process protocol stays compatible. `{"Look":{view,hand,board}}` carries the new entries;
the answer is a list of entries. A list of plain proposals (what `towers_advisor` answers today)
is still read as proposals. `Start` says which kinds the advisor posts.

### The local model

A slow advisor in its own process, talking to a local model server: Ollama on this machine,
`http://127.0.0.1:11434`, with `qwen3.5` (9.7B, Q4, 6.6 GB), which fits beside a 10-drone SITL
game in 32 GB. `qwen3.5:27b` (17 GB) and an MLX Qwen3.5-35B-A3B are on the machine too, for when
nothing else runs. The advisor shows the model a short text of the map (towers, holders, the
drones it knows of, the score, the time), the board since its last look and what happened, and
asks for one JSON object in a fixed schema (intent, assessment, critiques). It checks the answer
against the schema and the map (a tower that exists, a weight in range) and posts what is valid.
Tests never call a model: they use a stub server that answers fixed JSON, as the Claude
advisor's tests do.

## Steps

Each step is measured in the arena on fresh seeds with a paired comparison, as the threshold of
PR #138 was, and keeps the arena's results identical where it should change nothing.

1. **The board.** Entries, the board, and `Advise` shown the board and answering entries.
   Proposals go on the board; the commander reads proposals from it. Nothing else changes:
   the arena gives the same games as before.
2. **Intents, with a slow stand-in.** The `Intent` entry; the lookahead follows the intent in
   force. A `Deep` advisor: a lookahead with a long horizon and few looks, which posts only
   intents, run with a delay of 20 s. Measure: lookahead alone, against lookahead with `Deep`'s
   intents, and `Deep`'s intents with no delay (what lateness costs a slow thinker).
   *Results (2026-11-26, development seeds 1 to 200, 400 games a row; leads, not confirmed on
   fresh seeds).* Margin of lookahead + `Deep` against the lookahead alone, by how late `Deep`'s
   intents reach the board:

   | Delay | FOLLOW 10, LEAVE 20 | FOLLOW 10, LEAVE 0 | FOLLOW 0, LEAVE 20 | FOLLOW 3, LEAVE 5 |
   |---|---|---|---|---|
   | 0 s | +0.09 ± 0.06 | +0.12 ± 0.06 | +0.05 ± 0.06 | -0.02 ± 0.06 |
   | 5 s | | +0.07 ± 0.06 | | |
   | 10 s | | -0.06 ± 0.06 | | |
   | 20 s | -0.06 ± 0.06 | -0.05 ± 0.06 | -0.04 ± 0.06 | -0.01 ± 0.06 |

   With following off (FOLLOW 0, LEAVE 0) the games are the lookahead's alone, to the game
   (+0.00). The local model answers a short summary of the map in about 9 s (`qwen3.5` on this
   Mac, after a 4 s first load), which is where `Deep`'s intents are worth nothing. What `Deep`
   posts is still a judgement on the commander's plan of the moment (what one more drone at a
   tower would gain *now*), and it goes stale like a proposal. An intent that is to survive 10 to
   20 s needs to say something that stays true that long.

3. **Critiques and track records.** Critiques change a proposal's worth; each advisor's weight
   follows its record. Measure against step 2.
4. **Assessments.** The `Assessment` entry; the lookahead's belief uses it. Measure.
5. **The seat and the app.** The board on the wire, compatible with today's advisors. The app
   shows the whole board to a person: proposals (as now, to accept), the intent in force,
   assessments and critiques, each with its author and how long it holds.
6. **The local model.** The local advisor (Ollama, `qwen3.5`), its prompt, schema and checks,
   with a stub server in tests. In the arena its time to answer becomes its delay in game time.
   Measure against `Deep` as the slow thinker, and with both.
7. **SITL.** The board in a full game on ArduPlane SITL, with the local model beside it.

## Not settled

- The intent's form: per tower is the start; whether drones' roles are needed is for step 2 to
  show.
- Whether a slow thinker remembers across games (how this enemy played last time).
- The model and its size: `qwen3.5` 9.7B first; the 27B or the MLX MoE when the machine is free.
- How the track record treats an advisor that is right but rarely taken.
