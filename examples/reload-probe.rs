// Trace the hands through motions: [STEP=n] reload-probe motion...
//
// Poses the soldier skeleton through each motion and prints, every few frames (STEP), where
// the left palm is in the right hand's space, which is the held weapon's (x right, -y forward
// along the gun, z up toward its top), how far the right hand has turned since the first
// frame, and where the right hand is from the chest. A reload shows as the left hand leaving
// the gun and coming back. On the MP5K the magazine well is near y -25, z 5, and the top of
// the gun (where the cocking handle is) above z 18.

use std::path::Path;

use rogue_reborn::{motion, skeleton};

fn main() -> anyhow::Result<()> {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let skel = skeleton::read(Path::new("data/character/team_hrt/Arctic1_H_a.SKL"))?;
    let bone = |n: &str| skel.bones.iter().position(|b| b.name == n).unwrap();
    let (left, right, chest) = (bone("LeftHand"), bone("RightHand"), bone("Chest"));
    let inverse = |q: [f32; 4]| [-q[0], -q[1], -q[2], q[3]];
    for name in names {
        let m = motion::read(&Path::new("data/motion").join(format!("{name}.DAM")))?;
        println!("{name}: {} frames, {} keys every {}", m.frames, m.keys.len(), m.key_interval);
        let first = m.sample(0.0);
        let r0 = skel.pose([0.0, first.0[1], 0.0], &first.1)[right].rot;
        let step = std::env::var("STEP").ok().and_then(|s| s.parse().ok()).unwrap_or((m.frames / 16).max(1));
        for f in (0..m.frames).step_by(step as usize) {
            let (root, rots) = m.sample(f as f32);
            let pose = skel.pose([0.0, root[1], 0.0], &rots);
            let (l, r, c) = (pose[left], pose[right], pose[chest]);
            // The palm, a hand's length past the wrist
            let palm = motion::quat_rotate(l.rot, [0.0, -8.0, 0.0]);
            let d = [l.pos[0] + palm[0] - r.pos[0], l.pos[1] + palm[1] - r.pos[1], l.pos[2] + palm[2] - r.pos[2]];
            let local = motion::quat_rotate(inverse(r.rot), d);
            let turn = motion::quat_mul(inverse(r0), r.rot);
            let angle = 2.0 * turn[3].abs().clamp(0.0, 1.0).acos().to_degrees();
            // Right hand in front of the chest: height above it and distance ahead (+z)
            let rc = [r.pos[0] - c.pos[0], r.pos[1] - c.pos[1], r.pos[2] - c.pos[2]];
            println!("  {f:4}  left in gun space {:7.1} {:7.1} {:7.1}  right turned {angle:5.1}  right from chest {:6.1} {:6.1} {:6.1}",
                local[0], local[1], local[2], rc[0], rc[1], rc[2]);
        }
    }
    Ok(())
}
