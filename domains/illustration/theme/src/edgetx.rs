//! The theme as an EdgeTX radio theme: `theme.yml`, and the files that make up a folder in the
//! SD card's THEMES directory. The radio has thirteen colour variables, each for a few kinds of
//! thing on its screen (the top bar, the labels, a field being edited), so the roles of the
//! palette are placed on them by what they are for.

use crate::{Palette, Rgb};
use std::fmt::Write;

/// The radio's thirteen colour variables, in the order of its theme file.
///
/// The radio is drawn for a light screen, and reuses a few variables for more than one job, so
/// the roles are placed by how the radio uses them, which the tests check:
///
/// - `PRIMARY1` is the text on the main screen, the labels and the rows of a list: the console's
///   ink for OK (phosphor green at night, a dark green by day).
/// - `PRIMARY2` is the background of an unfocused row and the fill of the monitor's bars, and
///   also the text on every strong fill (the top and bottom bars and a focused, edited or active
///   field). So it is a mid panel tone that `PRIMARY1` reads on, and the fills are inks that it
///   reads on: green, amber and cyan at night, their dark versions by day.
/// - `PRIMARY3` is the scroll marker and the inactive icons.
/// - `SECONDARY1` is the top and bottom bars, `SECONDARY2` the labels and buttons, `SECONDARY3`
///   the main screen and popups.
/// - `FOCUS`, `EDIT` and `ACTIVE` are the backgrounds of a focused field, a field being edited
///   and an active button.
pub fn colors(p: &Palette) -> [(&'static str, Rgb); 13] {
    let dark = p.screen.luminance() < 0.5;
    [
        ("PRIMARY1", p.ok),
        ("PRIMARY2", if dark { p.bezel_edge } else { p.title }),
        ("PRIMARY3", if dark { p.muted } else { p.bezel_edge }),
        ("SECONDARY1", p.ok),
        ("SECONDARY2", p.control),
        ("SECONDARY3", p.screen),
        ("FOCUS", p.ok),
        ("EDIT", p.caution),
        ("ACTIVE", p.info),
        ("WARNING", p.warning),
        ("DISABLED", mix(p.faint, p.muted)),
        ("QM_BG", p.bezel),
        ("QM_FG", p.text),
    ]
}

/// Halfway between two colours.
fn mix(a: Rgb, b: Rgb) -> Rgb {
    let m = |x: u8, y: u8| ((x as u32 + y as u32) / 2) as u32;
    Rgb(m(a.r(), b.r()) << 16 | m(a.g(), b.g()) << 8 | m(a.b(), b.b()))
}

/// The contents of `theme.yml`.
pub fn theme_yml(p: &Palette, name: &str, info: &str) -> String {
    let mut s = String::from("---\n");
    let _ = write!(s, "summary:\n  name: {name}\n  author: T1-S sim\n  info: {info}\ncolors:\n");
    for (n, c) in colors(p) {
        let _ = writeln!(s, "  {n}: 0x{:06X}", c.0);
    }
    s
}

/// The files of a theme folder: the theme, a logo for the radio's theme list (a strip of the
/// signal colours) and the background behind the radio's screens.
pub fn folder(p: &Palette, name: &str, info: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        ("theme.yml".into(), theme_yml(p, name, info).into_bytes()),
        ("logo.png".into(), logo(p)),
        ("background.png".into(), background(p)),
    ]
}

/// The TX15's screen, 480 x 272: the main screen between the title bar at the top and the
/// status bar at the bottom, in the colours the radio draws them with, so the background matches
/// the UI around the widgets.
fn background(p: &Palette) -> Vec<u8> {
    let (w, h) = (480usize, 272usize);
    let (title_h, status_h) = (40usize, 20usize);
    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for _ in 0..w {
            let c = if y < title_h || y >= h - status_h { p.title } else { p.screen };
            px.extend([c.r(), c.g(), c.b()]);
        }
    }
    png(w, h, &px)
}

/// A 120 x 40 strip: the screen, with a band each of the OK, caution, warning and info inks.
fn logo(p: &Palette) -> Vec<u8> {
    let (w, h) = (120usize, 40usize);
    let bands = [p.ok, p.caution, p.warning, p.info];
    let mut px = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let c = if (14..26).contains(&y) && (10..110).contains(&x) { bands[(x - 10) / 25] } else { p.screen };
            px.extend([c.r(), c.g(), c.b()]);
        }
    }
    png(w, h, &px)
}

/// An RGB PNG with the pixels stored uncompressed, which is plenty for a small logo.
fn png(w: usize, h: usize, rgb: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for row in rgb.chunks(w * 3) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    // zlib: a header, stored blocks of at most 65535 bytes, and the Adler-32 of the data.
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, b) in blocks.iter().enumerate() {
        z.push((i + 1 == blocks.len()) as u8);
        z.extend((b.len() as u16).to_le_bytes());
        z.extend((!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut d) = (1u32, 0u32);
    for &b in &raw {
        a = (a + b as u32) % 65521;
        d = (d + a) % 65521;
    }
    z.extend(((d << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend(crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend((w as u32).to_be_bytes());
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DAY, NIGHT};

    fn get(p: &Palette, name: &str) -> Rgb {
        colors(p).iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("no variable {name}")).1
    }

    /// The radio has the thirteen variables, each once.
    #[test]
    fn there_are_thirteen_variables() {
        let names: Vec<_> = colors(&NIGHT).iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            ["PRIMARY1", "PRIMARY2", "PRIMARY3", "SECONDARY1", "SECONDARY2", "SECONDARY3", "FOCUS", "EDIT", "ACTIVE", "WARNING", "DISABLED", "QM_BG", "QM_FG"]
        );
    }

    /// Text has to read on every background the radio puts it on, in both variants: the main
    /// text on the screen, the labels and the rows of a list; the text of the strong fills
    /// (bars, focused, edited and active fields) on those fills; a warning on the screen; the
    /// quick menu's own pair.
    #[test]
    fn text_reads_where_the_radio_puts_it() {
        for p in [NIGHT, DAY] {
            let c = |n| get(&p, n);
            for bg in ["SECONDARY2", "SECONDARY3", "PRIMARY2"] {
                let r = c("PRIMARY1").contrast(c(bg));
                assert!(r >= 4.5, "{}: main text on {bg} is {r:.1}:1", p.name);
            }
            for bg in ["SECONDARY1", "FOCUS", "EDIT", "ACTIVE"] {
                let r = c("PRIMARY2").contrast(c(bg));
                assert!(r >= 4.5, "{}: text on the {bg} fill is {r:.1}:1", p.name);
            }
            let r = c("WARNING").contrast(c("SECONDARY3"));
            assert!(r >= 4.5, "{}: a warning is {r:.1}:1", p.name);
            let r = c("QM_FG").contrast(c("QM_BG"));
            assert!(r >= 4.5, "{}: the quick menu is {r:.1}:1", p.name);
            // Disabled things (the channel names on the monitor) are dim, but a pilot can read them.
            let r = c("DISABLED").contrast(c("SECONDARY3"));
            assert!(r >= 2.5, "{}: disabled is {r:.1}:1", p.name);
        }
    }

    /// The monitor's bars and the scroll marker are drawn in colours that have to show against
    /// the main screen, even though they are not text. By day the bars are a pale tint: the rows
    /// they share a colour with carry dark text, so they can't be much darker than that.
    #[test]
    fn bars_and_the_scroll_marker_show_on_the_screen() {
        for p in [NIGHT, DAY] {
            let screen = get(&p, "SECONDARY3");
            for n in ["PRIMARY2", "PRIMARY3"] {
                let r = get(&p, n).contrast(screen);
                assert!(r >= 1.25, "{}: {n} is {r:.1}:1 on the screen", p.name);
            }
        }
    }

    /// The file starts with `---` and has a summary and all thirteen colours as 0xRRGGBB.
    #[test]
    fn the_theme_file_has_a_summary_and_the_colours() {
        let y = theme_yml(&NIGHT, "T1S Night", "Night console");
        assert!(y.starts_with("---\n"));
        assert!(y.contains("summary:\n  name: T1S Night\n"));
        assert!(y.contains("colors:\n"));
        assert!(y.contains(&format!("  SECONDARY3: 0x{:06X}\n", NIGHT.screen.0)));
        assert_eq!(y.matches("0x").count(), 13);
    }

    /// The folder holds the theme file and a logo, and the logo is a PNG.
    #[test]
    fn the_folder_has_a_theme_a_logo_and_a_background() {
        let files = folder(&DAY, "T1S Day", "Day console");
        let names: Vec<_> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["theme.yml", "logo.png", "background.png"]);
        for (_, bytes) in &files[1..] {
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        }
    }

    /// The T1SFC widget on the radio carries the console's colours as literals (it cannot read this
    /// crate): they must be the palettes' own.
    #[test]
    fn the_flight_display_widget_has_the_palettes_colours() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../teleoperation/edgetx/WIDGETS/T1SFC/main.lua");
        let lua = std::fs::read_to_string(path).expect("the widget");
        for (p, table) in [(&NIGHT, "night"), (&DAY, "day")] {
            let start = lua.find(&format!("  {table} = {{")).unwrap_or_else(|| panic!("no {table} palette in the widget"));
            let block = &lua[start..lua.len().min(start + 700)];
            for (role, c) in [
                ("bg", p.screen), ("panel", p.title), ("text", p.text), ("muted", p.muted), ("ok", p.ok), ("caution", p.caution),
                ("warning", p.warning), ("marker", p.marker), ("sky", p.sky), ("ground", p.ground),
            ] {
                let want = format!("{role} = {{ 0x{:02x}, 0x{:02x}, 0x{:02x} }}", c.r(), c.g(), c.b());
                assert!(block.contains(&want), "{table}: {want}");
            }
        }
    }
}
