---
name: writing-specs
description: Write or revise behaviour specs (.expect files under expectations/) for the expectation domain, and the user docs they generate. Covers the whole lifecycle — high-level expectations and vocabulary, the conflict and flow-graph check, stories written as tutorials, generating the docs, a subagent audit of the docs as a reader would see them, and a feedback loop on the specs until the docs read well. Use it when adding or changing a feature's spec, writing a user story or tutorial, or improving the generated manual in docs/.
---

# Writing specs

A spec here is two things at once: the acceptance tests and the user manual. Every
expectation is a behaviour the tests check and a sentence in a topic page. Every story is a
test and a step-by-step tutorial. So write for the reader first, the way good open-source
docs do (ArduPilot's and Mission Planner's are the bar), and let the tooling make the tests.

Read [references/style.md](references/style.md) before writing anything. It's the language,
the wording rules that turn specs into docs, and the anti-patterns. The audit rubric is
[references/audit.md](references/audit.md).

**Write the reader-facing text in Simplified Technical English (ASD-STE100).** Prefer full
STE; at least 80 % of the sentences must comply. That covers descriptions, story
introductions, glossary terms, and the `Say:`, `Set:` and `Shows:` lines. Short commands,
one instruction per sentence, the active voice, one word for one thing. style.md lists the
rules that matter most, and the audit measures compliance page by page.

Where things are:

| What | Where |
| --- | --- |
| The suite | `expectations/` (one directory per docs section; `section.expect` holds its `Section:`, `Vocabulary:` and `Background:`) |
| The language and docs generator | [https://github.com/ptescher/expectation](https://github.com/ptescher/expectation) (its README describes both); the source Cargo checked out is under `~/.cargo/git/checkouts/expectation-*` |
| How steps become instructions | `crates/docs/src/voice.rs` in that repo |
| The sim's test driver | `src/acceptance.rs` (`KEYS` maps each event to a key) |
| The generated manual | `docs/` (Markdown) and `docs/site/` (HTML). Never edit these by hand |

## The lifecycle

Track these as a checklist and do them in order. Steps 6 to 8 are a loop.

### 1. Understand the behaviour

Before writing a line, find out what the user actually does and sees:

- Read the code for the feature: the real key bindings, button labels, mode names and
  messages. In the sim they're in `src/input.rs` and the help text in `src/hud.rs`; in the
  ground station, `domains/teleoperation/console/src/ui/`. Docs that name a button that doesn't exist are wrong docs.
- Read the section the feature belongs in (`expectations/<section>/`), its vocabulary, and
  the glossary page (`docs/glossary.md`), so you reuse terms instead of inventing new ones.
- Decide what the reader's goals are. Those become stories. The rules the system follows
  become expectations.

### 2. Write the vocabulary and the expectations

In the section's `section.expect`:

- **States** for what the user can see or set, with the values as they appear on screen. Add
  a `Set:` line to any state a reader changes directly ("press M until the flight mode is
  {value}"), and `Shows:` lines that say what the reader sees for each value ("crashed: the
  HUD says CRASHED").
- **Events** named for intent ("the operator uploads the plan"), each with a `Say:` naming the
  real control ("press UPLOAD"). Events nobody does ("the plane reaches a waypoint") have no
  `Say:`; the docs say "wait until …".
- **Phrases**: a setup phrase for the section's Background, with a `Say:` that's a real
  instruction (the command to run, what the reader should see). Action phrases for the
  multi-step intents stories repeat ("the operator plans three waypoints").

Define any term a new reader might not know with `Term:` in `expectations/glossary.expect`,
and link to it with `[[the term]]`.

Then a feature file (`NN-<name>.expect`) per topic: a `Feature:` with a description that
reads as the topic's opening paragraph, and one `Expectation:` per behaviour. Name each like
a manual heading (the rule in plain words: "Reset picks the plane back up"), give it a
description when the why isn't obvious, and keep its Given to what matters. Mark exceptions
with `Overrides:`.

### 3. Check the graph

```bash
cargo run -p expectation -- check expectations      # errors and warnings; exits 1 on errors
cargo run -p expectation -- graph expectations      # the flow DAG as Mermaid
```

Fix every error. Conflicts mean two expectations promise different things in the same
situation: decide which is the exception and add `Overrides:`, or narrow a Given. Read the
warnings too: `no-way-in` means the docs can't say how to reach a section, and `untested`
lists expectations no story shows (each comes with a route a story could take). The
diagnostic codes are explained in style.md.

### 4. Write the stories as tutorials

In the section's `stories.expect`. Each story is one tutorial page:

- Name it for the reader's goal ("Planning a hand-launched circuit"), not the mechanics.
- The narrative lines (As a / I want / So that) are for the test report. The paragraph after
  them is the tutorial's introduction: say what the reader will do and why it matters.
- Each `When` is a numbered step. Use phrases and intent-named events, never raw keys. One
  thing per step, three to eight steps.
- Give the steps that matter a `Then` the reader can see on screen. Without one, the docs
  list what the expectations promise, which is accurate but often wordy.

```bash
cargo run -p expectation -- plan expectations       # each tutorial as test steps and checks
```

A plan that stops early, or a story error, means the story relies on behaviour no
expectation specifies. Specify it, don't work around it.

### 5. Update the driver, if there is one

New or renamed events need the driver to know them. For the sim, add them to `KEYS` in
`src/acceptance.rs`, then:

```bash
cargo test --bin sim001 the_suite_checks_clean          # the suite checks clean and the driver covers it
```

The ground station has no driver yet, so its tests show "Not supported yet" placeholders.
That's expected; don't remove stories because of it.

### 6. Generate the docs

```bash
cargo run -p expectation -- docs expectations docs   # from the results saved in docs/results
cargo run --release -- --expect --docs docs              # or run the sim's tests and get screenshots
```

The sim run opens a window and takes several minutes. Its window must stay visible, or the
screenshots come out black and are retried. Run it in the background and check on it.

### 7. Audit the docs with a subagent

Spawn a fresh `general-purpose` agent with the Agent tool. It must judge the docs as a reader
who has only the docs, so don't let it read the `.expect` files or this conversation. Give it:

- the pages to read: the section's page in `docs/site/`, its topic pages, its tutorials, and
  the glossary. Name the files. The Markdown in `docs/` has the same content if HTML is awkward.
- the rubric: tell it to read `.claude/skills/writing-specs/references/audit.md` and follow it.
- the output format the rubric asks for, and an instruction not to edit anything.

A prompt that works:

> You're reviewing user documentation for a flight simulator and ground station, as a new
> user would read it. Read `.claude/skills/writing-specs/references/audit.md` and audit these
> pages against it: <list of files>. Don't read anything under `expectations/` or the
> source code except to check that a key, button or command the docs name really exists.
> Don't edit any files. Report findings in the rubric's format, most severe first.

### 8. Fix the specs, then loop

For each finding, change the spec, not the generated page: a vague step needs a better
`Say:`, a phrase, or a `Then`; a missing why needs a description; a confusing title needs a
better name; a state dump in "Before you begin" needs a setup phrase. Only when a finding is
about how every page words something (a rule in `voice.rs`, or a page layout in
`crates/docs/src/`) change the generator in a checkout of [https://github.com/ptescher/expectation](https://github.com/ptescher/expectation), run its tests
there, push, and `cargo update -p expectation_harness` here:

```bash
cargo test -p expectation_docs -p expectation_language -p expectation_story   # in the expectation repo
```

Then go back to step 3: check, regenerate and audit again. Reuse the same audit agent with
SendMessage ("I've changed these pages; re-audit them and say which findings are fixed"), so
it compares against its own earlier findings. Stop when an audit finds no blocker or major
findings and every page is at 80 % STE or better, or after three rounds. Then report what's left.

### 9. Report

Tell the user what changed in the specs, what the audit found and fixed in each round, what's
left (with the remaining findings), and the test results: passed, failed and not supported
yet, and whether the screenshots are real.
