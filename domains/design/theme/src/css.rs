//! The theme as CSS custom properties: `--canvas`, `--ok`, `--radius-panel`, `--font-mono` and
//! so on, Day for light browsers and Night for dark ones.

use crate::{DAY, NIGHT, Palette, geometry, typography};
use std::fmt::Write;

fn colours(p: &Palette) -> String {
    let mut s = String::new();
    for (name, c) in p.roles() {
        let _ = write!(s, "--{name}: {}; ", c.css());
    }
    s
}

/// A stylesheet that defines every token: Day by default, Night when the reader prefers dark,
/// either one when the page says so with `data-theme="day"` or `"night"` on `<html>`, and
/// either one inside an element with the class `theme-day` or `theme-night`.
pub fn stylesheet() -> String {
    let (day, night) = (colours(&DAY), colours(&NIGHT));
    format!(
        "/* The T1-S theme (domains/design/theme). Generated: edit the theme, not this file. */\n\
         :root {{\n  --font-mono: {mono};\n  --font-sans: {sans};\n  \
         --radius-window: {w}px; --radius-panel: {p}px; --radius-screen: {sc}px; --radius-control: {c}px;\n  \
         --gap: {g}px; --bezel-pad: {bp}px;\n}}\n\
         :root, :root[data-theme=\"day\"] {{\n  color-scheme: light;\n  {day}\n}}\n\
         @media (prefers-color-scheme: dark) {{\n  :root:not([data-theme=\"day\"]) {{\n    color-scheme: dark;\n    {night}\n  }}\n}}\n\
         :root[data-theme=\"night\"] {{\n  color-scheme: dark;\n  {night}\n}}\n\
         /* Either variant inside part of a page, whatever the page uses: for side-by-side previews. */\n\
         .theme-day {{ color-scheme: light; {day} }}\n\
         .theme-night {{ color-scheme: dark; {night} }}\n",
        mono = typography::MONO,
        sans = typography::SANS,
        w = geometry::WINDOW_RADIUS,
        p = geometry::PANEL_RADIUS,
        sc = geometry::SCREEN_RADIUS,
        c = geometry::CONTROL_RADIUS,
        g = geometry::GAP,
        bp = geometry::BEZEL_PAD,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn defines_every_role_in_both_variants() {
        let css = super::stylesheet();
        for (name, _) in crate::NIGHT.roles() {
            assert_eq!(css.matches(&format!("--{name}:")).count(), 5, "{name}");
        }
        assert!(css.contains("--radius-panel: 11px"));
        assert!(css.contains("prefers-color-scheme: dark"));
    }
}
