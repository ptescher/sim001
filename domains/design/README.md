# Design

The project's look, shared by everything with a screen:

| Where | How it uses the theme |
| --- | --- |
| Ground station (`gcs/`, GPUI) | `gcs/src/ui/theme.rs` names the Night roles as the console's colours (`GREEN`, `AMBER`, `BEZEL`, …) and adds GPUI conversions |
| Sim HUD, controller overlay and loading screen (Bevy) | `src/theme.rs` turns the Night roles into Bevy colours (`color(P.caution)`) |
| Docs site (`docs/site/`) | `assets/theme.css` is generated from the theme: Day for light browsers, Night for dark ones. The site's own CSS uses only its tokens |
| Docs Markdown | the Mermaid map uses the Day colours, for GitHub's light page |
| TX15 radio (EdgeTX) | `design_theme::edgetx` places the roles on the radio's thirteen colour variables. `cargo run -p design_theme --example edgetx -- radio/THEMES` writes the Night and Day theme folders (`theme.yml`, a logo and a background) for the SD card's THEMES folder |

## Modules

| Crate | Path | Role |
| --- | --- | --- |
| `design_theme` | [theme/](theme) | The palettes, type and geometry as plain data, plus a CSS export. No dependencies |

## The theme

It's the ground station's console: near-black screens in olive bezels, phosphor-green
symbology, amber cautions, red warnings, cyan for information and magenta for what the
autopilot is steering to.

- **Night** is the console, and it's what the ground station and the sim's HUD use. It's the
  console's own palette with two nudges so that every ink reads as text: the warning red is
  `#ff5a4f` (was `#ff3b30`) and the grey `#919c91` (was `#8a958a`).
- **Day** is the same instrument in sunlight, with light tan panels and the signals as darker inks
  of the same hues. The docs use it in a light browser.

Colours are **roles**, not hues. `canvas`, `bezel`, `screen`, `text`, `muted`, `ok`, `caution`,
`warning`, `info`, `highlight`, the fills behind buttons, badges and lamps, and the attitude
indicator's `sky` and `ground` mean the same thing in both variants. Code written against the
roles works in either.

Type is monospaced (Menlo) for the instruments, labels, headings and code, with the system
sans for prose in the docs. Corners are concentric: window 17, panel 11, screen 6, control 4,
each inset stepping the radius down by the inset.

The radio themes are checked the same way: the radio's main text reads at 4.5:1 on every background it sits on (and is the palette's secondary ink, not its extreme, because the radio fills its monitor bars with that colour too) (the bars, the labels, a focused, edited or active field), and so do its labels, warnings and quick menu.

The theme's tests check that every ink reads at WCAG AA (4.5:1) on every surface it sits on,
and that each signal reads on its own fill, in both variants. The docs site's
[Theme page](../../docs/site/theme.html) shows both palettes and the console's controls side by
side.
