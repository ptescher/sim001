//! The theme as an EdgeTX radio theme: `theme.yml`, and the files that make up a folder in the
//! SD card's THEMES directory. The radio has thirteen colour variables, each for a few kinds of
//! thing on its screen (the top bar, the labels, a field being edited), so the roles of the
//! palette are placed on them by what they are for.

use crate::{Palette, Rgb};
use std::fmt::Write;

/// The radio's thirteen colour variables, in the order of its theme file.
///
/// - `PRIMARY1` is the text of labels and unfocused buttons: the console's phosphor green.
/// - `PRIMARY2` is the main text and icons, and also the radio's solid fills (the monitor's bars,
///   the knobs), so it is the palette's secondary ink and not its extreme: near-black text on
///   the day theme would make black bars, and near-white on the night theme glaring ones.
///   `PRIMARY3` is the scroll marker and the inactive icons.
/// - `SECONDARY1` is the top and bottom bars, `SECONDARY2` the labels and buttons, `SECONDARY3`
///   the main screen and popups.
/// - `FOCUS`, `EDIT` and `ACTIVE` are the backgrounds of a focused field, a field being edited
///   and an active button: the fills of an OK lamp, a caution and a pressed control.
pub fn colors(p: &Palette) -> [(&'static str, Rgb); 13] {
    [
        ("PRIMARY1", p.ok),
        ("PRIMARY2", p.muted),
        ("PRIMARY3", p.bezel_edge),
        ("SECONDARY1", p.title),
        ("SECONDARY2", p.control),
        ("SECONDARY3", p.screen),
        ("FOCUS", p.ok_fill),
        ("EDIT", p.caution_fill),
        ("ACTIVE", p.control_hover),
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

/// The files of a theme folder: the theme and a logo for the radio's theme list (a strip of the
/// signal colours).
pub fn folder(p: &Palette, name: &str, info: &str) -> Vec<(String, Vec<u8>)> {
    vec![("theme.yml".into(), theme_yml(p, name, info).into_bytes()), ("logo.png".into(), logo(p))]
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
    /// text on the bars, the main screen and the focused, edited and active fields; the labels'
    /// text on the label background; a warning on the main screen; the quick menu's own pair.
    #[test]
    fn text_reads_where_the_radio_puts_it() {
        for p in [NIGHT, DAY] {
            let c = |n| get(&p, n);
            for bg in ["SECONDARY1", "SECONDARY2", "SECONDARY3", "FOCUS", "EDIT", "ACTIVE"] {
                let r = c("PRIMARY2").contrast(c(bg));
                assert!(r >= 4.5, "{}: main text on {bg} is {r:.1}:1", p.name);
            }
            let r = c("PRIMARY1").contrast(c("SECONDARY2"));
            assert!(r >= 4.5, "{}: label text is {r:.1}:1", p.name);
            let r = c("WARNING").contrast(c("SECONDARY3"));
            assert!(r >= 4.5, "{}: a warning is {r:.1}:1", p.name);
            let r = c("QM_FG").contrast(c("QM_BG"));
            assert!(r >= 4.5, "{}: the quick menu is {r:.1}:1", p.name);
            // Disabled things (the channel names on the monitor) are dim, but a pilot can read them.
            let r = c("DISABLED").contrast(c("SECONDARY3"));
            assert!(r >= 2.5, "{}: disabled is {r:.1}:1", p.name);
        }
    }

    /// The radio fills things with its main text colour as well (the monitor's bars, the knobs),
    /// so that colour can't be the extreme of the palette: black on the day theme, white on the
    /// night one. It reads at 4.5:1, and stays under 10:1 against the main screen.
    #[test]
    fn the_main_text_is_not_the_extreme_of_the_palette() {
        for p in [NIGHT, DAY] {
            let r = get(&p, "PRIMARY2").contrast(get(&p, "SECONDARY3"));
            assert!((4.5..10.0).contains(&r), "{}: the main text is {r:.1}:1 on the screen", p.name);
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
    fn the_folder_has_a_theme_and_a_logo() {
        let files = folder(&DAY, "T1S Day", "Day console");
        let names: Vec<_> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["theme.yml", "logo.png"]);
        assert_eq!(&files[1].1[..8], b"\x89PNG\r\n\x1a\n");
    }
}
