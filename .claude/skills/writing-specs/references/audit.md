# Docs audit rubric

You're auditing generated user documentation as a reader who has only the docs. The bar is
the user docs of a good open-source project: ArduPilot's and Mission Planner's, Bevy's book,
the Rust book. Could a new user follow these pages and succeed, and would they trust them?

Read the pages you were given in full. Don't read the specs or source code, except to check
that a key, button or command a page names really exists. Don't edit anything.

## What to check

Work through every page against each of these.

1. **Orientation.** The first paragraph says what the page is for and who it's for. A
   section page says what's in the section and when you'd use it. A reader landing from a
   search knows where they are.
2. **Before you begin.** It's complete and actionable: a reader who does what it says ends
   up where the page starts. Flag lists of bare states ("The rates are high. Differential
   thrust is on. …") where an instruction belongs, missing commands, and setup that assumes
   knowledge the page doesn't link to.
3. **Steps.** Each is an instruction in the imperative ("Press Space to throw the plane."),
   does one thing, and names the real control. Flag third person ("the pilot presses"),
   Gherkin words used as prose (Given, When, Then), vague controls ("set the flight mode"
   with no way to do it), and steps that bundle several unrelated actions.
4. **Outcomes.** After each step the reader knows what they should see, in the words the
   screen uses. Flag outcomes that are internal state rather than something visible ("the
   vehicle's current item is 1" with no hint where to look), long lists of states, and
   tautologies.
5. **Topic pages.** Each behaviour has a heading a reader would scan for, a sentence saying
   what happens and how to do it, and the why where it isn't obvious. Exceptions are clear
   about when they apply. Flag headings that are test names, and behaviours explained only in
   the collapsed test details.
6. **Terminology.** The same thing has the same name everywhere, and names match the UI
   (button labels, mode names, key names). Every term a page uses without explaining is in
   the glossary.
7. **Titles and navigation.** Tutorial titles are goals a reader would search for. Pages are
   where a reader would look for them. Previous and next make sense as a reading order.
8. **Test output stays out of the way.** Outside "How this is tested" blocks and the test
   report, there's no test jargon: expectation, verdict, coverage, generated, scenario,
   arranged, passed or failed. Flag any that leaks into the reading text.
9. **Screenshots.** Each step that changes what's on screen has one. Note placeholders
   ("Not supported yet", "No screenshot") separately: they're gaps in the tests, not the
   writing, and aren't blockers for the writing.
10. **Simplified Technical English.** The reading text is meant to follow ASD-STE100 in at
    least 80 % of its sentences. Count the sentences on each page that break a rule:
    instructions that aren't commands or exceed 20 words, descriptions over 25 words, more
    than one instruction in a sentence, a condition after its instruction, the passive voice,
    -ing verb forms, contractions, dropped articles, noun clusters of more than three words,
    and one thing called by several names. Leave out the Gherkin and the test details. Quoted
    screen text and product names don't count against a page. Give each page's compliance
    as a percentage, and flag the worst sentences.
11. **Accuracy and consistency.** Nothing contradicts another page. Commands, keys and
    numbers agree everywhere. Flag anything you checked against the code and found wrong.

12. **Size and place.** Each section has a few topics and a few tutorials, not one long list.
    A tutorial has three to eight steps, and it is a walk a reader would take, not one rule
    restated as a page. A topic page has a handful of behaviours, not one and not a dozen.
    Flag lists of near-identical tutorial titles in one section, one-step tutorials, and
    sections or parts a reader would not think to look in. Say where each would fit better.

## Severity

- **blocker**: a reader following the page would fail or be misled: a wrong key or command,
  a missing step, a contradiction.
- **major**: a reader would struggle or lose trust: state dumps where instructions belong,
  outcomes they can't see, unexplained jargon, a title that hides what the page is for.
- **minor**: polish: awkward wording, a missing why, an inconsistent name that's still clear,
  a sentence that breaks an STE rule.

A page under 80 % STE compliance is a **major** finding of its own.

## Report format

A short verdict first: one paragraph on whether the docs would serve a new user, and how
they compare with the bar. Then the STE compliance: the overall percentage, and any page
under 80 %.

Then the findings, most severe first, as a table:

| # | Severity | Page | Quote | Problem | Suggested fix |
| --- | --- | --- | --- | --- | --- |
| 1 | major | tutorial-sim--your-first-throw…html, step 2 | "The aircraft is in hand." | Doesn't say what the reader sees: the plane carried back to the pilot box | A Then worded as the screen shows it |

Quote the page's exact words (short). Say what to change in reader terms; the writer will
work out which spec line it comes from. Then list what's good, briefly, so it isn't lost in
the next round.

When asked to re-audit, say for each earlier finding whether it's fixed, partly fixed or not
fixed, then list any new findings in the same table.
