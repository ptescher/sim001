---
name: tdd
description: Test-driven development with the expectation framework. Before writing any code for a new behaviour or a bug fix, write the expectation (and a story if a reader would follow it), make a driver able to run it, watch it fail for the right reason, then implement until it passes, refactor under the suite, and regenerate the docs. Use it for every feature, behaviour change and bug fix in the sim, the ground station and the simulation domain, instead of writing the code first.
---

# TDD with expectations

The spec is the test and the manual, so the test comes first and the docs come out of it.
Never write the code and the test together, and never write the test after. A test you
haven't seen fail may not test anything.

Two loops:

- **Outer, the expectation.** One behaviour a person would notice. It lives in
  `expectations/`, runs against the real thing through a driver, and becomes a line in the
  manual. Red until the feature works.
- **Inner, the unit test.** The small steps inside the feature (a formula, a mixer, a
  parser). They live in the crate (`#[cfg(test)]`), run in a second, and go red, green and
  refactor many times per outer cycle.

The writing-specs skill covers how to write good specs (vocabulary, wording, stories,
audit). This skill is the order of work and the rules around it. Read writing-specs'
`references/style.md` before you write the spec.

## Which runner

| Behaviour of | Run it with | Driver |
| --- | --- | --- |
| the sim (keys, HUD, flight) | `cargo run --release -- --expect` | `src/acceptance.rs` |
| the ground station (buttons, map, vehicle commands) | `cd domains/groundstation/console && cargo run --release -- --station --expect` (needs Docker; add `--aircraft ID` for another plane) | `domains/groundstation/console/src/expect.rs` |
| the flight model alone (no window, no ArduPilot) | a `#[test]` that loads the suite and runs it through `expectation_harness::Bindings`, as in `domains/expectation/harness/examples/hand_launch` | the test's own bindings |

Checking a suite needs no run: `cargo run -p expectation_cli -- check expectations` and
`... -- plan expectations`.

## The cycle

Track these as a checklist. Do not skip ahead.

### 1. Red: write what should happen

1. Say the behaviour in one sentence a user would agree with. If you can't, ask them.
2. Write the vocabulary, the expectation and, if a reader would follow it as a task, the
   story (writing-specs steps 2 to 4). Name the real controls. Add a glossary term for any
   new word.
3. `check` the suite. Fix conflicts, add `Overrides:` for exceptions. Run `plan` and read the
   test steps: they must say what you meant.
4. Run the suite's own unit test (`cargo test --release the_suite_checks_clean` for the sim,
   `cargo test --release the_driver_covers` in `domains/groundstation/console/`). It will say the driver can't do the
   new steps yet.

### 2. Red: make the test runnable

A test the driver can't run shows "not supported yet". That is not red, it is no test.
Teach the driver the new steps first:

- an **event**: add it to `KEYS` (sim) or `EVENTS` (console), as the real key or click;
- a **state**: add it to the driver's observe (and arrange, if a Given sets it up);
- a state a step sets (`When the altitude is …`): add it to the driver's set.

Now run the suite and **look at the failure**. It must fail for the right reason: the
feature is missing or wrong, not the driver or the setup (a wrong key, a state read from the
wrong place, a timeout). Fix the test until the failure is the feature's. Keep the failing
output: it's evidence for the report.

If the behaviour has no way to be observed from outside (nothing on screen, in telemetry or
in the state the driver reads), the spec is incomplete. Add what makes it observable, as part
of the feature.

### 3. Green: the smallest change

1. Pick the smallest piece of the feature, and write a **unit test** for it first. Run it and
   see it fail (a missing function counts as failing to compile: make it compile, then fail).
2. Write only enough code to pass. Run the unit tests again.
3. Repeat until the outer test is the only thing left, then run the suite. Fix what it finds.

Don't add behaviour no test asks for. If you think of a case that matters, write its test
(or expectation) first.

### 4. Refactor

Only when green. Tidy names, remove duplication, move code to where it belongs. After each
step run the unit tests; at the end run everything:

```bash
cargo test --workspace --release                       # crates
cargo test --release                                   # the sim's own tests, the suite check
(cd domains/groundstation/console && cargo test --release)                       # the console's
cargo run --release -- --selftest > after.txt          # the T1-S's flight numbers
diff before.txt after.txt                              # they must not change
```

`before.txt` is `--selftest` from before you began (take it in step 1, from the same
commit). A behaviour that is meant to change the T1-S's numbers needs its own expectation and
a new baseline, with the reason in the report.

### 5. Docs

`docs/` is generated and not committed. Regenerate it (writing-specs step 6) and, for a
feature a reader will follow, audit it (steps 7 and 8). A story with no screenshot is a
tutorial with a gap: run the suite with `--docs`.

### 6. Report

Say, in this order: the behaviour; the red evidence (the failing output, and why it failed);
the green evidence (the same test passing, and the counts: passed, failed, not supported yet);
what you refactored; the regression check (selftest identical, or why not); and what the
docs now say. If you couldn't run something (no Docker, no display), say that and don't call
it passing.

## Rules

- **Test first, always.** A bug fix starts with an expectation or a unit test that
  reproduces the bug and fails. The fix is whatever turns it green.
- **See it fail.** Run the test before the code exists. Paste the failure in the report.
- **Never weaken a test to pass.** If an expectation is wrong, change it on purpose, with
  the reason, before touching the code, and say so.
- **One behaviour per cycle.** Several unrelated expectations written at once make a red
  you can't read.
- **Don't fix the generated docs.** Fix the spec.
- **A test that can't run is a gap, not a pass.** "Not supported yet" stays visible; don't
  delete stories because a driver can't do them. Make the driver do them.
- **No mocks for the thing under test.** The expectation runs the real sim or console. Inner
  tests may use small fakes (a flat terrain) for the parts around the thing.
- **Commit in a way that shows the order** when asked to commit: the spec and driver
  support (red), then the implementation (green), then the refactor. Don't commit a red
  suite without saying so.

## When it doesn't apply

A pure refactor (no behaviour change): the existing suite, the unit tests and the
`--selftest` diff are the test. A change to docs wording only: use writing-specs. An
experiment you will throw away: say so, and don't commit it.
