//! An advisor that asks Claude: every few seconds of the game it gives `claude -p` (headless, JSON
//! output, a small budget, no tools) a text picture of the team's map and what its commander has in
//! hand, and asks for at most two proposals as JSON. The reply is read strictly: a proposal for a
//! drone or a tower that is not in the game, or with no gain, is dropped. The advisor stamps
//! `made` and `until` itself, from the map it showed.
//!
//! The command is `claude`, or what `SIM001_CLAUDE` names (as the league runs it, and a stand-in in
//! the tests: nothing here is run against the real thing by a test). It runs in a process of its own
//! with the session's `CLAUDE*` variables taken out, so that it uses its own login.

use recreation_towers::{Advise, Class, Hand, Kind, Order, Phase, Proposal, Rules, Team, TeamView};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

/// Claude is asked this often (s of the game).
pub const EVERY: f32 = 8.0;
/// ...and not before this time in the game (s): at the start the map shows little.
pub const AFTER: f32 = 10.0;
/// A proposal of Claude is good this long (s of the game), counted from the map it was made on.
pub const VALID_FOR: f32 = 30.0;
/// At most this many proposals are read from a reply.
pub const MOST: usize = 2;
/// The most a gain may claim (points): a number Claude makes up is not to outweigh the rules.
pub const MOST_GAIN: f32 = 50.0;
/// One question may cost at most this much (US dollars).
pub const BUDGET_USD: f32 = 0.25;
/// One question may take this long.
pub const WITHIN: Duration = Duration::from_secs(60);

/// The advisor.
#[derive(Clone, Debug)]
pub struct ClaudeAdvisor {
    /// The `claude` command, or a stand-in.
    pub command: PathBuf,
    pub model: Option<String>,
    pub budget_usd: f32,
    pub within: Duration,
    /// Asked this often (s of the game).
    pub every: f32,
    pub valid_for: f32,
    last: f32,
}

impl ClaudeAdvisor {
    /// With `claude`, or `SIM001_CLAUDE`.
    pub fn new() -> ClaudeAdvisor {
        let command = std::env::var_os("SIM001_CLAUDE").map(PathBuf::from).unwrap_or_else(|| "claude".into());
        ClaudeAdvisor::with_command(command)
    }

    pub fn with_command(command: PathBuf) -> ClaudeAdvisor {
        ClaudeAdvisor {
            command,
            model: None,
            budget_usd: BUDGET_USD,
            within: WITHIN,
            every: EVERY,
            valid_for: VALID_FOR,
            last: f32::NEG_INFINITY,
        }
    }

    /// Runs `claude --model NAME`.
    pub fn with_model(mut self, model: &str) -> ClaudeAdvisor {
        self.model = Some(model.to_string());
        self
    }

    /// The command line, but for the prompt.
    fn process(&self, prompt: &str) -> Process {
        let mut cmd = Process::new(&self.command);
        cmd.arg("-p")
            .arg(prompt)
            .args(["--output-format", "json", "--permission-mode", "dontAsk", "--no-session-persistence"])
            .args(["--tools", ""])
            .args(["--max-budget-usd", &self.budget_usd.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(m) = &self.model {
            cmd.args(["--model", m]);
        }
        own_login(&mut cmd, std::env::vars_os().map(|(n, _)| n.to_string_lossy().into_owned()));
        cmd
    }

    /// Asks Claude, and returns the text of its answer (the `result` of `--output-format json`).
    pub fn ask(&self, prompt: &str) -> Result<String, String> {
        let mut child = self.process(prompt).spawn().map_err(|e| format!("{}: {e}", self.command.display()))?;
        let mut out = child.stdout.take().expect("a pipe");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut bytes = vec![];
            let _ = out.read_to_end(&mut bytes);
            let _ = tx.send(bytes);
        });
        let began = Instant::now();
        let bytes = match rx.recv_timeout(self.within) {
            Ok(b) => b,
            Err(RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer in {} s", self.within.as_secs_f32()));
            }
            Err(RecvTimeoutError::Disconnected) => vec![],
        };
        let _ = child.wait();
        let v: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("not JSON after {:.0} s: {e}", began.elapsed().as_secs_f32()))?;
        if v.get("is_error").and_then(|e| e.as_bool()) == Some(true) {
            return Err(format!("an error: {}", v.get("result").and_then(|r| r.as_str()).unwrap_or("")));
        }
        v.get("result").and_then(|r| r.as_str()).map(str::to_string).ok_or_else(|| "no result in its answer".into())
    }
}

impl Default for ClaudeAdvisor {
    fn default() -> ClaudeAdvisor {
        ClaudeAdvisor::new()
    }
}

/// Takes the session's own login variables out of the environment of `cmd`: started from inside a
/// Claude session, they point at that session's auth and endpoint, and `claude` must use its own.
pub fn own_login(cmd: &mut Process, names: impl IntoIterator<Item = String>) {
    for n in names {
        if n.starts_with("CLAUDE") || n == "ANTHROPIC_BASE_URL" {
            cmd.env_remove(&n);
        }
    }
}

impl Advise for ClaudeAdvisor {
    fn advise(&mut self, view: &TeamView, hand: &Hand) -> Vec<Proposal> {
        if view.time < AFTER || view.time - self.last < self.every {
            return vec![];
        }
        self.last = view.time;
        // The game may have gone on while Claude thought: the proposals are stamped with the map
        // they were made on, and the commander drops them when they are stale.
        match self.ask(&prompt(view, hand, self.valid_for)) {
            Ok(text) => parse(&text, view, hand, self.valid_for),
            Err(e) => {
                eprintln!("claude advisor: no proposals this time: {e}");
                vec![]
            }
        }
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}

fn holder(h: Option<Team>) -> &'static str {
    match h {
        Some(Team::Blue) => "Blue",
        Some(Team::Red) => "Red",
        None => "nobody",
    }
}

fn kind(k: Kind) -> &'static str {
    match k {
        Kind::Fort => "home",
        Kind::Spire => "spire (the most points)",
        Kind::Depot => "depot (recharges drones)",
        _ => "relay",
    }
}

fn order(o: Option<Order>, view: &TeamView) -> String {
    match o {
        None => "none".into(),
        Some(Order::Take(t)) => format!("take the {} tower", view.towers.get(t).map_or("?", |t| t.name.as_str())),
        Some(Order::Goto(p)) => format!("go to {:.0},{:.0}", p.x, p.y),
        Some(Order::Circle(p, r)) => format!("circle {:.0},{:.0} at {r:.0} m", p.x, p.y),
        Some(Order::Hold) => "hold".into(),
        Some(Order::Home) => "home".into(),
        Some(Order::Land) => "land for a battery swap".into(),
    }
}

/// A compact text picture of the map and the hand, for a model to read. Drones are numbered from 1.
pub fn picture(view: &TeamView, hand: &Hand) -> String {
    let rules = Rules::default();
    let mut s = format!(
        "You are the advisor of the {:?} team. Time {:.0} s, {:.0} s left. Score: Blue {:.0}, Red {:.0}.\n",
        view.team, view.time, view.time_left, view.score[0], view.score[1]
    );
    s.push_str("Positions are metres north, east of the centre of the field.\nTowers:\n");
    for t in &view.towers {
        let last_seen = if t.seen { String::new() } else { format!(", last seen {:.0} s ago", t.age) };
        s.push_str(&format!(
            "- {} ({}) at {:.0},{:.0}: held by {}, control {:+.0}% for Blue{}{}\n",
            t.name,
            kind(t.kind),
            t.at.x,
            t.at.y,
            holder(t.holder),
            100.0 * t.control,
            if t.contested { ", contested" } else { "" },
            last_seen
        ));
    }
    s.push_str("Your drones:\n");
    for d in &view.drones {
        let class = match rules.class(d.k) {
            Class::Light => "light",
            Class::Standard => "standard",
            Class::Heavy => "heavy",
        };
        let state = match (d.phase, d.linked) {
            (Phase::Down, _) => "down".to_string(),
            (Phase::Ground, _) => "on the ground".to_string(),
            (_, false) => format!("out of contact for {:.0} s", d.silent),
            _ => "flying".to_string(),
        };
        s.push_str(&format!(
            "- drone {} ({class}) at {:.0},{:.0}, {state}, battery {:.0} s, order: {}{}\n",
            d.k + 1,
            d.at.x,
            d.at.y,
            d.battery,
            order(hand.orders.get(d.k).copied().flatten(), view),
            if hand.free.get(d.k).copied().unwrap_or(false) { "" } else { " (cannot be given a new order now)" }
        ));
    }
    let seen: Vec<_> = view.enemies.iter().filter(|e| e.seen || e.age < 30.0).collect();
    if seen.is_empty() {
        s.push_str("Enemy drones seen lately: none.\n");
    } else {
        s.push_str("Enemy drones seen lately:\n");
        for e in seen {
            s.push_str(&format!(
                "- enemy {} at {:.0},{:.0}, {}\n",
                e.k + 1,
                e.at.x,
                e.at.y,
                if e.seen { "in sight now".to_string() } else { format!("last seen {:.0} s ago", e.age) }
            ));
        }
    }
    s
}

/// The question: the picture, what is asked, and the shape of the answer.
pub fn prompt(view: &TeamView, hand: &Hand, valid_for: f32) -> String {
    format!(
        "You help the commander of a team in Towers, a game of two teams of drones. A team scores \
         points each second for each tower it holds (the spire more). Drones take a tower by \
         circling it low; a tower flips when more of your drones than theirs circle it. Do not use \
         tools.\n\n{}\nPropose at most {MOST} orders that would win the team more points over the next \
         {valid_for:.0} s than its commander's present orders. Only propose for a drone that can be \
         given an order, and only to take a tower.\n\nAnswer with only a JSON list, nothing else, for \
         example [{{\"drone\": 2, \"tower\": \"centre\", \"gain\": 6, \"why\": \"the centre is \
         undefended\"}}]. drone is the number above, tower is a tower's name above, gain is the \
         points you expect to win, why is one short sentence. Answer [] if the orders are good.",
        picture(view, hand)
    )
}

/// The proposals in a reply, strictly: the first [`MOST`] items that name a drone that can take an
/// order and a tower of the game, with a gain over 0. Anything else is dropped. `made` and `until`
/// are stamped here, from the map.
pub fn parse(reply: &str, view: &TeamView, hand: &Hand, valid_for: f32) -> Vec<Proposal> {
    let (Some(a), Some(b)) = (reply.find('['), reply.rfind(']')) else { return vec![] };
    let Some(items) = reply.get(a..=b).and_then(|j| serde_json::from_str::<Vec<serde_json::Value>>(j).ok()) else {
        return vec![];
    };
    let mut out: Vec<Proposal> = vec![];
    for item in items {
        let Some(drone) = item.get("drone").and_then(|d| d.as_u64()).and_then(|d| (d as usize).checked_sub(1)) else {
            continue;
        };
        let Some(tower) = item
            .get("tower")
            .and_then(|t| t.as_str())
            .and_then(|name| view.towers.iter().position(|t| t.name.eq_ignore_ascii_case(name.trim())))
        else {
            continue;
        };
        let Some(gain) =
            item.get("gain").and_then(|g| g.as_f64()).map(|g| g as f32).filter(|g| g.is_finite() && *g > 0.0)
        else {
            continue;
        };
        let order = Order::Take(tower);
        if drone >= view.drones.len()
            || !hand.free.get(drone).copied().unwrap_or(false)
            || hand.orders.get(drone).copied().flatten() == Some(order)
            || out.iter().any(|p| p.drone == drone)
        {
            continue;
        }
        let why = item.get("why").and_then(|w| w.as_str()).unwrap_or("").trim().chars().take(160).collect::<String>();
        out.push(Proposal {
            drone,
            order,
            queue: vec![],
            gain: gain.min(MOST_GAIN),
            made: view.time,
            until: view.time + valid_for,
            why: if why.is_empty() {
                format!("Claude: send drone {} to the {} tower.", drone + 1, view.towers[tower].name)
            } else {
                format!("Claude: {why}")
            },
        });
        if out.len() == MOST {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use recreation_towers::Game;
    use std::os::unix::fs::PermissionsExt;

    fn view() -> (TeamView, Hand) {
        let mut g = Game::new(Rules::default());
        for k in 0..g.rules.drones {
            let i = g.drone(Team::Blue, k);
            g.drones[i].phase = Phase::Flying;
        }
        g.time = 30.0;
        let v = g.view(Team::Blue);
        let n = v.drones.len();
        let hand = Hand { orders: vec![None; n], free: vec![true; n], away: vec![vec![false; v.towers.len()]; n] };
        (v, hand)
    }

    /// A stand-in for `claude` that prints `body`'s output.
    fn stand_in(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("claude-stand-in-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn canned(result: &str) -> String {
        let reply = serde_json::json!({"type": "result", "is_error": false, "result": result, "total_cost_usd": 0.01});
        format!("cat <<'EOF'\n{reply}\nEOF")
    }

    #[test]
    fn the_picture_names_the_drones_the_towers_and_what_is_in_hand() {
        let (mut v, mut hand) = view();
        hand.orders[0] = Some(Order::Take(1));
        hand.free[2] = false;
        v.score = [40.0, 55.0];
        let p = picture(&v, &hand);
        assert!(p.contains("Blue team") && p.contains("Score: Blue 40, Red 55"), "{p}");
        assert!(p.contains("- west (") && p.contains("- drone 1 (") && p.contains("- drone 5"), "{p}");
        assert!(p.contains(&format!("order: take the {} tower", v.towers[1].name)), "{p}");
        assert!(p.contains("(cannot be given a new order now)"), "{p}");
        assert!(prompt(&v, &hand, 30.0).contains("JSON list"));
    }

    #[test]
    fn a_good_reply_is_read_and_stamped_with_the_map_s_time() {
        let (v, hand) = view();
        let reply = r#"Here you go:
```json
[{"drone": 2, "tower": "West", "gain": 6, "why": "the west is open"}, {"drone": 3, "tower": "centre", "gain": 400}]
```"#;
        let p = parse(reply, &v, &hand, 30.0);
        assert_eq!(p.len(), 2);
        let west = v.towers.iter().position(|t| t.name == "west").unwrap();
        assert_eq!((p[0].drone, p[0].order, p[0].gain), (1, Order::Take(west), 6.0));
        assert_eq!((p[0].made, p[0].until), (30.0, 60.0));
        assert!(p[0].why.contains("the west is open"));
        assert_eq!(p[1].gain, MOST_GAIN, "a gain is held to what the rules can be worth");
    }

    #[test]
    fn a_reply_is_read_strictly() {
        let (v, mut hand) = view();
        let one = |s: &str| parse(s, &v, &hand, 30.0);
        assert!(one("no list here").is_empty());
        assert!(one("[{\"drone\": 1, \"tower\": \"west\"").is_empty(), "cut off");
        assert!(one(r#"[{"drone": 99, "tower": "west", "gain": 5}]"#).is_empty(), "unknown drone");
        assert!(one(r#"[{"drone": 0, "tower": "west", "gain": 5}]"#).is_empty(), "drones count from 1");
        assert!(one(r#"[{"drone": 1, "tower": "moon", "gain": 5}]"#).is_empty(), "unknown tower");
        assert!(one(r#"[{"drone": 1, "tower": "west"}]"#).is_empty(), "no gain");
        assert!(one(r#"[{"drone": 1, "tower": "west", "gain": "lots"}]"#).is_empty(), "gain that is no number");
        assert!(one(r#"[{"drone": 1, "tower": "west", "gain": -3}]"#).is_empty(), "no gain over nothing");
        assert!(one(r#"[{"drone": "one", "tower": "west", "gain": 5}]"#).is_empty());
        // A good one among bad ones is kept; two for the same drone are one; three are two.
        let mixed = r#"[{"drone": 99, "tower": "west", "gain": 5}, {"drone": 1, "tower": "west", "gain": 5},
            {"drone": 1, "tower": "east", "gain": 9}, {"drone": 2, "tower": "east", "gain": 5}, {"drone": 3, "tower": "east", "gain": 5}]"#;
        assert_eq!(one(mixed).iter().map(|p| p.drone).collect::<Vec<_>>(), vec![0, 1]);
        // A drone that cannot take an order gets none.
        hand.free[0] = false;
        assert!(parse(r#"[{"drone": 1, "tower": "west", "gain": 5}]"#, &v, &hand, 30.0).is_empty());
    }

    #[test]
    fn it_asks_the_command_and_reads_what_it_proposes() {
        let (v, hand) = view();
        let good = stand_in("good", &canned(r#"[{"drone": 2, "tower": "east", "gain": 7, "why": "east is open"}]"#));
        let mut a = ClaudeAdvisor::with_command(good);
        let p = a.advise(&v, &hand);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].drone, p[0].gain), (1, 7.0));
        // Asked again at once, it does not ask: every few seconds.
        assert!(a.advise(&v, &hand).is_empty());
        let mut later = v.clone();
        later.time += EVERY;
        assert_eq!(a.advise(&later, &hand).len(), 1);
    }

    #[test]
    fn it_is_asked_with_headless_json_a_budget_and_no_tools() {
        let (v, hand) = view();
        let args = std::env::temp_dir().join(format!("claude-args-{}", std::process::id()));
        let spy = stand_in("spy", &format!("printf '%s\\n' \"$@\" > {}\n{}", args.display(), canned("[]")));
        let a = ClaudeAdvisor::with_command(spy);
        a.ask(&prompt(&v, &hand, 30.0)).unwrap();
        let args = std::fs::read_to_string(args).unwrap();
        let args: Vec<&str> = args.lines().collect();
        assert_eq!(args[0], "-p");
        for want in ["--output-format", "json", "--max-budget-usd", "--tools", "--no-session-persistence"] {
            assert!(args.contains(&want), "{want} in {args:?}");
        }
    }

    #[test]
    fn a_reply_that_is_not_json_or_an_error_or_late_gives_no_proposals() {
        let (v, hand) = view();
        for (name, body) in [
            ("malformed", "echo 'this is not what claude prints'".to_string()),
            ("empty", "true".to_string()),
            ("error", canned("x").replace("\"is_error\":false", "\"is_error\":true")),
            ("nonsense-result", canned("I think you should attack!")),
            ("missing-command-output", "exit 1".to_string()),
        ] {
            let mut a = ClaudeAdvisor::with_command(stand_in(name, &body));
            assert!(a.advise(&v, &hand).is_empty(), "{name}");
        }
        let mut gone = ClaudeAdvisor::with_command("/no/such/claude".into());
        assert!(gone.advise(&v, &hand).is_empty());
        let mut slow = ClaudeAdvisor::with_command(stand_in("slow", &format!("sleep 5\n{}", canned("[]"))));
        slow.within = Duration::from_millis(300);
        let t = Instant::now();
        assert!(slow.advise(&v, &hand).is_empty());
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    }

    #[test]
    fn it_runs_with_its_own_login_not_the_session_it_was_started_from() {
        let mut cmd = Process::new("true");
        own_login(
            &mut cmd,
            ["PATH", "CLAUDE_CODE_OAUTH_TOKEN", "CLAUDECODE", "ANTHROPIC_BASE_URL", "SIM001_CLAUDE", "HOME"]
                .map(String::from),
        );
        let mut removed: Vec<String> =
            cmd.get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        removed.sort();
        assert_eq!(removed, ["ANTHROPIC_BASE_URL", "CLAUDECODE", "CLAUDE_CODE_OAUTH_TOKEN"]);
    }
}
