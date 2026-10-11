// Show the vertical range of the vertices each bone moves: bone-ranges model.CRP skeleton.SKL
use std::path::Path;
use rogue_reborn::{crp, skeleton};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let c = crp::read(Path::new(&args[0]))?;
    let s = skeleton::read(Path::new(&args[1]))?;
    let skin = c.skin(&s).expect("skin");
    let (min, _) = c.bounds();
    for (bi, bone) in s.bones.iter().enumerate() {
        let ys: Vec<f32> = skin.bone.iter().enumerate().filter(|&(_, &b)| b == bi).map(|(v, _)| c.vertices[v][1] - min[1]).collect();
        if ys.is_empty() { println!("{:12} (no vertices)", bone.name); continue; }
        let (lo, hi) = ys.iter().fold((f32::MAX, f32::MIN), |(l, h), &y| (l.min(y), h.max(y)));
        println!("{:12} {:3} vertices, height above feet {:6.1} .. {:6.1}", bone.name, ys.len(), lo, hi);
    }
    Ok(())
}
