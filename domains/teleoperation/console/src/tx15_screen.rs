//! The simulated TX15's screen, as the sim writes it for the station's tests (`--tx15-screen`):
//! what its lamps show, and its picture. The picture goes beside the console's in the tests'
//! screenshots, so that a tutorial shows the radio as well as the console.

/// What the lamps along the top of the screen show.
#[derive(Debug, PartialEq)]
pub struct Lamps {
    pub mode: String,
    pub armed: bool,
}

/// Read the sim's JSON: `{"live":true,"mode":"FBWA","armed":true}`.
pub fn parse(json: &str) -> Option<Lamps> {
    let field = |name: &str| -> Option<&str> {
        let at = json.find(&format!("\"{name}\":"))? + name.len() + 3;
        let rest = json[at..].trim_start();
        let end = rest.find([',', '}']).unwrap_or(rest.len());
        Some(rest[..end].trim())
    };
    Some(Lamps { mode: field("mode")?.trim_matches('"').to_string(), armed: field("armed")? == "true" })
}

/// The size of a JPEG, from its start-of-frame marker.
pub fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
    }
    None
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                s.push(A[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// A tag without one attribute: ` width="480"` is taken out of `<svg width="480" ...`.
fn without_attribute(tag: &str, name: &str) -> String {
    let key = format!(" {name}=\"");
    match tag.find(&key) {
        Some(at) => match tag[at + key.len()..].find('"') {
            Some(end) => format!("{}{}", &tag[..at], &tag[at + key.len() + end + 1..]),
            None => tag.to_string(),
        },
        None => tag.to_string(),
    }
}

/// One picture: the TX15's screen (twice its size) above the console's window.
pub fn composite(console_jpg: &[u8], tx15_svg: &str) -> Option<String> {
    let (w, h) = jpeg_size(console_jpg)?;
    let (sw, sh, margin) = (960u32, 544u32, 24u32);
    let inner = tx15_svg.find("<svg")?;
    // The screen's own size gives way to the one it has here (an attribute may not appear twice).
    let tag_end = inner + tx15_svg[inner..].find('>')?;
    let tag = without_attribute(&without_attribute(&tx15_svg[inner..tag_end], "width"), "height");
    let screen = format!(
        "{}{}",
        tag.replacen("<svg", &format!("<svg x=\"{}\" y=\"{margin}\" width=\"{sw}\" height=\"{sh}\"", (w.saturating_sub(sw)) / 2), 1),
        &tx15_svg[tag_end..]
    );
    let total = h + sh + 2 * margin;
    Some(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 {w} {total}\" width=\"{w}\" height=\"{total}\">\
<rect width=\"{w}\" height=\"{total}\" fill=\"#1a1b17\"/>{screen}\
<image x=\"0\" y=\"{}\" width=\"{w}\" height=\"{h}\" href=\"data:image/jpeg;base64,{}\"/></svg>",
        sh + 2 * margin,
        base64(console_jpg)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lamps_are_read_from_the_json_the_sim_writes() {
        assert_eq!(parse(r#"{"live":true,"mode":"FBWA","armed":true}"#), Some(Lamps { mode: "FBWA".into(), armed: true }));
        assert_eq!(parse(r#"{"live":false,"mode":"NO LINK","armed":false}"#), Some(Lamps { mode: "NO LINK".into(), armed: false }));
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn base64_matches_the_standard_examples() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn a_jpegs_size_is_read_from_its_frame_header() {
        // SOI, a frame header (baseline) 7 high and 5 wide.
        let jpg = [0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0, 7, 0, 5, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
        assert_eq!(jpeg_size(&jpg), Some((5, 7)));
    }

    #[test]
    fn the_composite_is_well_formed_where_the_screen_has_a_size_of_its_own() {
        let jpg = [0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0, 100, 0, 200, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
        let screen = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 480 272\" width=\"480\" height=\"272\"><text>X</text></svg>";
        let svg = composite(&jpg, screen).unwrap();
        for attribute in [" width=\"", " height=\""] {
            let start = svg.find("<svg x=").unwrap();
            let tag = &svg[start..start + svg[start..].find('>').unwrap()];
            assert_eq!(tag.matches(attribute).count(), 1, "{tag}");
        }
        assert!(svg.contains("<text>X</text>"));
    }

    #[test]
    fn the_composite_has_the_radio_above_the_console() {
        let jpg = [0xFF, 0xD8, 0xFF, 0xC0, 0, 17, 8, 0, 100, 0, 200, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
        let svg = composite(&jpg, "<svg viewBox=\"0 0 480 272\"><text>FBWA</text></svg>").unwrap();
        assert!(svg.contains("viewBox=\"0 0 200 692\"") && svg.contains("FBWA") && svg.contains("data:image/jpeg;base64,"));
    }
}
