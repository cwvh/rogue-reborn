//! The network lab (see `net::lab`): bots fight over pretend connections, in a moment, and the
//! tables say how fair and smooth it was.
//!
//!   rrlab                  every table, 30 seconds of play for each row
//!   rrlab --seconds 120    longer games, steadier figures

use rogue_reborn::net::lab::{Tally, fight, internet, peek_judged};
use rogue_reborn::net::link::Conditions;

fn usage(problem: &str) -> ! {
    if !problem.is_empty() {
        eprintln!("rrlab: {problem}");
    }
    eprintln!("usage: rrlab [--seconds N]");
    std::process::exit(if problem.is_empty() { 0 } else { 2 });
}

fn mean(xs: &[f32]) -> f32 {
    if xs.is_empty() { f32::NAN } else { xs.iter().sum::<f32>() / xs.len() as f32 }
}

/// The value `p` (0 to 1) of the way up the sorted values
fn percentile(xs: &[f32], p: f32) -> f32 {
    if xs.is_empty() {
        return f32::NAN;
    }
    let mut v = xs.to_vec();
    v.sort_by(f32::total_cmp);
    v[((v.len() - 1) as f32 * p).round() as usize]
}

fn percent(n: usize, of: usize) -> String {
    if of == 0 { "-".to_string() } else { format!("{:.0}%", 100.0 * n as f32 / of as f32) }
}

fn ms(s: f32) -> String {
    if s.is_nan() { "-".to_string() } else { format!("{:.0}", s * 1000.0) }
}

fn units(u: f32) -> String {
    if u.is_nan() { "-".to_string() } else { format!("{u:.0}") }
}

/// A row of hit figures: shots, agreement, hits honored and unseen, and the gap
fn hits(t: &Tally) -> String {
    format!("{:>6} {:>6} {:>8} {:>7} {:>5} {:>5}",
        t.shots, percent(t.agree, t.shots), percent(t.honored, t.seen_hits), percent(t.unseen, t.server_hits),
        units(mean(&t.gaps)), units(percentile(&t.gaps, 0.95)))
}

fn main() {
    let mut seconds = 30.0f32;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--seconds" => seconds = it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage("--seconds expects a number")),
            "--help" | "-h" => usage(""),
            other => usage(&format!("unknown option {other}")),
        }
    }

    println!("Fights: bots in the box room, everyone for themselves, {seconds:.0} s each");
    println!();
    println!("A shot agrees when the server and the shooter's screen hit the same player, or both miss.");
    println!("Honored: hits on the shooter's screen the server gave. Unseen: hits the server gave that");
    println!("the shooter's screen showed as misses. Gap: how far the victim, on their own screen, was");
    println!("from where the shot found them (units, about a centimeter; mean and 95th percentile).");
    println!();
    println!("{:<22} {:^39} {:^39}", "", "shooters on the LAN", "shooters over the internet");
    println!("{:<22} {:>6} {:>6} {:>8} {:>7} {:>5} {:>5} {:>6} {:>6} {:>8} {:>7} {:>5} {:>5}",
        "players", "shots", "agree", "honored", "unseen", "gap", "p95", "shots", "agree", "honored", "unseen", "gap", "p95");
    let rows: Vec<(usize, usize, f64)> = vec![(8, 0, 0.0), (8, 2, 0.05), (8, 2, 0.10), (8, 2, 0.15), (14, 2, 0.10)];
    let mut fights = Vec::new();
    for &(lan, remote, ping) in &rows {
        let f = fight(lan, remote, ping, seconds);
        let label = if remote == 0 { format!("{lan} LAN") } else { format!("{lan} LAN, {remote} at {:.0} ms", ping * 1000.0) };
        println!("{label:<22} {} {}", hits(&f.lan), if remote == 0 { String::new() } else { hits(&f.remote) });
        fights.push((label, f));
    }

    println!();
    println!("Delays (ms, mean): from sending the input that fired to being told of the shot, and from");
    println!("dying on the server to being told. Fixes: the server moving a player from where they");
    println!("predicted, a minute, and the worst (units). Bytes: each client's download, and the");
    println!("server's upload to all of them (KB a second).");
    println!();
    println!("{:<22} {:>9} {:>9} {:>9} {:>9} {:>8} {:>8} {:>8} {:>8}",
        "players", "shot LAN", "shot net", "died LAN", "died net", "fixes", "worst", "down KB", "up KB");
    for (label, f) in &fights {
        let fixes: u32 = f.meters.iter().map(|(_, m)| m.fixes).sum();
        let worst = f.meters.iter().map(|(_, m)| m.worst_fix).fold(0.0, f32::max);
        let down = mean(&f.meters.iter().map(|(_, m)| m.bytes_in).collect::<Vec<_>>());
        println!("{label:<22} {:>9} {:>9} {:>9} {:>9} {:>8.1} {:>8.0} {:>8.1} {:>8.1}",
            ms(mean(&f.lan.shot_delays)), ms(mean(&f.remote.shot_delays)), ms(mean(&f.lan.death_delays)), ms(mean(&f.remote.death_delays)),
            fixes as f32 * 60.0 / seconds / f.meters.len() as f32, worst, down / 1000.0, f.upload / 1000.0);
    }

    println!();
    println!("Peeking: one player steps out past a corner toward another holding still, again and again.");
    println!("Advantage: how much sooner the one moving sees the other than they are seen (ms). The");
    println!("holder fires whenever their screen shows the peeker. Behind: hits on the peeker while,");
    println!("on their own screen, they were back behind the corner. The last row judges shots where");
    println!("the others are now, not where the shooter saw them, to show what looking back does.");
    println!();
    println!("{:<26} {:>6} {:>10} {:>6} {:>6} {:>8} {:>7} {:>5} {:>5}",
        "holder / peeker", "peeks", "advantage", "shots", "agree", "honored", "behind", "gap", "p95");
    let lan = Conditions::lan();
    let rows = [
        ("LAN / LAN", lan, lan, true),
        ("LAN / 50 ms", lan, internet(0.05), true),
        ("LAN / 100 ms", lan, internet(0.10), true),
        ("LAN / 150 ms", lan, internet(0.15), true),
        ("100 ms / LAN", internet(0.10), lan, true),
        ("100 ms / 100 ms", internet(0.10), internet(0.10), true),
        ("100 ms / LAN, judged now", internet(0.10), lan, false),
    ];
    for (label, holder, peeker, looking_back) in rows {
        let p = peek_judged(holder, peeker, seconds, looking_back);
        let h = &p.holder;
        println!("{label:<26} {:>6} {:>10} {:>6} {:>6} {:>8} {:>7} {:>5} {:>5}",
            p.advantage.len(), ms(mean(&p.advantage)), h.shots, percent(h.agree, h.shots), percent(h.honored, h.seen_hits),
            percent(p.peeker.behind, p.peeker.gaps.len()), units(mean(&p.peeker.gaps)), units(percentile(&p.peeker.gaps, 0.95)));
    }
}
