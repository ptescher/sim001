//! Running a program with a time limit.

use std::process::{Command, ExitStatus};
use std::time::{Duration, Instant};

/// How a limited run ended.
#[derive(Debug, PartialEq)]
pub enum Ended {
    Exited(i32),
    /// It ran out of time and was killed.
    TimedOut,
    CouldNotStart(String),
}

impl Ended {
    pub fn ok(&self) -> bool {
        matches!(self, Ended::Exited(0))
    }
}

fn code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

/// Runs `cmd` for at most `limit`, then kills it (its children are left to its own cleanup:
/// the programs run here stop what they started when they are killed).
pub fn run(cmd: &mut Command, limit: Duration) -> Ended {
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ended::CouldNotStart(e.to_string()),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ended::Exited(code(status)),
            Ok(None) if start.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Ended::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Ended::CouldNotStart(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_finishes_gives_its_exit_code() {
        assert_eq!(run(&mut Command::new("true"), Duration::from_secs(5)), Ended::Exited(0));
        assert_eq!(run(&mut Command::new("false"), Duration::from_secs(5)), Ended::Exited(1));
        assert!(run(&mut Command::new("true"), Duration::from_secs(5)).ok());
    }

    #[test]
    fn a_program_that_runs_too_long_is_killed() {
        let began = Instant::now();
        assert_eq!(run(Command::new("sleep").arg("30"), Duration::from_millis(300)), Ended::TimedOut);
        assert!(began.elapsed() < Duration::from_secs(5), "it waited the whole 30 s");
    }

    #[test]
    fn a_program_that_is_not_there_says_so() {
        match run(&mut Command::new("definitely-not-a-program-sim001"), Duration::from_secs(1)) {
            Ended::CouldNotStart(why) => assert!(!why.is_empty()),
            other => panic!("{other:?}"),
        }
    }
}
