//! The control server: the acceptance driver reads the station's state through it. One request
//! per line, one answer per line:
//!
//! ```text
//! observe <key>   ->  ok <value>  |  err <why>
//! ```

use crate::Station;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;

/// Serve `station` on `127.0.0.1:port` (0 picks one). Returns the address it listens on.
pub fn serve(station: Arc<Station>, port: u16) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let addr = listener.local_addr()?;
    std::thread::Builder::new().name("control".into()).spawn(move || {
        for conn in listener.incoming().flatten() {
            let station = station.clone();
            std::thread::spawn(move || client(station, conn));
        }
    })?;
    Ok(addr)
}

fn client(station: Arc<Station>, conn: TcpStream) {
    let Ok(mut out) = conn.try_clone() else { return };
    for line in BufReader::new(conn).lines().map_while(Result::ok) {
        let answer = match line.split_once(' ') {
            Some(("observe", key)) => match station.observe(key.trim()) {
                Some(v) => format!("ok {v}"),
                None => format!("err no state called {}", key.trim()),
            },
            _ => format!("err unknown request {line}"),
        };
        if writeln!(out, "{answer}").is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(addr: SocketAddr, request: &str) -> String {
        let mut c = TcpStream::connect(addr).unwrap();
        writeln!(c, "{request}").unwrap();
        let mut line = String::new();
        BufReader::new(c).read_line(&mut line).unwrap();
        line.trim().to_string()
    }

    #[test]
    fn it_answers_with_the_stations_state() {
        let s = Arc::new(Station::start("tcpout:127.0.0.1:1"));
        let addr = serve(s, 0).unwrap();
        assert_eq!(ask(addr, "observe link"), "ok lost");
        assert_eq!(ask(addr, "observe autopilot mode"), "ok ----");
        assert!(ask(addr, "observe nonsense").starts_with("err "));
        assert!(ask(addr, "dance").starts_with("err "));
    }
}
