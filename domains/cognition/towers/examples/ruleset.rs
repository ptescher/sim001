//! A round robin of fixed standing-order policies under a ruleset, on paired seeds, both sides:
//! `cargo run --release -p cognition_towers --example ruleset [v1|v2] [SEEDS]`. The numbers in
//! `plans/towers-rules-v2.md` section 7 are read off its output.

use cognition_towers::{After, Autonomy, Failsafe, Persist, Prepared, Surface, compare_with, play_with};
use recreation_towers::{Commander, Order, Outcome, Rules, Team};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut rules = match args.next().as_deref() {
        Some("v1") => Rules::v1(),
        _ => Rules::default(),
    };
    let n: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);
    // `field=value` after the number of seeds changes a rule, to tune.
    for kv in args {
        let (k, v) = kv.split_once('=').ok_or("a rule is field=value")?;
        let v: f32 = v.parse().map_err(|_| format!("{kv}: not a number"))?;
        match k {
            "contest_drain" => rules.contest_drain = v,
            "link_timeout" => rules.link_timeout = v,
            "mesh_range" => rules.mesh_range = v,
            "mesh_hops" => rules.mesh_hops = v as u32,
            "decay_time" => rules.decay_time = v,
            "capture_time" => rules.capture_time = v,
            "win_score" => rules.win_score = v,
            "spire_score" => rules.spire_score = v,
            "standard_battery" => rules.standard_battery = v,
            "bingo" => rules.bingo = v,
            _ => return Err(format!("{k}: not a rule this example can change")),
        }
    }
    let seeds: Vec<u64> = (1..=n).collect();
    let policies: Vec<Box<dyn Autonomy>> = vec![
        Box::new(Failsafe),
        Box::new(Persist),
        Box::new(After { after: 5.0, then: Order::Hold }),
        Box::new(After { after: 10.0, then: Order::Home }),
        Box::new(After { after: 30.0, then: Order::Home }),
        Box::new(Surface { after: 5.0, rules }),
    ];
    let k = policies.len();
    let mut margin = vec![vec![0.0f32; k]; k];
    let mut error = vec![vec![0.0f32; k]; k];
    let (mut downs, mut dark, mut games) = (0.0f32, 0.0f32, 0.0f32);
    for a in 0..k {
        for b in (a + 1)..k {
            let c = compare_with(rules, policies[a].as_ref(), policies[b].as_ref(), &seeds)?;
            margin[a][b] = c.margin;
            margin[b][a] = -c.margin;
            error[a][b] = c.error;
            error[b][a] = c.error;
            downs += c.downs[0] + c.downs[1];
            dark += c.dark[0] + c.dark[1];
            games += 2.0;
        }
    }
    println!(
        "ruleset {:?}, {} seeds (each pair plays {} games)\n",
        std::env::args().nth(1).unwrap_or("v2".into()),
        n,
        2 * n
    );
    print!("| row beats column |");
    for p in &policies {
        print!(" {} |", p.name());
    }
    println!(" mean |\n|---|{}---|", "---|".repeat(k));
    let mut means = vec![];
    for a in 0..k {
        print!("| {} |", policies[a].name());
        for b in 0..k {
            if a == b {
                print!(" . |");
            } else {
                print!(" {:+.2} ± {:.2} |", margin[a][b], error[a][b]);
            }
        }
        let mean = margin[a].iter().sum::<f32>() / (k - 1) as f32;
        means.push(mean);
        println!(" {mean:+.2} |");
    }
    let mut ranked = means.clone();
    ranked.sort_by(|x, y| y.total_cmp(x));
    println!("\nbest mean {:+.2}, second {:+.2}: lead {:.2}", ranked[0], ranked[1], ranked[0] - ranked[1]);
    println!("persist against failsafe: {:+.2} ± {:.2}", margin[1][0], error[1][0]);
    let dominant = (0..k).filter(|&a| (0..k).filter(|&b| b != a).all(|b| margin[a][b] > 3.0 * error[a][b])).count();
    println!("policies that beat every other at 3 standard errors: {dominant}");
    println!("per game and team: {:.2} drones down, {:.0} s dark", downs / games, dark / games);

    // Health: the same policy on both sides.
    let (mut blue, mut length, mut drawn, mut played) = (0, 0.0, 0, 0);
    for &seed in &seeds {
        let seat = || Prepared { captain: Commander::new(rules), autonomy: Failsafe };
        let r = play_with(rules, &mut seat(), &mut seat(), seed)?;
        blue += (r.outcome == Outcome::Won(Team::Blue)) as u32;
        drawn += (r.outcome == Outcome::Draw) as u32;
        length += r.length;
        played += 1;
    }
    println!(
        "default against default: Blue wins {:.0} %, draws {:.0} %, a game lasts {:.0} s of {:.0}",
        100.0 * blue as f32 / played as f32,
        100.0 * drawn as f32 / played as f32,
        length / played as f32,
        rules.duration
    );
    Ok(())
}
