//! Advisors beside the commander (plans/actors.md, step 4): each runs on a thread of its own with a
//! mailbox, the commander never waits for one, and one in another process that dies, hangs or
//! writes nonsense is dropped.

use recreation_towers::{Advise, Commander, Game, Hand, Order, Phase, Proposal, Rules, Team, TeamView};
use recreation_towers_ground::advisors::{self, Advisors, ProcessAdvisor, Runner};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn game_with_drones_up() -> (Game, TeamView) {
    let mut g = Game::new(Rules::default());
    for k in 0..g.rules.drones {
        let i = g.drone(Team::Blue, k);
        g.drones[i].phase = Phase::Flying;
        g.drones[i].alt = 50.0;
    }
    g.time = 20.0;
    let v = g.view(Team::Blue);
    (g, v)
}

fn hand(view: &TeamView) -> Hand {
    let n = view.drones.len();
    Hand { orders: vec![None; n], free: vec![true; n], away: vec![vec![false; view.towers.len()]; n] }
}

fn proposal(view: &TeamView, drone: usize, tower: usize, gain: f32, until: f32) -> Proposal {
    Proposal { drone, order: Order::Take(tower), queue: vec![], gain, made: view.time, until, why: "test".into() }
}

/// An advisor that thinks for `think` and proposes a take of tower 1 by drone 0, good for `good` s.
#[derive(Clone, Debug)]
struct Slow {
    think: Duration,
    good: f32,
    looked: Arc<Mutex<Vec<f32>>>,
}

impl Advise for Slow {
    fn advise(&mut self, view: &TeamView, _: &Hand) -> Vec<Proposal> {
        self.looked.lock().unwrap().push(view.time);
        std::thread::sleep(self.think);
        vec![proposal(view, 0, 1, 9.0, view.time + self.good)]
    }

    fn clone_box(&self) -> Box<dyn Advise> {
        Box::new(self.clone())
    }
}

fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(x) = f() {
            return x;
        }
        assert!(Instant::now() < end, "waited for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn an_advisor_that_sleeps_for_seconds_does_not_delay_the_commander() {
    let (g, mut view) = game_with_drones_up();
    let looked = Arc::new(Mutex::new(vec![]));
    let mut advisors = Advisors::default();
    advisors.add(Runner::start(
        "slow",
        Box::new(Slow { think: Duration::from_secs(3), good: 4.0, looked: looked.clone() }),
    ));
    let mut commander = Commander::new(g.rules).with_lookahead(false);
    let began = Instant::now();
    let mut worst = Duration::ZERO;
    // Ten decisions, one for each game second; the wall clock runs 20 times faster.
    for second in 0..10 {
        let t = Instant::now();
        view.time = 20.0 + second as f32;
        commander.advise(advisors.take(view.time));
        let orders = commander.decide(&view);
        assert_eq!(orders.len(), g.rules.drones, "every drone has its order at second {second}");
        advisors.show(&view, commander.hand().expect("a hand after a decision"));
        worst = worst.max(t.elapsed());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(worst < Duration::from_millis(100), "the slowest decision took {worst:?}");
    assert!(began.elapsed() < Duration::from_secs(2), "ten decisions took {:?}", began.elapsed());
    // The advisor is still thinking about the first map; the others went into its mailbox.
    assert_eq!(looked.lock().unwrap().len(), 1);
}

#[test]
fn a_late_proposal_is_dropped_once_it_is_stale() {
    let (_, mut view) = game_with_drones_up();
    let mut advisors = Advisors::default();
    let looked = Arc::new(Mutex::new(vec![]));
    // Good for 4 s of the game, and it takes 300 ms to think.
    advisors.add(Runner::start("slow", Box::new(Slow { think: Duration::from_millis(300), good: 4.0, looked })));
    advisors.show(&view, &hand(&view));
    std::thread::sleep(Duration::from_millis(500));
    // The game has gone on 10 s while it thought: the proposal is stale when it arrives.
    assert!(advisors.take(view.time + 10.0).is_empty(), "a stale proposal is not handed on");
    // The next one, with the game at the time it was made, is handed on.
    view.time = 40.0;
    advisors.show(&view, &hand(&view));
    let p = wait_for("a fresh proposal", || Some(advisors.take(41.0)).filter(|p| !p.is_empty()));
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].made, 40.0);
    assert!(advisors.take(41.0).is_empty(), "a proposal is handed on once");
}

#[test]
fn the_mailbox_keeps_only_the_newest_map() {
    let (_, mut view) = game_with_drones_up();
    let looked = Arc::new(Mutex::new(vec![]));
    let runner =
        Runner::start("slow", Box::new(Slow { think: Duration::from_millis(400), good: 50.0, looked: looked.clone() }));
    runner.show(&view, &hand(&view));
    wait_for("the first look", || (!looked.lock().unwrap().is_empty()).then_some(()));
    for t in 1..=4 {
        view.time = 20.0 + t as f32;
        runner.show(&view, &hand(&view));
    }
    wait_for("the second look", || (looked.lock().unwrap().len() >= 2).then_some(()));
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(*looked.lock().unwrap(), vec![20.0, 24.0], "the maps in between were dropped, not queued");
}

#[test]
fn an_advisor_that_panics_is_dropped_and_the_commander_goes_on() {
    #[derive(Clone, Debug)]
    struct Bad;
    impl Advise for Bad {
        fn advise(&mut self, _: &TeamView, _: &Hand) -> Vec<Proposal> {
            panic!("a bug in an advisor");
        }
        fn clone_box(&self) -> Box<dyn Advise> {
            Box::new(Bad)
        }
    }
    let (_, view) = game_with_drones_up();
    let runner = Runner::start("bad", Box::new(Bad));
    runner.show(&view, &hand(&view));
    wait_for("the runner to give up", || (!runner.alive()).then_some(()));
    runner.show(&view, &hand(&view));
    assert!(runner.take().is_empty());
}

fn script(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("advisors-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_process_advisor_is_one_line_in_and_one_line_out_and_the_rules_come_once() {
    let (g, mut view) = game_with_drones_up();
    // The process is `serve` with the lookahead: a thread plays it through two pipes.
    let (to_r, to_w) = std::io::pipe().unwrap();
    let (from_r, from_w) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        advisors::serve(std::io::BufReader::new(to_r), from_w, |rules, _| {
            Box::new(recreation_towers::Lookahead::new(rules))
        })
    });
    let mut piped =
        ProcessAdvisor::over(Box::new(to_w), Box::new(std::io::BufReader::new(from_r)), "piped", g.rules, Team::Blue)
            .expect("the start is answered");
    view.time = 30.0;
    let proposals = piped.advise(&view, &hand(&view));
    assert!(piped.dropped().is_none(), "{:?}", piped.dropped());
    // Whatever the lookahead proposes, it comes back as proposals made on this map.
    assert!(proposals.iter().all(|p| p.made == 30.0 && p.drone < g.rules.drones));
}

#[test]
fn a_process_advisor_that_answers_gives_its_proposals() {
    let (g, view) = game_with_drones_up();
    let cmd = script(
        "good.sh",
        r#"read start; echo '"ok"'
while read look; do echo '[{"drone":1,"order":{"Take":2},"queue":[],"gain":6.5,"made":20.0,"until":60.0,"why":"from a process"}]'; done"#,
    );
    let mut a = ProcessAdvisor::spawn(&cmd, g.rules, Team::Blue).unwrap();
    let p = a.advise(&view, &hand(&view));
    assert_eq!(p.len(), 1);
    assert_eq!((p[0].drone, p[0].order, p[0].gain, p[0].why.as_str()), (1, Order::Take(2), 6.5, "from a process"));
    assert!(a.dropped().is_none());
}

#[test]
fn a_process_advisor_that_writes_nonsense_is_dropped_with_a_line_and_the_game_goes_on() {
    let (g, view) = game_with_drones_up();
    let cmd = script("nonsense.sh", "read start; echo '\"ok\"'\nwhile read look; do echo 'this is not json'; done");
    let mut a = ProcessAdvisor::spawn(&cmd, g.rules, Team::Blue).unwrap();
    assert!(a.advise(&view, &hand(&view)).is_empty());
    let why = a.dropped().expect("dropped");
    assert!(why.contains("said"), "{why}");
    // Dropped for good: no further question is asked of it.
    let t = Instant::now();
    assert!(a.advise(&view, &hand(&view)).is_empty());
    assert!(t.elapsed() < Duration::from_millis(50));
}

#[test]
fn a_process_advisor_that_proposes_for_a_drone_that_does_not_exist_is_dropped() {
    let (g, view) = game_with_drones_up();
    let cmd = script(
        "ghost.sh",
        r#"read start; echo '"ok"'
while read look; do echo '[{"drone":99,"order":{"Take":2},"queue":[],"gain":6.5,"made":20.0,"until":60.0,"why":"x"}]'; done"#,
    );
    let mut a = ProcessAdvisor::spawn(&cmd, g.rules, Team::Blue).unwrap();
    assert!(a.advise(&view, &hand(&view)).is_empty());
    assert!(a.dropped().is_some());
}

#[test]
fn a_process_advisor_that_dies_is_dropped() {
    let (g, view) = game_with_drones_up();
    let cmd = script("dies.sh", "read start; echo '\"ok\"'\nread look; exit 3");
    let mut a = ProcessAdvisor::spawn(&cmd, g.rules, Team::Blue).unwrap();
    assert!(a.advise(&view, &hand(&view)).is_empty());
    assert!(a.dropped().is_some_and(|w| w.contains("ended")), "{:?}", a.dropped());
}

#[test]
fn a_process_advisor_that_hangs_is_dropped_after_its_time() {
    let (g, view) = game_with_drones_up();
    let cmd = script("hangs.sh", "read start; echo '\"ok\"'\nread look; sleep 30");
    let mut a = ProcessAdvisor::spawn(&cmd, g.rules, Team::Blue).unwrap().with_timeout(Duration::from_millis(300));
    let t = Instant::now();
    assert!(a.advise(&view, &hand(&view)).is_empty());
    assert!(t.elapsed() < Duration::from_secs(3));
    assert!(a.dropped().is_some_and(|w| w.contains("did not answer")), "{:?}", a.dropped());
}

#[test]
fn a_command_that_does_not_start_is_no_advisor() {
    let g = Game::new(Rules::default());
    assert!(ProcessAdvisor::spawn("/no/such/advisor", g.rules, Team::Blue).is_err());
    let cmd = script("mute.sh", "sleep 30");
    assert!(ProcessAdvisor::spawn_within(&cmd, g.rules, Team::Blue, Duration::from_millis(300)).is_err());
}

#[test]
fn the_choices_of_advisor_are_read_from_a_flag() {
    use advisors::AdvisorChoice;
    assert_eq!("lookahead".parse(), Ok(AdvisorChoice::Lookahead));
    assert_eq!("none".parse(), Ok(AdvisorChoice::None));
    assert_eq!("towers_advisor --claude".parse(), Ok(AdvisorChoice::Command("towers_advisor --claude".into())));
    assert!("".parse::<AdvisorChoice>().is_err());
}
