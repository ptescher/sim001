//! Advisors beside a commander, at its ground station (plans/actors.md, step 4).
//!
//! An advisor ([`recreation_towers::Advise`]) is slow, and it is not in the commander's loop. Here
//! each one runs on a thread of its own, behind a mailbox ([`Runner`]): the newest map and hand go
//! in (an older one that was not looked at is dropped, never queued), and proposals come out when the
//! advisor has them. The commander only ever does two quick things: take what has arrived
//! ([`Advisors::take`], then `Commander::advise`) and show the advisors the map it decided on
//! ([`Advisors::show`]). Proposals that are stale when they arrive are dropped on the way.
//!
//! An advisor can also be a program of its own ([`ProcessAdvisor`]): the station starts a command
//! and speaks JSON lines with it (see [`ToAdvisor`]); [`serve`] is the other end, for any `Advise`.
//! A process that dies, hangs or writes nonsense is dropped with a line on stderr, and the game
//! goes on. A person's seat runs advisors the same way ([`attend`]) and the app lists what they
//! propose.

use crate::seat::{Seat, Status};
use recreation_towers::seat::Join;
use recreation_towers::{Advise, Hand, Lookahead, Order, Phase, Proposal, Rules, Team, TeamView};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::process::{Child, Command as Process, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// How long an advisor in a process has to answer one map before it is dropped.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(90);
/// How long a process has to answer the start.
pub const START_WITHIN: Duration = Duration::from_secs(20);

/// What the station says to an advisor in a process, one JSON line each. The answer to `Start` is
/// the line `"ok"`; the answer to `Look` is a list of [`Proposal`]s (maybe empty).
#[derive(Serialize, Deserialize)]
pub enum ToAdvisor {
    /// Once, at the start: the rules of the game and the team the advisor is for.
    Start { team: Team, rules: Rules },
    /// The newest map of the team and what its commander has in hand.
    Look { view: Box<TeamView>, hand: Hand },
}

/// Which advisor a commander has beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdvisorChoice {
    /// The lookahead, on a thread of the station (the default).
    Lookahead,
    /// None: the commander does not think ahead.
    None,
    /// A command, started with `sh -c`, that serves an advisor ([`serve`]; `towers_advisor`).
    Command(String),
}

impl std::str::FromStr for AdvisorChoice {
    type Err = String;

    fn from_str(s: &str) -> Result<AdvisorChoice, String> {
        match s.trim() {
            "" => Err("an advisor is lookahead, none or a command".into()),
            "lookahead" => Ok(AdvisorChoice::Lookahead),
            "none" => Ok(AdvisorChoice::None),
            command => Ok(AdvisorChoice::Command(command.into())),
        }
    }
}

impl AdvisorChoice {
    /// The advisors of this choice for `team`, each on a thread of its own. A command that does not
    /// start gives none, with a line on stderr.
    pub fn start(&self, rules: Rules, team: Team) -> Advisors {
        let mut advisors = Advisors::default();
        match self {
            AdvisorChoice::None => {}
            AdvisorChoice::Lookahead => advisors.add(Runner::start("lookahead", Box::new(Lookahead::new(rules)))),
            AdvisorChoice::Command(command) => match ProcessAdvisor::spawn(command, rules, team) {
                Ok(p) => advisors.add(Runner::start(command, Box::new(p))),
                Err(e) => eprintln!("advisor {command}: not started: {e}"),
            },
        }
        advisors
    }
}

struct Slot {
    next: Option<(TeamView, Hand)>,
    out: Vec<Proposal>,
    stop: bool,
    alive: bool,
}

struct Shared {
    slot: Mutex<Slot>,
    wake: Condvar,
}

/// An advisor on a thread of its own, behind a mailbox.
pub struct Runner {
    name: String,
    shared: Arc<Shared>,
}

impl Runner {
    /// Starts `advisor` on a thread. It looks at each map it is shown, one at a time.
    pub fn start(name: impl Into<String>, mut advisor: Box<dyn Advise>) -> Runner {
        let name = name.into();
        let shared = Arc::new(Shared {
            slot: Mutex::new(Slot { next: None, out: vec![], stop: false, alive: true }),
            wake: Condvar::new(),
        });
        let (s, label) = (shared.clone(), name.clone());
        std::thread::Builder::new()
            .name(format!("advisor-{name}"))
            .spawn(move || {
                loop {
                    let (view, hand) = {
                        let mut slot = s.slot.lock().unwrap();
                        while slot.next.is_none() && !slot.stop {
                            slot = s.wake.wait(slot).unwrap();
                        }
                        if slot.stop {
                            return;
                        }
                        slot.next.take().expect("a map")
                    };
                    let said = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| advisor.advise(&view, &hand)));
                    let mut slot = s.slot.lock().unwrap();
                    match said {
                        Ok(proposals) => slot.out.extend(proposals),
                        Err(_) => {
                            eprintln!("advisor {label}: dropped: it panicked");
                            slot.alive = false;
                            return;
                        }
                    }
                }
            })
            .expect("an advisor thread");
        Runner { name, shared }
    }

    /// Puts the newest map and hand in the mailbox. It does not wait: a map in the mailbox that the
    /// advisor has not looked at yet is replaced.
    pub fn show(&self, view: &TeamView, hand: &Hand) {
        let mut slot = self.shared.slot.lock().unwrap();
        if slot.alive {
            slot.next = Some((view.clone(), hand.clone()));
            self.shared.wake.notify_one();
        }
    }

    /// The proposals that have arrived since the last call.
    pub fn take(&self) -> Vec<Proposal> {
        std::mem::take(&mut self.shared.slot.lock().unwrap().out)
    }

    /// The advisor goes on (its thread has not ended on a panic).
    pub fn alive(&self) -> bool {
        self.shared.slot.lock().unwrap().alive
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.shared.slot.lock().unwrap().stop = true;
        self.shared.wake.notify_one();
    }
}

/// The advisors beside one commander.
#[derive(Default)]
pub struct Advisors {
    runners: Vec<Runner>,
}

impl Advisors {
    pub fn add(&mut self, runner: Runner) {
        self.runners.push(runner);
    }

    pub fn len(&self) -> usize {
        self.runners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runners.is_empty()
    }

    /// Shows every advisor the map and the hand. Never waits.
    pub fn show(&self, view: &TeamView, hand: &Hand) {
        for r in &self.runners {
            r.show(view, hand);
        }
    }

    /// The proposals that have arrived and are not stale at game time `now`. A stale one is dropped.
    pub fn take(&self, now: f32) -> Vec<Proposal> {
        let mut out: Vec<Proposal> = self.runners.iter().flat_map(Runner::take).collect();
        out.retain(|p| p.until >= now);
        out
    }
}

/// An advisor in a process of its own: a command, started by the station, that serves an advisor
/// ([`serve`]). One JSON line goes in for each map, one comes out.
pub struct ProcessAdvisor {
    name: String,
    rules: Rules,
    team: Team,
    command: Option<String>,
    child: Option<Child>,
    to: Option<Box<dyn Write + Send>>,
    lines: Receiver<String>,
    timeout: Duration,
    dropped: Option<String>,
}

impl std::fmt::Debug for ProcessAdvisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProcessAdvisor({}, {})", self.name, self.dropped.as_deref().unwrap_or("running"))
    }
}

impl ProcessAdvisor {
    /// Starts `command` (with `sh -c`; `towers_advisor` is found beside this program too) and
    /// sends it the rules. An error if it does not answer.
    pub fn spawn(command: &str, rules: Rules, team: Team) -> Result<ProcessAdvisor, String> {
        ProcessAdvisor::spawn_within(command, rules, team, START_WITHIN)
    }

    /// The same, with `start` as the time the process has to answer the start.
    pub fn spawn_within(command: &str, rules: Rules, team: Team, start: Duration) -> Result<ProcessAdvisor, String> {
        let mut process = Process::new("sh");
        process.arg("-c").arg(format!("exec {command}"));
        // `towers_advisor`, beside this program, is on the way.
        if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) {
            let mut path = vec![dir];
            path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
            if let Ok(path) = std::env::join_paths(path) {
                process.env("PATH", path);
            }
        }
        let mut child = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("{command}: {e}"))?;
        let to = child.stdin.take().expect("a pipe in");
        let from = std::io::BufReader::new(child.stdout.take().expect("a pipe out"));
        let mut a = ProcessAdvisor::over_within(Box::new(to), Box::new(from), command, rules, team, start)?;
        a.command = Some(command.to_string());
        a.child = Some(child);
        Ok(a)
    }

    /// An advisor at the other end of a pair of streams (a thread that runs [`serve`], in a test).
    pub fn over(
        to: Box<dyn Write + Send>,
        from: Box<dyn BufRead + Send>,
        name: &str,
        rules: Rules,
        team: Team,
    ) -> Result<ProcessAdvisor, String> {
        ProcessAdvisor::over_within(to, from, name, rules, team, START_WITHIN)
    }

    fn over_within(
        to: Box<dyn Write + Send>,
        from: Box<dyn BufRead + Send>,
        name: &str,
        rules: Rules,
        team: Team,
        start: Duration,
    ) -> Result<ProcessAdvisor, String> {
        let (tx, lines) = channel();
        std::thread::Builder::new()
            .name(format!("advisor-{name}-lines"))
            .spawn(move || {
                for line in from.lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        return;
                    }
                }
            })
            .expect("a reader thread");
        let mut a = ProcessAdvisor {
            name: name.to_string(),
            rules,
            team,
            command: None,
            child: None,
            to: Some(to),
            lines,
            timeout: ANSWER_WITHIN,
            dropped: None,
        };
        match a.ask(&ToAdvisor::Start { team, rules }, start) {
            Ok(line) if line.trim() == "\"ok\"" => Ok(a),
            Ok(line) => Err(format!("the advisor answered the start with {line:?}")),
            Err(e) => Err(e),
        }
    }

    /// The time the process has to answer one map (default [`ANSWER_WITHIN`]).
    pub fn with_timeout(mut self, timeout: Duration) -> ProcessAdvisor {
        self.timeout = timeout;
        self
    }

    /// Why the advisor was dropped, if it was.
    pub fn dropped(&self) -> Option<&str> {
        self.dropped.as_deref()
    }

    fn ask(&mut self, msg: &ToAdvisor, within: Duration) -> Result<String, String> {
        let to = self.to.as_mut().ok_or("the advisor's process is gone")?;
        let line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        writeln!(to, "{line}").and_then(|_| to.flush()).map_err(|e| format!("the advisor's process ended ({e})"))?;
        match self.lines.recv_timeout(within) {
            Ok(line) => Ok(line),
            Err(RecvTimeoutError::Timeout) => Err(format!("the advisor did not answer in {} s", within.as_secs_f32())),
            Err(RecvTimeoutError::Disconnected) => Err("the advisor's process ended".into()),
        }
    }

    /// Drops the advisor for good, with a line on stderr.
    fn drop_with(&mut self, why: String) {
        eprintln!("advisor {}: dropped: {why}", self.name);
        self.dropped = Some(why);
        self.to = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Advise for ProcessAdvisor {
    fn advise(&mut self, view: &TeamView, hand: &Hand) -> Vec<Proposal> {
        if self.dropped.is_some() {
            return vec![];
        }
        let msg = ToAdvisor::Look { view: Box::new(view.clone()), hand: hand.clone() };
        let line = match self.ask(&msg, self.timeout) {
            Ok(line) => line,
            Err(e) => {
                self.drop_with(e);
                return vec![];
            }
        };
        let proposals: Vec<Proposal> = match serde_json::from_str(&line) {
            Ok(p) => p,
            Err(e) => {
                let said: String = line.chars().take(80).collect();
                self.drop_with(format!("it said {said:?}, which is not a list of proposals ({e})"));
                return vec![];
            }
        };
        if let Some(bad) = proposals.iter().find(|p| !sane(p, view)) {
            self.drop_with(format!("it proposed {bad:?}, which is not for a drone and a tower of this game"));
            return vec![];
        }
        proposals
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        // A process is not copied: the copy starts its own, if it was started from a command.
        match self.command.as_deref().map(|c| ProcessAdvisor::spawn(c, self.rules, self.team)) {
            Some(Ok(p)) => Box::new(p),
            _ => {
                let (_, lines) = channel();
                Box::new(ProcessAdvisor {
                    name: self.name.clone(),
                    rules: self.rules,
                    team: self.team,
                    command: None,
                    child: None,
                    to: None,
                    lines,
                    timeout: self.timeout,
                    dropped: Some("not copied".into()),
                })
            }
        }
    }
}

impl Drop for ProcessAdvisor {
    fn drop(&mut self) {
        self.to = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// A proposal that names a drone of the team, a tower of the game and a gain that is a number.
pub fn sane(p: &Proposal, view: &TeamView) -> bool {
    p.drone < view.drones.len()
        && p.gain.is_finite()
        && p.made.is_finite()
        && p.until.is_finite()
        && match p.order {
            Order::Take(t) => t < view.towers.len(),
            _ => true,
        }
}

/// The other end of a [`ProcessAdvisor`]: reads [`ToAdvisor`] lines from `input` and answers each on
/// `output`, with the advisor `make` builds from the rules at the start. Any [`Advise`] can be a
/// process this way. Returns when `input` ends, or on a line that is not a [`ToAdvisor`].
pub fn serve(
    input: impl BufRead,
    mut output: impl Write,
    make: impl Fn(Rules, Team) -> Box<dyn Advise>,
) -> Result<(), String> {
    let mut advisor: Option<Box<dyn Advise>> = None;
    for line in input.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let reply = match serde_json::from_str::<ToAdvisor>(&line).map_err(|e| e.to_string())? {
            ToAdvisor::Start { team, rules } => {
                advisor = Some(make(rules, team));
                "\"ok\"".to_string()
            }
            ToAdvisor::Look { view, hand } => {
                let a = advisor.as_mut().ok_or("a map before the start")?;
                serde_json::to_string(&a.advise(&view, &hand)).map_err(|e| e.to_string())?
            }
        };
        writeln!(output, "{reply}").and_then(|_| output.flush()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// What a person's seat has in hand, for advisors: the orders the map shows, and the drones that
/// are up and in contact. A person's seat keeps no record of towers given up.
pub fn hand_of(view: &TeamView) -> Hand {
    Hand {
        orders: view.drones.iter().map(|d| d.order).collect(),
        free: view.drones.iter().map(|d| d.linked && d.phase != Phase::Down).collect(),
        away: vec![vec![false; view.towers.len()]; view.drones.len()],
    }
}

/// The proposals to list for a person at game time `view.time`: not stale, and not for an order the
/// drone already has; the one that gains most first.
pub fn listed<'a>(proposals: &'a [Proposal], view: &TeamView) -> Vec<&'a Proposal> {
    let mut live: Vec<&Proposal> = proposals
        .iter()
        .filter(|p| p.until >= view.time && view.drones.get(p.drone).is_some_and(|d| d.order != Some(p.order)))
        .collect();
    live.sort_by(|a, b| b.gain.total_cmp(&a.gain));
    live
}

/// Runs advisors for a seat that a person commands, on a thread of its own, until the seat is given
/// up: what they propose is in `seat.state().proposals` (`Seat::accept` takes one). Advisors are
/// started when the seat is seated at a commander's place and stopped when it is not.
pub fn attend(seat: Arc<Seat>, choice: AdvisorChoice) {
    std::thread::Builder::new()
        .name("attend".into())
        .spawn(move || {
            let mut advisors: Option<Advisors> = None;
            let mut shown = f32::NEG_INFINITY;
            while !seat.stopped() {
                std::thread::sleep(Duration::from_millis(50));
                let (status, rules, view) = {
                    let s = seat.state();
                    (s.status.clone(), s.rules, s.view.clone())
                };
                let (Status::Seated(Join::Command(team)), Some(rules), Some(view)) = (status, rules, view) else {
                    advisors = None;
                    seat.state().proposals.clear();
                    continue;
                };
                let advisors = advisors.get_or_insert_with(|| choice.start(rules, team));
                if view.time < shown {
                    // A new game on the same seat.
                    seat.state().proposals.clear();
                }
                if view.time != shown {
                    shown = view.time;
                    advisors.show(&view, &hand_of(&view));
                }
                let arrived = advisors.take(view.time);
                let mut s = seat.state();
                s.proposals.extend(arrived);
                s.proposals.retain(|p| p.until >= view.time);
            }
        })
        .expect("an attending thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use recreation_towers::{Game, Phase};

    fn view() -> TeamView {
        let mut g = Game::new(Rules::default());
        for k in 0..g.rules.drones {
            let i = g.drone(Team::Blue, k);
            g.drones[i].phase = Phase::Flying;
        }
        g.time = 20.0;
        g.view(Team::Blue)
    }

    fn take(drone: usize, tower: usize, gain: f32, until: f32) -> Proposal {
        Proposal { drone, order: Order::Take(tower), queue: vec![], gain, made: 20.0, until, why: String::new() }
    }

    #[test]
    fn the_panel_lists_the_live_proposals_best_first_and_not_the_stale_or_the_done() {
        let mut v = view();
        v.drones[2].order = Some(Order::Take(3));
        let all = vec![take(0, 1, 3.0, 30.0), take(1, 2, 8.0, 30.0), take(0, 2, 9.0, 10.0), take(2, 3, 7.0, 30.0)];
        let shown: Vec<(usize, f32)> = listed(&all, &v).iter().map(|p| (p.drone, p.gain)).collect();
        // Drone 0's 9-point proposal is stale (until 10, now 20); drone 2 already has its order.
        assert_eq!(shown, vec![(1, 8.0), (0, 3.0)]);
        v.time = 31.0;
        assert!(listed(&all, &v).is_empty(), "all are stale after their time");
    }

    #[test]
    fn a_person_s_hand_is_the_orders_on_the_map_and_the_drones_in_contact() {
        let mut v = view();
        v.drones[1].linked = false;
        v.drones[3].phase = Phase::Down;
        v.drones[0].order = Some(Order::Hold);
        let h = hand_of(&v);
        assert_eq!(h.orders[0], Some(Order::Hold));
        assert_eq!(h.free, vec![true, false, true, false, true][..v.drones.len()].to_vec());
        assert_eq!(h.away.len(), v.drones.len());
    }

    #[test]
    fn a_proposal_for_a_tower_that_is_not_in_the_game_is_not_sane() {
        let v = view();
        assert!(sane(&take(0, 1, 5.0, 30.0), &v));
        assert!(!sane(&take(0, 99, 5.0, 30.0), &v));
        assert!(!sane(&take(99, 1, 5.0, 30.0), &v));
        assert!(!sane(&take(0, 1, f32::NAN, 30.0), &v));
    }
}
