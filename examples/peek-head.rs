// Where the head goes through the peek motions: peek-head [--keys] [motion...]
//
// The original's eye follows the head sideways while peeking (RogueSpear.exe 0x5d67c0), so
// this poses the soldier skeleton through each motion and prints, every frame, the head's
// offset from the body (x sideways, y up, z forward) and from where it starts. With --keys,
// only how far it has moved right and up at each stored key, as sim::peek keeps them.

use std::path::Path;

use rogue_reborn::{motion, skeleton};

fn main() -> anyhow::Result<()> {
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    let keys = names.first().is_some_and(|n| n == "--keys");
    if keys {
        names.remove(0);
    }
    if names.is_empty() {
        names = ["tau_plf", "tau_prt", "tac_plf", "tac_prt", "tmac_plf", "tmac_prt"].map(String::from).to_vec();
    }
    let skel = skeleton::read(Path::new("data/character/team_hrt/Arctic1_H_a.SKL"))?;
    let head = skel.bones.iter().position(|b| b.name == "Head").expect("a head");
    for name in names {
        let m = motion::read(&Path::new("data/motion").join(format!("{name}.DAM")))?;
        println!("{name}: {} frames, {} keys, one every {}", m.frames, m.keys.len(), m.key_interval);
        let at = |f: f32| {
            let (root, rots) = m.sample(f);
            skel.pose(root, &rots)[head].pos
        };
        let start = at(0.0);
        if keys {
            let moved: Vec<String> = (0..m.keys.len()).map(|k| {
                let p = at((k as u32 * m.key_interval) as f32);
                format!("[{:.1}, {:.1}]", p[0] - start[0], p[1] - start[1])
            }).collect();
            println!("    {}", moved.join(", "));
            continue;
        }
        for f in 0..m.frames {
            let p = at(f as f32);
            println!("  {f:3}  head {:7.1} {:7.1} {:7.1}  moved {:7.1} {:7.1} {:7.1}", p[0], p[1], p[2], p[0] - start[0], p[1] - start[1], p[2] - start[2]);
        }
    }
    Ok(())
}
