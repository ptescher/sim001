# Spec style

How to write `.expect` files that make good tests and good docs. The language itself is
described in the [expectation README](https://github.com/ptescher/expectation); this is how to use it well.

## The one test

From Cucumber's [Better Gherkin](https://cucumber.io/docs/bdd/better-gherkin/): write steps
that say *what* the user does and sees, not *how*. For every step ask: **would this wording
change if the UI changed?** If moving a button or rebinding a key would change it, the
wording belongs in a `Say:` or `Set:` line, and the step should name the intent.

| Imperative (avoid) | Declarative (write this) | `Say:` |
| --- | --- | --- |
| `When the pilot presses Space` | `When the pilot throws the plane` | `press Space to throw the plane` |
| `When the operator presses UPLOAD` | `When the operator uploads the plan` | `press UPLOAD` |
| `When the operator clicks the map` ×3 | `When the operator plans three waypoints` (a phrase) | `click the map at three points along the circuit` |
| eleven `Given`s in a Background | `Given the sim has just started` (a setup phrase) | `start the simulator with cargo run --release …` |

## Write in Simplified Technical English

Write all the reader-facing text in ASD-STE100, Simplified Technical English: descriptions,
`Say:`, `Set:` and `Shows:` lines, story introductions and `Term:` definitions. STE is the
controlled language of aerospace maintenance manuals, and it suits a flight simulator's docs
for the same reasons. It's easy to read, easy to translate, and hard to misread.

**Target: at least 80 % of sentences comply.** Full STE is the goal. Where a rule would make
a sentence harder to follow (a well-known product name, a quoted on-screen message, a term the
glossary defines), keep the clearer wording and count it against the 20 %.

The rules that matter most here:

| Rule | Write | Not |
| --- | --- | --- |
| Instructions are commands, one per sentence | "Press P. The sim stops." | "You'll want to pause by hitting P, which stops the sim." |
| Instructions are at most 20 words; descriptions at most 25 | Split long sentences | Chains of clauses joined with "and", "which" or semicolons |
| Put a condition before the instruction | "If the button reads ARM: NOT READY, wait." | "Wait if the button reads ARM: NOT READY." |
| Use the active voice | "UPLOAD writes the plan to the vehicle." | "The plan is written to the vehicle by UPLOAD." |
| Use simple tenses: present, past, future, imperative | "The plane climbs." | "The plane will be climbing." |
| Don't use -ing verb forms, except in technical names | "When you pan the map, …" | "Panning the map, …" |
| One word, one meaning; one thing, one word | "the plane" on every page | "the plane", "the aircraft" and "the vehicle" for the same thing |
| Keep the articles and "that" | "Press the button that reads ARM." | "Press button reading ARM." |
| No noun clusters longer than three words | "the altitude of the plan" | "plan altitude limit warning text" |
| No contractions | "do not", "it is" | "don't", "it's" |
| Name controls and screen text exactly | "Press CONFIRM ARM." | "Confirm the arming." |
| One topic per paragraph, at most six sentences | Short paragraphs | A description that covers three topics |

Gherkin steps (`Given`, `When`, `Then`) and names in the vocabulary are not reader text. They
follow the declarative rules above, not STE. A state's subject ("the plane") is a term of
the test vocabulary; use its `Shows:` lines to give the reader the words they see.

## How specs become docs

Knowing what the generator does with each line tells you what to write.

| In the spec | In the docs |
| --- | --- |
| `Section:` and the text under it | The section page's title and opening paragraphs |
| The section's `Background:` | Its "Before you begin". A setup phrase's `Say:` is shown as written; bare states are listed as "The plane is in hand." |
| `Feature:` and its description | A topic page and its opening paragraphs. The first paragraph is also its summary on the section page, so make it stand alone |
| `Expectation:` name | A heading on the topic page. Write it as the rule in plain words |
| An expectation's description | The paragraph under the heading: the why |
| An expectation's steps | A sentence or two, conditions first: "If the plane is in hand, press Space to throw the plane. The plane leaves your hand and climbs away." The outcome comes from the `Shows:` lines |
| `Docs: hidden` in an expectation | Left out of the topic page; still tested and in the test report. For checks a reader doesn't need |
| `Overrides:` | A note on both: "Except when the sim is paused: see …" |
| `Story:` name | The tutorial's title and its card. Name the reader's goal |
| The story's narrative (As a / I want / So that) | Only the test details |
| The story's other text | The tutorial's introduction |
| Each `When` of a scenario | A numbered step: what to do, from the `Say:`s |
| The `Then`s after it | What you'll see, from each state's `Shows:`. Without Thens, what the expectations promise |
| `Term:` and the text under it | An entry in the glossary |
| `[[name]]` in any text | A link to the section, topic, expectation, tutorial or glossary term with that name |
| `` `text` `` in any text | Code: commands, file names, keys typed as text |
| The step's screenshot | Taken by the test right after the step |

Instructions come from, in order: the event's or phrase's `Say:`; the state's `Set:` with
`{value}` filled in; the step's own wording with the actor dropped ("the operator clicks the
map" becomes "Click the map"); or, for numbers, "Set the throttle to 80 %". Events without an
actor ("the plane reaches a waypoint") become "Wait until the plane reaches a waypoint."

## Vocabulary

- **States are what the reader can see or set.** Values should be the words on the screen
  (`MANUAL | ANGLE | ACRO`, not `mode0 | mode1`). Keep the subject short and concrete: "the
  map tool", "the plan altitude".
- **Counts read as counts.** Declare them with `has`: `the plan has 0..650 items`. Then
  relative Thens read "the plan has 1 more item".
- **One state per thing.** Before adding a state, look in the glossary for one that already
  says it. Two sections can declare the same state; the declarations merge, and a `Set:` or
  `Say:` on either counts.
- **Every event a reader does gets a `Say:`.** Name the real key or button, checked in the
  code. Use the label exactly as the screen shows it (`CONFIRM ARM`, `AUTO: NO MISSION?`).
- **`Set:` for states the reader changes directly.** Phrase it so `{value}` fits every value:
  "press C until the camera is {value}". With a comparison ("above 55 %"), a "to {value}"
  in the template reads "above 55 %" without the "to".
- **A `Set:` for each value** when the actions differ: `Set: lost: switch off the TX15` and
  `Set: up: switch on the TX15`, not one "connect the TX15, or unplug it". A `Set:` with no
  value in front is for the other values.
- **A `Set:` is something the reader can do.** "Let the GPS have 9 satellites" is the test
  rig, not the reader. For what the plane or another person does, write "wait until the GPS
  of the plane has {value} satellites" or "ask the other pilot to …".

## Shows

A state's `Shows:` lines say what the reader sees when it has a value, in the words on the
screen. They're what turns "the plane is crashed" into "The plane hits the ground, and the
HUD says CRASHED". Give every state a story or expectation checks a `Shows:` line for each
value a reader would see.

```gherkin
State: the plane is in hand | flying | crashed
  Shows: flying: the plane leaves your hand and climbs away
  Shows: crashed: the plane hits the ground, and the HUD says CRASHED
State: the plan has 0..650 items
  Shows: the UPLOAD button reads UPLOAD {value}
State: the battery is 0..100 %
  Shows: at least 99 %: the HUD's BAT reading is back to about 100 %
```

Do not write `Shows: {value}: …`: `{value}` is not a value, so the docs print "18: the AS
reading is this speed". Put `{value}` in the sentence, with its unit: `Shows: the AS reading is
{value} m/s`. Start a line with the value and a colon for one value, or a number or comparison as steps
write it ("80 %", "at least 99 %"). A line with `{value}` and no value in front is for any
exact value. Say where to look: the HUD, a button, a line on the map.

## The glossary

Define every term a reader might not know with `Term:` in `expectations/glossary.expect`:
modes, protocols, views and the names of screens. Write each definition in STE, in two or
three sentences. Link to it from any text with `[[the term]]` the first time a page uses it.

## Phrases

- **A setup phrase** (`Given`s only) names a starting point: "the console is connected to a
  plane on the ground". Use one as each section's Background, and give it a `Say:` that a
  reader can follow: the command to run and what they'll see when it's ready.
- **An action phrase** (`When`s only) names an intent that takes several actions: "the
  operator plans three waypoints". One `When` per action; `And` joins changes into one
  action ("When the throttle is 0 % / And the pilot resets the plane"). In a story it's one
  numbered step; the test still checks after each action.
- Phrases are declared in a `Vocabulary:` block and can only use phrases declared before them.
- An expectation can use a phrase that's a single action. One of several is an error
  (`bad-phrase`): an expectation is one behaviour.

## Expectations

- **One behaviour each.** If the name needs "and", it's probably two.
- **Name it as the rule**: "Reset picks the plane back up", "UPLOAD won't send an empty plan".
  Keys and button names in a heading are fine; they're what readers look for.
- **The Given is only what matters.** Everything else keeps whatever value it has. A Given
  that pins too much makes the expectation look narrower than it is, and hides conflicts.
- **Say why** in a description when the behaviour would surprise someone: limits, safety
  interlocks, differences from other tools ("A copter would loiter instead").
- **Exceptions are explicit.** When `check` reports a conflict, decide which behaviour wins
  in the overlap and mark it with `Overrides:`. Don't paper over it by narrowing Givens until
  they stop overlapping, unless that's really what the system does.
- **Invariants** (no `When`) are facts that always hold in a state: "An empty plan has no
  last item". Their sentence reads "Whenever …".

## Stories

- **One goal per story**, named the way a reader would look for it: "Your first throw",
  "Fixing a misplaced waypoint". Avoid names that describe the test ("Press PLAN twice").
- **Write the introduction.** One short paragraph after the narrative: what the reader will
  do, and why. It's the first thing on the tutorial page and on its card.
- **Three to eight steps, in one scenario.** Fewer is a topic, not a tutorial; more should be
  split, or folded into phrases. See [Sizes](#sizes).
- **Steps are intents.** Use the declarative events and phrases. Never put a key or button
  name in a story step.
- **Thens say what the reader sees.** Add one to every step whose result isn't obvious, and
  word it as the screen shows it.
- **Given only what's special about this tutorial.** The section's setup is already in
  Before you begin. A `Given` in a scenario is listed there too, so keep it to one or two.
- **One scenario.** Each scenario is a page of its own, titled with the scenario's name, so a
  story with several scenarios scatters one goal over several pages. Write a second story
  for a second goal.

## Diagnostics

| Code | Means | Usually |
| --- | --- | --- |
| `unknown-step` | A step isn't in the vocabulary | A typo, or a state, event or phrase to declare. The help says what's close |
| `conflict` | Two expectations both apply and promise different things | Add `Overrides:` to the one that should win |
| `override-cycle` | Overrides go round in a loop | One of them is the wrong way round |
| `unclear-step` | A story step might or might not set off an expectation | Pin the state with a `Given` or an earlier step, or make the `When` exact |
| `unspecified` | An event happens in a story and nothing is expected to | Write the expectation the story relies on |
| `contradicts` | A story's Then disagrees with the expectations | The story or the expectation is wrong; find out which |
| `clash` | Two expectations fire together and disagree | Settle the conflict first |
| `bad-phrase` | An expectation uses a phrase of several actions | Use a single action, or split the expectation |
| `unpromised` (warning) | A story checks something no expectation promises | Write the expectation, so it's specified, not just tested |
| `untested` (warning) | No story shows an expectation | A story, if a reader would want a tutorial; otherwise a generated test covers it |
| `shows-what` (warning) | A `Shows:` line doesn't say which value it's for | Start it with the value and a colon, or use `{value}` |
| `many-scenarios` (warning) | A story has more than one scenario | One story per goal; leave variants to the expectations |
| `short-story` / `long-story` (warning) | A tutorial has fewer than 3 or more than 8 steps | Add the steps around it, or let the topic page say it; split, or use a phrase |
| `big-feature` / `small-feature` (warning) | A topic has more than 8 expectations, or only 1 | Split it into topics a reader would scan for; fold it into the closest topic |
| `crowded-section` (warning) | A section has more than 8 topics or 8 tutorials | Make subsections |
| `no-tutorial` (warning) | A section with topics has no story | A story for the reader's main goal there |
| `no-way-in` (warning) | Nothing says how to get into a section | An expectation that leads there from the parent section's setup |

## Sizes

Each generated page has a size that reads well. Too many pages in one place, or pages with one
line on them, make the manual hard to find your way in. `check` warns about these sizes.

| What | Reads well | Too small | Too big |
| --- | --- | --- | --- |
| A section | 2 to 6 topics, 1 to 6 tutorials | No tutorial (`no-tutorial`) | More than 8 topics or 8 tutorials: make subsections (`crowded-section`) |
| A topic (`Feature:`) | 2 to 8 expectations that show | 1: fold it into the closest topic (`small-feature`) | More than 8: split it into topics a reader would scan for (`big-feature`) |
| A tutorial (a scenario) | 3 to 8 steps | Fewer than 3: it is a topic, not a tutorial (`short-story`) | More than 8: split it, or use a phrase (`long-story`) |
| A story | 1 scenario | | More than 1: each scenario is a page of its own (`many-scenarios`) |

**Do not restate the rules as stories.** A story with one scenario for each rule ("One drone
gets half way in 10 seconds", "Two drones take the tower in 10 seconds", …) gives one page for
each rule, each with one step, in the reader's navigation. It says nothing that the topic page
does not say. Write a tutorial as one walk through the feature: each step starts where the last
one stopped, and the steps show several rules on the way. Leave the rules that no tutorial shows
to the topic page. The suite tests each of them on its own.

**Write expectations that a tutorial can chain.** A tutorial step only fires an expectation if
the planner knows that its Given holds.

- Pin only what matters in a Given. "The same number of drones stops the control" needs no
  control value: say `Then the control … stays the same`.
- Prefer exact Thens to ranges. After `Then the airspeed is at least 17.5 m/s`, the planner does
  not know the airspeed, and the next step that needs `the airspeed is 18 m/s` is
  `unclear-step`. Let the driver read the value as the screen shows it (rounded to a tenth),
  and write `Then the airspeed is 18 m/s`.
- A relative Then (`stays the same`, `goes up by 1`) is checked against what the planner knows,
  not what the system had. Do not use one for a state that the spec does not model (Red's score
  when no expectation says Red earns points).
- A state that a tutorial changes in a `When` needs a `set` binding in the driver. An
  `arrange` binding is only for the start of a test.

## Line starts

A line that starts with `Given`, `When`, `Then`, `And` or `But` is a step wherever steps can
go: in a Background, an Expectation, a Story or a Scenario. STE puts conditions first ("When
the plane is in hand, …"), so in an expectation's or a story's text, wrap the line so that it
starts with another word. In a Feature, Section or Term, such a line is prose, because those
hold no steps.
