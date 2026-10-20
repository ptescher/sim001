//! The touch bridge: the Rust driver listens, and an XCUITest running in the Simulator
//! (`ios/App/UITests/Bridge.swift`) connects and does what it is asked, a line at a time.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Bridge {
    conn: BufReader<TcpStream>,
    xcodebuild: Child,
}

impl Bridge {
    /// Start `xcodebuild test` on the app's UI test and wait for it to connect. The first run
    /// builds the Rust station and the app, so it is given a good while.
    pub fn start(app_dir: &Path, device: &str, bridge_port: u16, control_port: u16, address: &str) -> Result<Bridge, String> {
        let listener = TcpListener::bind(("127.0.0.1", bridge_port)).map_err(|e| format!("port {bridge_port}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let log = std::fs::File::create(std::env::temp_dir().join("sim001_ios_expect_xcodebuild.log")).map_err(|e| e.to_string())?;
        let mut xcodebuild = Command::new("xcodebuild")
            .current_dir(app_dir)
            .args(["test", "-project", "T1SGroundStation.xcodeproj", "-scheme", "T1SGroundStation"])
            .arg("-destination")
            .arg(format!("platform=iOS Simulator,name={device}"))
            .args(["-derivedDataPath", "build/dd", "-only-testing:T1SUITests/Bridge/testBridge"])
            // xcodebuild hands TEST_RUNNER_* variables to the test, without the prefix.
            .env("TEST_RUNNER_T1S_BRIDGE", format!("127.0.0.1:{bridge_port}"))
            .env("TEST_RUNNER_T1S_CONTROL_PORT", control_port.to_string())
            .env("TEST_RUNNER_T1S_ADDRESS", address)
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| format!("couldn't start xcodebuild: {e}"))?;
        let begun = Instant::now();
        loop {
            match listener.accept() {
                Ok((conn, _)) => {
                    conn.set_nonblocking(false).map_err(|e| e.to_string())?;
                    conn.set_read_timeout(Some(Duration::from_secs(120))).map_err(|e| e.to_string())?;
                    return Ok(Bridge { conn: BufReader::new(conn), xcodebuild });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if let Ok(Some(status)) = xcodebuild.try_wait() {
                        return Err(format!(
                            "xcodebuild ended ({status}) before the test connected: see {}",
                            std::env::temp_dir().join("sim001_ios_expect_xcodebuild.log").display()
                        ));
                    }
                    if begun.elapsed() > Duration::from_secs(900) {
                        let _ = xcodebuild.kill();
                        return Err("the UI test didn't connect in 15 minutes".into());
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    /// One request, one answer: the text after `ok `, or the error after `err `.
    pub fn ask(&mut self, request: &str) -> Result<String, String> {
        writeln!(self.conn.get_mut(), "{request}").map_err(|e| format!("the bridge is gone: {e}"))?;
        let mut line = String::new();
        let n = self.conn.read_line(&mut line).map_err(|e| format!("the bridge didn't answer: {e}"))?;
        if n == 0 {
            return Err("the bridge closed".into());
        }
        let line = line.trim_end();
        match line.split_once(' ') {
            Some(("ok", rest)) => Ok(rest.to_string()),
            Some(("err", why)) => Err(why.to_string()),
            _ if line == "ok" => Ok(String::new()),
            _ => Err(format!("unexpected answer {line:.80}")),
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.ask("quit");
        let _ = self.xcodebuild.wait();
    }
}

/// Standard base64, as the test sends a screenshot.
pub fn base64_decode(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_decodes_with_and_without_padding() {
        assert_eq!(base64_decode("aGVsbG8="), b"hello");
        assert_eq!(base64_decode("aGVsbG8"), b"hello");
        assert_eq!(base64_decode("AAEC/w=="), [0, 1, 2, 255]);
    }
}
