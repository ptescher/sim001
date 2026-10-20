# Expectation

Acceptance testing where the behaviour itself is the source of truth.

Features are written as **expectations**: given some state, when something happens, then
the system ends up in some state. Every step uses a shared **vocabulary** of states and
events, so expectations can be compared mechanically. The domain uses that to:

1. find expectations that **conflict** (both apply, and they promise incompatible things),
   and let you mark one as an exception to the other with `Overrides:`. Overrides must form
   a DAG.
2. arrange everything into a **flow DAG** from the starting state. That shows what leads to
   what and gives each expectation a shortest route from the start.
3. compile **user stories** into **acceptance tests**. A story only describes the journey;
   at each step the expectations it passes through supply the checks. A story that relies on
   unspecified behaviour, or contradicts an expectation, is rejected before anything runs.
4. **run** those tests against the system under test, capturing a **screenshot** at every
   step.
5. write **documentation** from the stories, results and screenshots: a manual, where
   stories are step-by-step tutorials and features are topic pages, with the test output
   tucked away in "How this is tested" blocks and a test report.

```
.expect files ─language─▶ Suite ─graph─▶ conflicts · override DAG · flow DAG
                            │                         │
                            └────────story────────────┴─▶ plans (acceptance tests)
                                                             │
                                          harness + driver ──┴─▶ report + screenshots ─docs─▶ HTML + Markdown
```

## Modules

| Crate | Path | Role |
| --- | --- | --- |
| `expectation_model` | [model/](model) | Vocabulary, the condition algebra, expectations and stories as plain data |
| `expectation_language` | [language/](language) | The `.expect` format: parsing, and resolving steps against the vocabulary |
| `expectation_graph` | [graph/](graph) | Conflicts, override precedence, the flow DAG, Mermaid and DOT export |
| `expectation_story` | [story/](story) | Stories → acceptance-test plans, coverage, results and verdicts |
| `expectation_docs` | [docs/](docs) | The manual, as an HTML site and Markdown, from all of the above |
| `expectation_service` | [service/](service) | The external things a run needs (a flight controller, a database, a container): started once, stopped after, and "not supported yet" with how to get them when they can't be had. No dependencies |
| `expectation_harness` | [harness/](harness) | The `Driver` trait, closure `Bindings`, the runner, and the `Acceptance` pipeline |
| `expectation_cli` | [cli/](cli) | The `expectation` command: `check`, `graph`, `plan`, `docs` |

There are no external dependencies.

## External services

Tests sometimes need something that is not part of the system under test: ArduPilot to fly a
plane, a database, a container. The harness treats that as a **service**, so no test, driver or
script has to start it:

```rust
let mut services = Services::new()
    .with(simulation_sitl::service::ardupilot(root, "t1s-vtol", &work))   // native build, else Docker
    .with(Docker::new("database", "postgres:16").port(5432, 55432)        // a database is a container
        .env("POSTGRES_PASSWORD", "test").ready(Ready::Tcp(55432)));
let mut driver = Bindings::new(world).needs(&["ardupilot", "database"]).fresh(&["ardupilot"]);
let report = acceptance.run_with(&mut driver, &RunOptions::default(), &mut services);
```

- **Needs.** A driver says what it needs (`Driver::needs`). Before a plan the runner makes sure
  each is up: one that is up stays, one that died is started again. All stop when `services`
  is dropped, the last started first.
- **Fresh.** What a test leaves behind (a flight controller armed by the last one) is cleared
  by naming the service in `Driver::fresh`: it is started afresh before each test that runs.
- **Not supported yet, not failed.** A service that can't run here (no Docker, no build) makes
  its plans "not supported yet", and the reason says how to get it. Plans the driver can't run
  anyway start nothing.
- **Providers.** `Command` (a program, ready when a port answers), `Docker` (a container, its
  image built from a directory when it isn't there, Docker Desktop woken on a Mac), `Present`
  (a build or a file that must be there), and `Fallback` (the first of several ways that is
  available: a native build, else a container). A port that something else already holds is an
  error, not a ready service: what answers there is a leftover of an earlier run.

## Platforms

One suite can describe a system that exists on several platforms (the desktop console and the
iOS app do the same job). Everything written plainly is true on every platform. Where one
differs, the suite says so, and each platform reads the suite as its own:

```gherkin
Vocabulary:
  # The first is the default: plain lines are about it.
  Platform: desktop | iOS

  Event: the operator arms the vehicle
    Say: press ARM
    Say[iOS]: tap ARM           # how the docs tell an iOS reader to do it
  State: the home position is set | not set
    Shows[iOS]: set: the HOME badge turns green   # replaces the plain Shows lines

  Phrase: the console is connected to a plane on the ground
    Given the map tool is pan
  Phrase[iOS]: the console is connected to a plane on the ground
    Given the link is connected                    # a start that iOS can reach

Section: Planning a mission
  On: desktop                   # also under a Feature, Expectation or Story
```

- `Say[P]:`, `Set[P]:` and `Shows[P]:` replace the plain line for platform `P`.
- `Phrase[P]:` replaces the plain phrase of the same name for `P`.
- `On: desktop, iOS` says the section (the whole directory), feature, expectation or story
  is only about those platforms. Use it for what a platform does not have. An exception to
  an expectation that is not on a platform is dropped there.
- Tag only the platforms that differ from the default. A name that is not declared is an error.

`expectation check` checks every platform. A driver reads the suite for its platform
(`Acceptance::load_for(dir, "iOS")`) and runs its tests; its results are its own
(`results/gcs@iOS.tsv`), apart from those of runners that are the same everywhere (`sim.tsv`).
`expectation docs` writes one manual per platform, the default at the root and the others
under `platforms/<name>/`, and every HTML page has a Platform dropdown that opens the same page
in the other manual.

## The language

A suite is every `*.expect` file under a directory. The format is line oriented like
Gherkin. A line's first word decides what it is, a line starting with `#` is a comment, and
indentation is only for people.

```gherkin
Vocabulary:
  State: the aircraft is in hand | launching | flying | crashed
  State: the throttle is 0..100 %
  State: the flight mode is MANUAL | ANGLE | ACRO
    # How the docs say to change it:
    Set: press M until the flight mode is {value}
  Event: the pilot throws the plane
    # How the docs say to do it:
    Say: press Space to throw the plane

  # A setup, in one line:
  Phrase: the sim has just started
    Say: start the simulator with cargo run --release
    Given the aircraft is in hand
    And the throttle is 0 %
    And the flight mode is MANUAL

  Phrase: the pilot throws the plane with the throttle
    Say: open the throttle past 55 % and hold it there
    When the throttle is 80 %

Background:
  Given the sim has just started

Feature: Hand launch
  # Free text before the first step is a description:
  The plane starts in your hand.

  Expectation: Throttle up throws the plane
    Given the aircraft is in hand
    When the throttle is above 55 %
    Then the aircraft is flying

  Expectation: Space throws the plane
    Given the aircraft is in hand
    When the pilot throws the plane
    Then the aircraft is flying

Story: Your first throw
  As a new pilot
  I want to throw the plane by opening the throttle
  So that launching feels like the real thing

  # The tutorial's introduction:
  The T1-S is hand-launched, and the sim works the same way.

  Scenario: Throttle up
    When the pilot throws the plane with the throttle
    Then the aircraft is flying
```

**Suite and sections.** An optional `Suite: <name>` line names the suite; otherwise it's
named after its directory. Each directory under the suite is a **section**, a nested set of
features that share a setup. Its `Section: <name>` line (in any of its files) and the text
under it name and describe it. A leading `01-` in a directory name only orders it. A
section's `Background:` is its setup, written over its parent's, so everything in
`domains/teleoperation/console/planning/` starts where `domains/teleoperation/console/` does with the PLAN tool picked on top. Pages about a
section start from that setup and don't re-explain how to get there, the way a manual
assumes you've read the chapter before.

**Vocabulary.** `State: <subject> is <a> | <b> | …` declares a state with named values.
`State: <subject> is <min>..<max> [unit]` declares a number. Either can use `are`, `has` or
`have` instead of `is` ("the plan has 0..650 items"), and steps about it read the same way.
`Event: <phrase>` declares something that happens. Vocabularies from all files merge into one.

Name events for what the user means, not the button they press: "the pilot throws the plane",
not "the pilot presses Space". The button goes in a `Say:` line under the event, which is
what the docs tell the reader to do ("Press Space to throw the plane."). A `Set:` line under
a state says how to change it, with `{value}` for the value ("press M until the flight mode
is {value}"). Without them, the docs make do: "the operator clicks the map" becomes "Click the
map", and a state becomes "Set the throttle to 80 %".

A `Shows:` line under a state says what the reader sees when it has a value, in the screen's
own words: `Shows: crashed: the HUD says CRASHED`. Start the line with the value and a colon
(for a number, a value or comparison as steps write it: `at least 99 %:`), or put `{value}`
in it for any exact value (`Shows: the UPLOAD button reads UPLOAD {value}`). The docs use it
for every outcome, so "Then the aircraft is crashed" reads "The plane hits the ground, and
the HUD says CRASHED".

`Phrase: <name>` is a step of your own, made of other steps, for saying more with less. A
phrase of `Given`s is a setup ("the sim has just started"), usable in a Background, a Given
or a scenario's start. A phrase of `When`s is a sequence of actions ("the operator plans
three waypoints", one `When` per action); an expectation can use one that's a single action,
and a story step can use any. A phrase's `Say:` is how the docs put it. Phrases can use the
phrases declared before them.

**Steps.** A step is either an event or a fact `<subject> is [not] <predicate>`. The
predicate is one or more values (`in hand or flying`) or, for numbers, a comparison:
`80 %`, `above 55 %`, `below 10`, `at least 20`, `at most 40`, `between 20 and 40 %`. Matching
ignores case, extra spaces and a leading "the", and "1 item" matches a unit of items.
Anything outside the vocabulary is an error, with a "did you mean" hint. An expectation's
Then can also be relative: `the plan goes up by 1`, `goes down by 10 m`, `stays the same`.

**Glossary and links.** `Term: <name>` and the text under it define a word for the
glossary. Any text can link to a term, a section, a feature, an expectation or a story with
`[[its name]]`, and mark code with backticks. A `Docs: hidden` line in an expectation keeps
it off the topic page; it's still tested, and it's in the test report.

**Expectations** read `Given …` (the state before), `When …` (at most one event, plus any
states the outside world changes, such as the throttle) and `Then …` (the state after).
States the Then doesn't mention keep their values. An expectation without a `When` is an
**invariant**: it holds whenever its Given does. `Overrides: <name>` makes it an exception
to another expectation: where both apply, this one wins.

**Stories** have a narrative (the "As a …", "I want …", "So that …" lines), an introduction
(any other text, which opens the tutorial in the docs) and one or more `Scenario:`s. Steps
written straight under the story form a single scenario. A scenario may start with `Given`s,
which are arranged on top of the Background. After that, each `When` starts a step,
`And`/`But` add to it, and its `Then`s are what the story checks afterwards. A step that's a
phrase of several actions becomes several test steps but stays one numbered step in the docs.
Each top-level section is a system of its own: a story only goes through its own section's
expectations.

## How expectations are compared

A condition (a Given, a Then, a trigger) stands for a set of states. Everything below reduces
to three questions about those sets: do two overlap, does one imply the other, and what is
left after an action overwrites some states.

- **Outcome conflict.** Two expectations can be set off by the same action, their Givens
  overlap, and their Thens contradict. In the overlap the system can't do both. The error
  names the overlap ("both apply when the aircraft is in hand and the calibration wizard is
  open, and the throttle is above 55 %") and suggests which is the exception: the more
  specific one.
- **Invariant conflict.** An expectation leaves the system in a state where an invariant's
  Given holds but its Then can't, or the Background does.
- **Exceptions.** An `Overrides:` between the two, directly or through a chain, settles a
  conflict. Overrides must form a DAG, so a cycle is an error. An override that settles
  nothing gets a warning. Settled conflicts are listed in the docs as the suite's exceptions.
- **Relative Thens.** Two different relative Thens ("goes up by 1" and "goes up by 2") always
  disagree. A relative one and an absolute one disagree unless the shared situation pins down
  the one value where they meet.
- **Flow DAG.** Starting from each section's setup, and from each scenario's arranged start, a
  breadth-first search applies every transition that can fire. It only uses expectations from
  the same top-level section, so the ground station's never apply to the sim. An expectation's
  *level* is the fewest steps before it applies. Its parents are the expectations that take
  the last step on those shortest routes. Levels only increase along edges, so the result is a
  DAG even though the behaviour loops (throw, crash, reset, throw again). An expectation the
  search never reaches gets a warning, because nothing can test it.
- **Ways into a section.** For each section, a goal-directed search finds the chain of
  expectations that gets from its parent's setup to its own: "press PLAN", or "plan three
  waypoints, upload, press AUTO, arm, throw" for flying a mission. That chain, with any step it works
  without dropped, is the "Before you begin" of the section's page. If there's none, it's a
  warning, because then nothing explains how to get there.

## From stories to acceptance tests

Planning walks each scenario through the expectations:

- An expectation **fires** when the current state implies its Given and the action implies
  its trigger. In a compound action ("When the throttle is 0 % and the pilot presses
  reset") the other changes happen first. When both an exception and the expectation it
  overrides fire, only the exception's Then applies.
- Each step's checks are the Thens of what fired (and of invariants now in force), plus the
  story's own Thens. The story never has to restate what the expectations already promise.
- It's an **error** if a step *might* set off an expectation but the story doesn't pin it
  down (`unclear-step`), if an event happens and nothing is expected (`unspecified`), or if
  a story's Then contradicts the expectations (`contradicts`). A Then that no expectation
  promises is a warning (`unpromised`), and it's checked anyway.
- Expectations no story exercises get a warning with a route that would reach them.
  `Acceptance::with_coverage()` and `expectation plan --coverage` add generated tests that
  follow those routes, starting from the expectation's own section setup.
- A test starts from its section's setup, with its scenario's Givens written over it.

## Running against the system under test

The system plugs in through the `Driver` trait, or through `Bindings` for one closure per
vocabulary term:

```rust
let mut driver = Bindings::new(World::default())
    .observe("the aircraft", |w| w.phase_name())
    .set("the throttle", |w, v| w.throttle = v.as_f64().unwrap())
    .on("the pilot presses reset", World::reset)
    .on_settle(|w| w.run_for(1.0))                         // after each action
    .on_advance(|w| { w.run_for(0.1); Progress::Advanced }) // between looks
    .on_screenshot(|w| Screenshot::png(w.capture()));

Acceptance::load("expectations")?
    .with_coverage()
    .assert_passes(&mut driver, &RunOptions { docs_dir: Some("docs/behaviour".into()), ..Default::default() });
```

For each test, the runner resets the driver and arranges the start (a value inside each
constraint: the first allowed name, or the middle of a range). Then for each step it acts,
lets the system **settle**, and observes until every check holds at once or its patience
runs out. Settling is what makes "Then the aircraft is in hand" mean that it stays in hand,
not just that it was in hand an instant before the throw. Polling is what lets "Then the
aircraft is flying" wait for the throw. Steps after a failure are skipped. Before a test
starts, the runner checks that the driver can do everything the plan needs. If it can't, the
test is **not supported yet**. That isn't a failure: the system, or its driver, just doesn't
do it yet, and its steps get a placeholder instead of a screenshot.

`Acceptance::publish(dir, runner, &report)` saves a run's results as
`dir/results/<runner>.tsv`, merges them with the results other runners saved there (the sim
and, one day, the ground station each run their own part), and writes the docs from the
merged report. `expectation docs` rebuilds the docs from saved results without running
anything.

## Docs

`Docs::write` produces a manual, as an HTML site in `site/` and as Markdown that GitHub
renders. Its navigation follows the sections. It reads like a project's user docs, not a
test report:

- **Tutorials**, one per story scenario: the story's introduction, **Before you begin** (the
  setup, in the specs' own words), then numbered steps. Each says what to do (from the
  `Say:`s and `Set:`s), what you'll see (the story's Thens, or what the expectations promise)
  and the screenshot the test took there.
- **Topic pages**, one per feature: its description, then each expectation as a sentence
  ("If the aircraft is in hand, press Space to throw the plane. The plane leaves your hand and
  climbs away."), its
  exceptions as notes, a screenshot and the tutorials that show it.
- **Section pages**: the description, Before you begin (the setup, or the steps that get
  there from the parent section), the topics in it and its tutorials as cards.
- **Reference**: the glossary of `Term:`s, the DAG map, the theme, and the **test report**:
  every test and its result, a page per test with each step, check and screenshot, and the
  test vocabulary (each state, event and phrase, and how the docs word it).

The test details (each expectation's Gherkin, what it follows and leads to, its verdict and
the tests that cover it) are on every page too, collapsed under "How this is tested".
Generated coverage tests only appear in the test report. A step without a screenshot shows a
"Not supported yet" or "No screenshot" placeholder.

The Markdown has the same pages: `README.md` (the overview), `tutorials/`, `features/`,
`glossary.md`, `tests.md` (the report, with a Mermaid map of the flow DAG), `tests/` and
`vocabulary.md`. Pages read in order: each section, then its tutorials, its topics and its
subsections. Each page's footer names the spec file to edit to change it.

**The outline.** A suite can have an `outline.txt` beside its sections. It groups the
top-level sections into **parts** (the navigation's labels and the README's contents), sets
the order the reader meets them in (and so the previous and next links), and places
**hand-written pages** among them: Markdown files in `manual/` (install, controls,
reference, troubleshooting) with `[[links]]` into the generated pages and `[desktop]` to keep a
page out of another platform's manual. The sections themselves stay where they are, because
their setups inherit from their parents. A top-level section no part names goes into a last
part, "More", with a warning. Without an `outline.txt` the manual follows the sections.

## Writing specs

The project skill [writing-specs](../../.claude/skills/writing-specs/SKILL.md) walks through
the whole lifecycle: write high-level expectations in the user's words, check the graph,
write stories as tutorials, generate the docs, have a subagent audit them as a reader would,
and loop on the specs until the docs read well. Reader-facing text is written in Simplified
Technical English (ASD-STE100): full STE preferred, and at least 80 % of sentences.

## The `expectation` command

```bash
cargo run -p expectation_cli -- check <dir>                # diagnostics; exits 1 on errors
cargo run -p expectation_cli -- graph <dir> [--dot]        # the flow DAG as Mermaid or DOT
cargo run -p expectation_cli -- plan  <dir> [--coverage]   # the acceptance tests, step by step
cargo run -p expectation_cli -- docs  <dir> <out-dir>      # docs from the results saved in <out-dir>/results
```

## Example

[harness/examples/hand_launch](harness/examples/hand_launch) is a suite for the sim's hand
launch, F2 calibration wizard and flight modes. It runs against a toy model that follows
`src/sim.rs` and `src/input.rs`: the throttle has to be held above 55 % for 0.6 s, and the
wizard keeps the sticks from flying the plane. It has two exceptions, three stories and four
generated tests, with SVG screenshots.

```bash
cargo run -p expectation_harness --example hand_launch               # docs in target/expectation-docs/hand_launch
cargo run -p expectation_harness --example hand_launch -- out --bug  # a seeded defect, to see a failure
cargo test -p expectation_harness                                    # the same as tests
```

## In this project

The project's suite is [expectations/](../../expectations): `01-sim/` for the simulator and
`02-gcs/` for the ground control station, whose planning and flying sections follow Mission
Planner's docs, and `glossary.expect` for the glossary. The sim's driver is
[src/acceptance.rs](../../src/acceptance.rs). `cargo run --release -- --expect --docs docs` runs
the suite in the sim window and writes [docs/](../../docs/README.md) and
[docs/site/](../../docs/site/index.html). There's no driver for the ground station yet, so its tests are
not supported yet. The runner works on its own thread.
Each driver call becomes a request served between frames. Events are real key presses,
injected just before the input system reads the keyboard. The throttle and pitch stick are
held the way a radio holds them. Settling and polling count real time, and screenshots come from
Bevy's `Screenshot`, scaled down and saved as JPEG.
