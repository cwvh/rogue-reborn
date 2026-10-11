// Summarize character motions: motion-info [prefix]
//
// For each motion in data/motion, poses the soldier skeleton through every
// stored key and reports how far the feet travel relative to the hips on the
// model's X (sideways) and Z (forward/back) axes, and the hips height. A walk
// swings the feet along Z; a strafe swings them along X.

use std::path::Path;

use rogue_reborn::{motion, skeleton};

fn main() -> anyhow::Result<()> {
    let prefix = std::env::args().nth(1).unwrap_or_default().to_lowercase();
    let skel = skeleton::read(Path::new("data/character/team_hrt/Arctic1_H_a.SKL"))?;
    let feet: Vec<usize> = ["LeftFoot", "RightFoot"].iter()
        .filter_map(|n| skel.bones.iter().position(|b| &b.name == n)).collect();
    let hips = skel.bones.iter().position(|b| b.name == "Hips").unwrap();

    let mut files: Vec<_> = std::fs::read_dir("data/motion")?.flatten().map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dam"))).collect();
    files.sort();

    println!("{:22} {:>6} {:>5} {:>8} {:>8} {:>8} {:>9} {:>9} {:>9} {:>9}", "motion", "frames", "keys", "swing_x", "swing_z", "hips_y", "travel_x", "travel_z", "planted_z", "planted_x");
    for f in files {
        let stem = f.file_stem().unwrap().to_string_lossy().to_lowercase();
        if !stem.starts_with(&prefix) { continue; }
        let m = motion::read(&f)?;
        let (mut min_x, mut max_x, mut min_z, mut max_z) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        let mut hy = 0.0;
        for k in &m.keys {
            let pose = skel.pose([0.0, k.root[1], 0.0], &k.rotations);
            hy += k.root[1] / m.keys.len() as f32;
            for &foot in &feet {
                // Feet relative to the hips on the ground plane
                let p = pose[foot].pos;
                let h = pose[hips].pos;
                min_x = min_x.min(p[0] - h[0]);
                max_x = max_x.max(p[0] - h[0]);
                min_z = min_z.min(p[2] - h[2]);
                max_z = max_z.max(p[2] - h[2]);
            }
        }
        // Travel speed (units per second at 30 fps): the planted foot slides
        // backwards relative to the hips, so the body moves the other way
        let (mut tx, mut tz, mut n) = (0.0f32, 0.0f32, 0);
        let sample = |f: f32| {
            let (root, rots) = m.sample(f);
            skel.pose([0.0, root[1], 0.0], &rots)
        };
        let mut prev = sample(0.0);
        for fr in 1..m.frames {
            let cur = sample(fr as f32);
            // The planted foot is the lower one in both frames
            let low = |p: &Vec<skeleton::Transform>| if p[feet[0]].pos[1] < p[feet[1]].pos[1] { 0 } else { 1 };
            let (a, b) = (low(&prev), low(&cur));
            if a == b {
                let rel = |p: &Vec<skeleton::Transform>| {
                    let (f, h) = (p[feet[a]].pos, p[hips].pos);
                    (f[0] - h[0], f[2] - h[2])
                };
                let (p0, p1) = (rel(&prev), rel(&cur));
                tx -= (p1.0 - p0.0) * 30.0;
                tz -= (p1.1 - p0.1) * 30.0;
                n += 1;
            }
            prev = cur;
        }
        let n = n.max(1) as f32;

        // Planted-foot estimate: sample finely, find each foot's lowest height
        // over the cycle, and treat it as planted while within a few units of
        // that. The body travels at the speed the planted foot slides back.
        let steps = (m.frames * 8) as usize;
        let samples: Vec<Vec<skeleton::Transform>> = (0..=steps).map(|i| sample(i as f32 / 8.0)).collect();
        let floor_y = samples.iter().flat_map(|p| feet.iter().map(move |&f| p[f].pos[1])).fold(f32::MAX, f32::min);
        let (mut px, mut pz, mut pn) = (Vec::new(), Vec::new(), 0);
        for w in samples.windows(2) {
            for &f in &feet {
                let planted = |p: &Vec<skeleton::Transform>| p[f].pos[1] < floor_y + 4.0;
                if planted(&w[0]) && planted(&w[1]) {
                    let rel = |p: &Vec<skeleton::Transform>| (p[f].pos[0] - p[hips].pos[0], p[f].pos[2] - p[hips].pos[2]);
                    let (a, b) = (rel(&w[0]), rel(&w[1]));
                    px.push(-(b.0 - a.0) * 30.0 * 8.0);
                    pz.push(-(b.1 - a.1) * 30.0 * 8.0);
                    pn += 1;
                }
            }
        }
        let median = |v: &mut Vec<f32>| { v.sort_by(|a, b| a.total_cmp(b)); if v.is_empty() { 0.0 } else { v[v.len() / 2] } };
        // SPEED=v reports how far a planted foot slips along the ground, in
        // units per second, if the body moves along Z at v
        if let Some(v) = std::env::var("SPEED").ok().and_then(|v| v.parse::<f32>().ok()) {
            let mut slip: Vec<f32> = pz.iter().map(|z| (z - v).abs()).collect();
            let med = median(&mut slip);
            let worst = slip.last().copied().unwrap_or(0.0);
            println!("  {stem}: planted-foot slip at {v:.0}/s: median {med:.1}, worst {worst:.1} units/s");
            // The speed with the smallest typical slip
            let best = (-500..=500).map(|c| c as f32).map(|c| {
                let mut r: Vec<f32> = pz.iter().map(|z| (z - c).abs()).collect();
                (c, median(&mut r))
            }).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            println!("  {stem}: least-slip speed {:.0}/s (median slip {:.1})", best.0, best.1);
            let best_x = (-500..=500).map(|c| c as f32).map(|c| {
                let mut r: Vec<f32> = px.iter().map(|x| (x - c).abs()).collect();
                (c, median(&mut r))
            }).min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            println!("  {stem}: least-slip sideways speed {:.0}/s (median slip {:.1})", best_x.0, best_x.1);
        }
        let (mx, mz) = (median(&mut px), median(&mut pz));
        let _ = pn;
        println!("{:22} {:6} {:5} {:8.1} {:8.1} {:8.1} {:9.0} {:9.0} {:9.0} {:9.0}", stem, m.frames, m.keys.len(), max_x - min_x, max_z - min_z, hy, tx / n, tz / n, mz, mx);
    }
    Ok(())
}
