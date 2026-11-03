//! The network lab (see `net::lab`): bots fight over pretend connections, in a moment, and the
//! tables say how fair and smooth it was.
//!
//!   rrlab                  every table, 30 seconds of play for each row
//!   rrlab --seconds 120    longer games, steadier figures
//!   rrlab --tune           try settings against one another (see `Tuning`)

use rogue_reborn::net::lab::{Tally, Tuning, fight, fight_with, internet, peek_with};
use rogue_reborn::net::link::Conditions;

fn usage(problem: &str) -> ! {
    if !problem.is_empty() {
        eprintln!("rrlab: {problem}");
    }
    eprintln!("usage: rrlab [--seconds N] [--tune]");
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

/// Each setting tried, against the costs it trades: how soon others are seen, and how often
/// they stand still for want of a snapshot
fn tune(seconds: f32) {
    let (lan, far, wifi) = (Conditions::lan(), internet(0.10), Conditions::wifi());
    println!("Tuning: the snapshot rate, and how far behind the newest snapshot others are shown (in");
    println!("snapshot intervals, and in ms). Advantage: the peeker's, both on the LAN and the peeker at");
    println!("100 ms. Still: frames in which the holder's view of the peeker stood still for want of a");
    println!("snapshot, the holder on the LAN, at 100 ms and on Wi-Fi. Agree: the holder on Wi-Fi.");
    println!("Up: the host's upload for 16 players (KB a second). {seconds:.0} s a peek.");
    println!();
    println!("{:>5} {:>6} {:>6} {:>9} {:>9} {:>7} {:>7} {:>7} {:>6} {:>6}", "rate", "delay", "ms", "adv LAN", "adv 100", "still", "100", "Wi-Fi", "agree", "up KB");
    for rate in [30.0f32, 60.0] {
        for delay in [1.0, 1.25, 1.5, 2.0, 3.0] {
            let t = Tuning { view_delay: delay, snapshot_rate: rate, ..Tuning::default() };
            let a = peek_with(t, lan, lan, seconds);
            let b = peek_with(t, lan, far, seconds);
            let c = peek_with(t, far, lan, seconds);
            let d = peek_with(t, wifi, lan, seconds);
            let up = fight_with(t, 14, 2, far, seconds.min(20.0)).upload;
            println!("{rate:>5.0} {delay:>6.2} {:>6.0} {:>9} {:>9} {:>7} {:>7} {:>7} {:>6} {:>6.0}",
                delay * 1000.0 / rate as f64, ms(mean(&a.advantage)), ms(mean(&b.advantage)),
                percent(a.holder.starved as usize, a.holder.frames as usize), percent(c.holder.starved as usize, c.holder.frames as usize),
                percent(d.holder.starved as usize, d.holder.frames as usize), percent(d.holder.agree, d.holder.shots), up / 1000.0);
        }
    }

    println!();
    println!("How far back the server looks at most. A shooter on Wi-Fi or at 300 ms against a peeker on");
    println!("the LAN: the shooter's agreement, and the peeker's hits behind cover and gap (mean, p95).");
    println!("Fights: 8 on the LAN and 2 on Wi-Fi, the Wi-Fi players' agreement and the LAN players' gap.");
    println!();
    println!("{:>7} {:>11} {:>7} {:>5} {:>5} {:>11} {:>7} {:>5} {:>5} {:>11} {:>5} {:>5}",
        "back ms", "Wi-Fi agree", "behind", "gap", "p95", "300 agree", "behind", "gap", "p95", "fight agree", "gap", "p95");
    for back in [0.25f32, 0.5, 0.75, 1.0] {
        let t = Tuning { max_rewind: back, ..Tuning::default() };
        let w = peek_with(t, wifi, lan, seconds);
        let x = peek_with(t, internet(0.30), lan, seconds);
        let f = fight_with(t, 8, 2, wifi, seconds);
        println!("{:>7.0} {:>11} {:>7} {:>5} {:>5} {:>11} {:>7} {:>5} {:>5} {:>11} {:>5} {:>5}", back * 1000.0,
            percent(w.holder.agree, w.holder.shots), percent(w.peeker.behind, w.peeker.gaps.len()), units(mean(&w.peeker.gaps)), units(percentile(&w.peeker.gaps, 0.95)),
            percent(x.holder.agree, x.holder.shots), percent(x.peeker.behind, x.peeker.gaps.len()), units(mean(&x.peeker.gaps)), units(percentile(&x.peeker.gaps, 0.95)),
            percent(f.remote.agree, f.remote.shots), units(mean(&f.lan.gaps)), units(percentile(&f.lan.gaps, 0.95)));
    }
}

fn main() {
    let mut seconds = 30.0f32;
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--tune" => {
                tune(seconds);
                return;
            }
            "--seconds" => seconds = it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| usage("--seconds expects a number")),
            "--help" | "-h" => usage(""),
            other => usage(&format!("unknown option {other}")),
        }
    }

    println!("Fights: bots in the box room, everyone for themselves, {seconds:.0} s each. A ping is a");
    println!("round trip. Wi-Fi is 60 ms with everything lost for a fifth of a second every five");
    println!("seconds or so, and held up for about a third of a second every fifteen.");
    println!();
    println!("A shot agrees when the server and the shooter's screen hit the same player, or both miss.");
    println!("Honored: hits on the shooter's screen the server gave. Unseen: hits the server gave that");
    println!("the shooter's screen showed as misses. Gap: how far the victim, on their own screen, was");
    println!("from where the shot found them (units, about a centimeter; mean and 95th percentile).");
    println!();
    println!("{:<22} {:^39} {:^39}", "", "shooters on the LAN", "shooters over the internet");
    println!("{:<22} {:>6} {:>6} {:>8} {:>7} {:>5} {:>5} {:>6} {:>6} {:>8} {:>7} {:>5} {:>5}",
        "players", "shots", "agree", "honored", "unseen", "gap", "p95", "shots", "agree", "honored", "unseen", "gap", "p95");
    let rows = [
        ("8 LAN", 8, 0, Conditions::lan()),
        ("8 LAN, 2 at 50 ms", 8, 2, internet(0.05)),
        ("8 LAN, 2 at 100 ms", 8, 2, internet(0.10)),
        ("8 LAN, 2 at 150 ms", 8, 2, internet(0.15)),
        ("8 LAN, 2 on Wi-Fi", 8, 2, Conditions::wifi()),
        ("14 LAN, 2 at 100 ms", 14, 2, internet(0.10)),
    ];
    let mut fights = Vec::new();
    for (label, lan, remote, far) in rows {
        let f = fight(lan, remote, far, seconds);
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
        let fixes = f.lan.fixes + f.remote.fixes;
        let worst = f.lan.worst_fix.max(f.remote.worst_fix);
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
        ("LAN / Wi-Fi", lan, Conditions::wifi(), true),
        ("Wi-Fi / LAN", Conditions::wifi(), lan, true),
        ("100 ms / LAN, judged now", internet(0.10), lan, false),
    ];
    for (label, holder, peeker, looking_back) in rows {
        let p = peek_with(Tuning { lag_compensation: looking_back, ..Tuning::default() }, holder, peeker, seconds);
        let h = &p.holder;
        println!("{label:<26} {:>6} {:>10} {:>6} {:>6} {:>8} {:>7} {:>5} {:>5}",
            p.advantage.len(), ms(mean(&p.advantage)), h.shots, percent(h.agree, h.shots), percent(h.honored, h.seen_hits),
            percent(p.peeker.behind, p.peeker.gaps.len()), units(mean(&p.peeker.gaps)), units(percentile(&p.peeker.gaps, 0.95)));
    }
}
