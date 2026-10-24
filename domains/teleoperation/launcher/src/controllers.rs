//! The controllers that are plugged in, and what `controls.cfg` says about the radio.

use crate::job::layout;
#[cfg(test)]
use std::path::Path;

#[derive(Clone)]
pub struct Device {
    pub name: String,
    pub vendor: u16,
    pub product: u16,
}

/// USB joysticks and gamepads (HID usage page 1, usage 4 or 5).
pub fn connected() -> Vec<Device> {
    let Ok(api) = hidapi::HidApi::new() else { return vec![] };
    let mut out: Vec<Device> = vec![];
    for d in api.device_list() {
        if d.usage_page() == 1 && matches!(d.usage(), 4 | 5) {
            let name = d.product_string().unwrap_or("Controller").to_string();
            if !out.iter().any(|o| o.vendor == d.vendor_id() && o.product == d.product_id() && o.name == name) {
                out.push(Device { name, vendor: d.vendor_id(), product: d.product_id() });
            }
        }
    }
    out
}

/// What `controls.cfg` (in the repository, where the sim reads it) holds.
pub struct Mapping {
    pub exists: bool,
    /// Each `radio.<name>` the file maps.
    pub channels: Vec<String>,
    pub radios: Vec<String>,
}

pub fn mapping() -> Mapping {
    match std::fs::read_to_string(layout().data.join("controls.cfg")) {
        Ok(text) => parse_mapping(&text),
        Err(_) => Mapping { exists: false, channels: vec![], radios: vec![] },
    }
}

/// What the text of a `controls.cfg` maps.
pub fn parse_mapping(text: &str) -> Mapping {
    let mut channels = vec![];
    let mut radios = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim();
        if k == "radio.names" {
            radios = v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        } else if let Some(ch) = k.strip_prefix("radio.") {
            channels.push(ch.to_string());
        }
    }
    Mapping { exists: true, channels, radios }
}

/// A radio on its USB cable in serial mode (USB Serial set to LUA in EdgeTX): it shows up as a
/// serial device, not a joystick, and carries the flight computer's line (telemetry, MAVLink).
#[derive(Clone)]
pub struct SerialRadio {
    pub name: String,
    pub path: String,
    /// The name says it is a TX15 / EdgeTX radio (other serial devices are listed too).
    pub radio: bool,
}

/// The serial devices that are plugged in, with the product names macOS (ioreg) gives them.
pub fn serial_radios() -> Vec<SerialRadio> {
    let mut paths: Vec<String> = std::fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("cu.usbmodem") || n.starts_with("cu.usbserial") || n.starts_with("ttyACM"))
        .map(|n| format!("/dev/{n}"))
        .collect();
    paths.sort();
    // ioreg lists each USB device's product name before the serial device under it.
    let names = std::process::Command::new("ioreg")
        .args(["-r", "-c", "IOUSBHostDevice", "-l", "-w0"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let mut by_path: Vec<(String, String)> = vec![];
    let mut product = String::new();
    for line in names.lines() {
        let value = line.split_once(" = ").map(|(_, v)| v.trim().trim_matches('"').to_string());
        if line.contains("\"USB Product Name\"") {
            product = value.unwrap_or_default();
        } else if line.contains("\"IOCalloutDevice\"")
            && let Some(v) = value
        {
            by_path.push((v, product.clone()));
        }
    }
    paths
        .into_iter()
        .map(|path| {
            let name = by_path.iter().find(|(p, _)| *p == path).map(|(_, n)| n.clone()).filter(|n| !n.is_empty());
            let l = name.as_deref().unwrap_or("").to_lowercase();
            let radio = ["tx15", "radiomaster", "edgetx", "opentx"].iter().any(|k| l.contains(k));
            SerialRadio { name: name.unwrap_or_else(|| "Serial device".into()), path, radio }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_cfg_maps_its_radio_channels_and_names_its_radios() {
        let m = parse_mapping(
            "# made by the wizard\nradio.names = TX15, Pro Controller\nradio.roll = axis 0\nradio.pitch = axis 1 inverted\n\
             # radio.yaw = axis 3\nkeyboard.throttle = shift\nradio.arm = button 4\n",
        );
        assert!(m.exists);
        assert_eq!(m.radios, ["TX15", "Pro Controller"]);
        assert_eq!(m.channels, ["roll", "pitch", "arm"]);
    }

    #[test]
    fn the_example_controls_cfg_maps_the_tx15() {
        let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../controls.cfg.example"))
            .unwrap();
        let m = parse_mapping(&text);
        assert!(m.channels.len() >= 4, "{:?}", m.channels);
    }
}
