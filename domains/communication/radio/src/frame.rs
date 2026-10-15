//! MAVLink frames as they go through the radio: cut out of a byte stream without being read,
//! so that a message this domain has never heard of (ArduPilot has hundreds) goes through
//! untouched.

/// Who sent a frame and what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub sysid: u8,
    pub component: u8,
    pub msgid: u32,
}

/// Cuts a byte stream into frames (MAVLink 1 and 2). It does not check the checksum: it only
/// follows the lengths. Bytes that are not the start of a frame are dropped.
#[derive(Default)]
pub struct Framer {
    buf: Vec<u8>,
}

fn frame_len(b: &[u8]) -> Option<usize> {
    match b.first()? {
        0xFD => {
            let len = *b.get(1)? as usize;
            let signed = *b.get(2)? & 1 == 1;
            Some(12 + len + if signed { 13 } else { 0 })
        }
        0xFE => Some(8 + *b.get(1)? as usize),
        _ => Some(0),
    }
}

impl Framer {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut frames = vec![];
        loop {
            // Skip anything that does not start a frame.
            let start = self.buf.iter().position(|&b| b == 0xFD || b == 0xFE).unwrap_or(self.buf.len());
            self.buf.drain(..start);
            let Some(len) = frame_len(&self.buf) else { break };
            if len == 0 || self.buf.len() < len {
                break;
            }
            frames.push(self.buf.drain(..len).collect());
        }
        frames
    }
}

/// Who sent a frame and what it is.
pub fn info(frame: &[u8]) -> Option<Info> {
    match *frame.first()? {
        0xFD if frame.len() >= 10 => {
            Some(Info { sysid: frame[5], component: frame[6], msgid: u32::from_le_bytes([frame[7], frame[8], frame[9], 0]) })
        }
        0xFE if frame.len() >= 6 => Some(Info { sysid: frame[3], component: frame[4], msgid: frame[5] as u32 }),
        _ => None,
    }
}

/// Messages that a plane streams again and again, so that an old one is no use once there is
/// a new one: the state of the plane, and the Remote ID messages. (Anything not in this list is
/// treated as something that must arrive.)
pub fn is_stream(msgid: u32) -> bool {
    matches!(
        msgid,
        0 | 1 | 24 | 27 | 29 | 30 | 32 | 33 | 35 | 36 | 42 | 62 | 65 | 74 | 116 | 125 | 136 | 152 | 163 | 168 | 178 | 182 | 193 | 241 | 246
            | 12900..=12919
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use communication_telemetry::{Beacon, frame, global_position, heartbeat};
    use communication_traffic::Position;

    fn beacon() -> Beacon {
        Beacon { sysid: 4, position: Position::new(1.0, 2.0, 3.0), velocity: [0.0; 3], heading: 0.0, uas_id: "x".into(), mac: [0; 6] }
    }

    #[test]
    fn a_stream_in_pieces_gives_whole_frames() {
        let b = beacon();
        let mut stream = frame(4, 1, 0, &heartbeat(&b));
        let first = stream.clone();
        let second = frame(4, 1, 1, &global_position(&b, 5));
        stream.extend(&second);
        let mut f = Framer::default();
        let mut got = vec![];
        for piece in stream.chunks(5) {
            got.extend(f.push(piece));
        }
        assert_eq!(got, vec![first, second]);
    }

    #[test]
    fn noise_is_dropped_and_the_next_frame_is_found() {
        let b = beacon();
        let good = frame(4, 1, 0, &heartbeat(&b));
        let mut f = Framer::default();
        let mut stream = vec![0x00, 0x11, 0x22];
        stream.extend(&good);
        assert_eq!(f.push(&stream), vec![good]);
    }

    #[test]
    fn a_frame_says_who_sent_it_and_what_it_is() {
        let b = beacon();
        let i = info(&frame(4, 236, 0, &global_position(&b, 5))).unwrap();
        assert_eq!(i, Info { sysid: 4, component: 236, msgid: 33 });
        assert_eq!(info(&[1, 2, 3]), None);
        assert!(is_stream(33) && is_stream(12915));
        assert!(!is_stream(77), "COMMAND_ACK must arrive");
        assert!(!is_stream(39), "MISSION_ITEM must arrive");
    }

    #[test]
    fn a_message_this_domain_has_never_heard_of_goes_through_whole() {
        // A MAVLink 2 frame with a message id that no dialect here has: the framer only follows the length.
        let mut raw = vec![0xFD, 3, 0, 0, 9, 1, 1, 0x34, 0x12, 0x00, 7, 8, 9, 0xAA, 0xBB];
        let mut f = Framer::default();
        assert_eq!(f.push(&raw), vec![raw.clone()]);
        raw[2] = 1; // signed: 13 more bytes
        raw.extend([0; 13]);
        assert_eq!(f.push(&raw).len(), 1);
    }
}
