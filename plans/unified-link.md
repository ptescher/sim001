# Plan: one link, in the sim and in the field

The game server of Towers is the world, and the commander app is the ground station. A game in
the field should use the same app, the same commander and advisor, the same plan protocol and
the same executor on board. Only the world changes: real drones, real air, a real radio.

So in the sim, an order should go the way it goes in the field: from the app, as MAVLink, through
a simulated radio, over simulated air, to a simulated flight controller (ArduPlane SITL in
AGENT mode). And the expectations should check that the two systems behave the same, not just
that each one behaves.

## Where Towers stands today

```text
 commander app ──seat (JSON, TCP 5955)──▶ towers_server
                 Order, TeamView, Advice     ├─ referee: Game (towers, score, fog, jam, mesh)
                                             ├─ World: orders, standing orders, link lost → home
                                             ├─ Ground: MAVLink to each SITL, loopback TCP
                                             └─ physics + SITL fleet
```

- **The app never speaks to a drone.** It speaks the seat protocol
  (`recreation_towers::seat`): game words such as `Order`, `TeamView`, `Advice`. The server
  turns an order into a plan.
- **The radio is perfect.** `towers_server/src/ground.rs` holds a `cognition_agent::link::Link`
  to each SITL on `127.0.0.1`. No frame is late or lost.
- **Radio shadows and the mesh are rules, not radio.** `Game::jam` sets `jammed[i]`. The server
  then refuses the order (`World::command`), keeps the drone out of the fog, and ticks its
  standing order. Nothing on the wire changes. In the field there is no `jammed[]`, so none of
  this code would run.
- **The fog is made from the truth.** The server knows where everything is and cuts the team's
  map from it. In the field, a team knows only the telemetry that arrived.
- **What a drone does out of contact is decided on the ground.** `Instruction::tick` (standing
  orders, and the flight home after `link_timeout`) runs in the server. A real drone out of
  contact cannot be told anything, so this has to run on board.
- **A realistic link already exists, but not here.** `communication_air` (range and losses),
  `communication_radio` (`Downlink`, `Relay`, the narrow rate-limited link) carry the station
  sim's traffic. Towers does not use them.

## The split

```text
 GROUND STATION (the same in sim and field)          │ THE WORLD
                                                      │
 commander app                                        │ sim:   physics + SITL fleet
   Ground: AGENT mode, plans, traffic                 │        simulated air (shadows, mesh,
   commander / advisor                                │        range, rate, loss)
   team map: fog from the telemetry heard             │        referee (score)
        │                                             │
        │ MAVLink                                     │ field: real drones, real air
   Transport ─┬─ TCP to the simulated ground radio ───┼──▶ simulated air ─▶ SITL
              └─ serial to a real ground radio ───────┼──▶ real air ─▶ ArduPlane
                                                      │
   umpire channel (score, clock, outcome) ◀───────────┤ sim: towers_server   field: umpire service
```

1. **The ground moves into the station.** `towers_server/src/ground.rs` becomes a crate that the
   commander app runs (and the computer commander, for a computer team). It sends MAVLink
   through a `Transport`: TCP to the sim's ground radio, or serial to a real radio. This is the
   arrangement the console already has with `--radio 5750` and `communication_radio::serve`.
2. **The world simulates the air.** Between the ground radio and each drone's MAVLink port sits
   a proxy built on `communication_air` and `communication_radio`. Range, rate and loss apply to
   every frame. A radio shadow drops frames, and the mesh forwards them through a friend (hop by
   hop, at most `mesh_hops`). "Out of contact" then means the ground hears nothing and its frames
   do not arrive, which is also what it means in the field.
3. **The team map is made from what was heard.** Each drone reports what it sees (its own
   position, and enemies in its sensor range) in its telemetry. The station builds the fog from
   those reports, with their ages. A drone out of contact reports nothing, so its corner of the
   map goes stale on its own.
4. **What a drone does out of contact runs on board.** The plan carries the standing order, and
   the executor (or an ArduPilot failsafe) flies it after silence, and flies home after
   `link_timeout`. The ground only shows the order it last sent.
5. **The seat protocol shrinks to an umpire channel.** The score, the clock, the outcome, and a
   spectator's view of the whole game. In the sim the server is the umpire; in the field a
   service that reads the drones' positions (Remote ID, or logs) scores the game.

## The field's radio shadow

An enemy tower in the field is not a jammer. It is a game rule, so the field needs something
that drops frames in the same places. The choice:

- **A game filter in the link (recommended).** The ground radio (or the drone) knows the towers,
  who holds them and where each drone is, and drops the frames that the rule says are lost. The
  rule then lives in the link layer on both platforms, and one expectation covers both.
- A real jammer, or real terrain. Honest, but the shadow would not match the rule, and the
  expectations would split.

## Holding both systems to one behaviour: Platforms

The expectation framework has [Platforms](https://github.com/ptescher/expectation/blob/main/book/src/platforms.md):
one suite that describes one system on several platforms. Everything written plainly is true on
every platform. A platform that differs says so with `Say[P]:`, `Set[P]:`, `Shows[P]:` or
`Phrase[P]:`, and `On:` keeps a section, feature, expectation or story to some platforms.
`expectation check` checks the suite as each platform reads it.

For Towers:

```gherkin
Vocabulary:
  Platform: sim | field

  State: the link of the first Blue drone is in contact | cut | lost
    Shows: cut: the list of drones shows NO LINK beside B1, with the time since the last contact
  State: the first Blue drone is near the Red home tower | ...
    Shows: the first Blue drone flies 50 m high, 100 m from the Red home tower
    Set[field]: fly B1 to the marker 100 m from the Red home tower
```

- **What is observed is what the station shows.** Each `Shows` line is about the commander app
  (or the station's state under it), never about the server's truth. Then the same observe
  binding runs on both platforms, and the expectation checks the system a person would use.
- **Only the setup differs.** `Set` and `Arrange` move the world: the sim puts a drone where the
  step says, the field flies it there. A step the field cannot stage (a tower held by Red, at
  will) is `can_arrange` false, and its tests are "not supported yet", not failures.
- **One driver per platform.**
  - `sim`: the commander app's driver (`towers_commander --expect`), on a server with
    simulated air. It loads the suite with `Acceptance::load_for(dir, "sim")`.
  - `field`: a bench rig — the app, a real ground radio and a real flight controller (or SITL
    behind a real radio hop). The radio and the flight controller are services in
    `Driver::needs`, so without the bench the tests are "not supported yet".
- **The results are the proof.** Each runner saves its own results (`results/<runner>@field.tsv`)
  and `Acceptance::publish` merges them. The merged results say, for each expectation, whether
  it passed on each platform: the matrix of where the two systems agree.
- **`On: sim` marks what has no field counterpart**: the spectator who sees the whole game, a
  game sped up ten times, the batteries switched off.
- **The rules stay the model.** `expectations/09-towers/rules` and its driver (the referee alone,
  `domains/recreation/towers/tests/acceptance.rs`) stay as they are: the game, with no radio.
  The `sim` and `field` platforms are held to the same expectations, through the link.

The same pattern fits the rest of the repo: the station (`gcs --station`) on the sim's TX15
stand-in and on a real TX15 is one system on two platforms.

## Steps

Each step is spec first (the `tdd` and `writing-specs` skills): the expectation, a driver that
can run it, a failure for the right reason, then the code.

1. **Platforms in the Towers spec.** Declare `Platform: sim | field` in `09-towers`, mark what is
   sim only with `On: sim`, and word the radio, mesh and standing-order `Shows` lines from the
   station's side. `expectation check` passes for both platforms; the field driver is a stub
   that supports nothing yet.
2. **Simulated air between the ground and the drones.** A MAVLink proxy per drone in the
   server, built on `communication_air` and `communication_radio`, that applies range, rate,
   loss, shadows and the mesh to frames. `Game::jam` stays the rule; the proxy enforces it.
   The radio and mesh expectations pass on `sim` with the order no longer gated in
   `World::command`.
3. **Out of contact on board.** The standing order and the flight home go into the plan and the
   executor (`cognition_plan`, `cognition_executor`). `Instruction::tick` leaves the server.
4. **The ground in the station.** `ground.rs` moves into a crate the commander app runs, behind
   a `Transport`. The app sends MAVLink through the sim's ground radio; the computer commanders
   do the same.
5. **The team map from telemetry.** Drones report what they see; the station builds the fog. The
   server's fog check (`towers_server/tests/acceptance.rs`, held against the spectator) becomes
   the check that the station's map never shows what was not heard.
6. **The umpire channel.** The seat protocol keeps only the score, the clock, the outcome and the
   spectator.
7. **The field bench.** A real ground radio on serial, a real flight controller on the bench, the
   field driver's services, and the radio-shadow filter in the link. The first expectations pass
   on `field`.

## Open questions

- The field's radio shadow: a filter in the link (recommended above), or something physical.
- Where the filter runs in the field: the ground radio, the drone, or both (a drone in a shadow
  must also stop hearing the ground).
- The field umpire's source of truth: Remote ID, the drones' own telemetry, or logs after the game.
- How a drone reports what it sees within the link's rate (`Downlink` is narrow).
