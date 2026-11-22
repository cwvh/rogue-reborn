//! The pictures and models the game draws with, loaded once at startup.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use rogue_reborn::{map, rsb, crp, actor, weapon, skeleton, casings, impact, blastlight};
use rogue_reborn::body::PlayerMotions;
use rogue_reborn::impact::Surface;
use rogue_reborn::net::proto::Setup;
use rogue_reborn::sim::characters::{Characters, load_motion};
use rogue_reborn::util::find_file;
use crate::args::Args;
use crate::opengl::*;
use crate::render::draw::{CharacterArrays, FlashDraw, FlashPart, WeaponDraw};
use crate::textures::load_character_texture;

/// Soot marks a blast can leave
pub const SCORCH_VARIANTS: usize = 4;
/// Bullet hole pictures per surface with --realistic-decals
pub const HOLE_VARIANTS: usize = 6;

/// A side's uniform: its own mesh, skinned to the players' skeleton
pub struct Uniform {
    pub model: crp::Crp,
    pub arrays: CharacterArrays,
    pub textures: Vec<Option<u32>>,
    pub skin: crp::Skin,
}

pub struct Assets {
    pub player_motions: PlayerMotions,
    pub weapon_def: Option<weapon::Weapon>,
    pub weapon_draw: Option<WeaponDraw>,
    pub grenade_draw: Option<WeaponDraw>,
    pub flash_draw: Option<FlashDraw>,
    pub hole_tex: HashMap<(Surface, usize), u32>,
    pub cloud_tex: Option<u32>,
    pub flash_tex: Option<u32>,
    pub glare_tex: Option<u32>,
    pub smoke_tex: Option<u32>,
    pub ring_tex: Option<u32>,
    pub light_tex: Option<u32>,
    pub weapon_attach: Option<(usize, skeleton::Transform)>,
    pub part_textures: Vec<Option<u32>>,
    pub character_arrays: CharacterArrays,
    pub side_dress: Vec<Option<Uniform>>,
    pub actors: Vec<actor::Actor>,
    pub face_textures: Vec<(Option<u32>, Option<u32>)>,
    // Loaded after the characters are placed, which fills in their bodies and weapons
    pub body_draws: Vec<(CharacterArrays, Vec<Option<u32>>)>,
    pub held_draws: Vec<Option<WeaponDraw>>,
    pub held_ejectors: Vec<Option<casings::Ejector>>,
}

// The player carries the MP5K, so uses the SMG-holding movement set
fn slot_motion(args: &Args, slot: &str, default: &str) -> Option<rogue_reborn::motion::Motion> {
    let name = args.player_anims.iter().rev().find(|(k, _)| k == slot).map_or(default, |(_, m)| m.as_str());
    load_motion(name)
}

fn load_model_draw(file: &str, muzzle: [f32; 3], flash_scale: f32) -> Option<WeaponDraw> {
    let model = map::read_model(&Path::new("data/model").join(file)).ok()?;
    let mut verts = Vec::new();
    let mut uvs = Vec::new();
    for object in &model.geometries.objects {
        for od in &object.object_datas {
            let n = od.faces.face_indices.len().min(od.faces.texture_indices.len());
            for k in 0..n {
                let (i0, i1, i2) = od.faces.face_indices[k];
                let (t0, t1, t2) = od.faces.texture_indices[k];
                for (i, t) in [(i0, t0), (i1, t1), (i2, t2)] {
                    let v = &object.vertices[i as usize];
                    verts.push([v.x, v.y, v.z]);
                    let uv = &od.texture_vertices.uv_coords[t as usize];
                    uvs.push([uv.u, uv.v]);
                }
            }
        }
    }
    let material = model.materials.materials.first()?.filename.clone();
    let stem = Path::new(&material).file_stem()?.to_string_lossy().to_string();
    let texture = find_file(Path::new("data/texture"), &stem, "rsb")
        .and_then(|p| rsb::read(&p).ok())
        .and_then(|t| upload_world_rsb_to_gl(&t));
    println!("loaded {} ({} triangles, texture {})", file, verts.len() / 3, if texture.is_some() { "ok" } else { "missing" });
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &verts {
        for a in 0..3 {
            lo[a] = lo[a].min(v[a]);
            hi[a] = hi[a].max(v[a]);
        }
    }
    let center = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
    let length = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f32::max).max(1.0);
    Some(WeaponDraw { verts, uvs, texture, muzzle, center, length, flash_scale })
}

fn load_flash_draw() -> Option<FlashDraw> {
    let model = map::read_model(Path::new("data/model/muzzle_flash.qob")).ok()?;
    let mut parts = Vec::new();
    for object in &model.geometries.objects {
        for od in &object.object_datas {
            let material = model.materials.materials.get(od.material as usize)?;
            let stem = Path::new(&material.filename).file_stem()?.to_string_lossy().to_string();
            // Some textures are named with a TGA prefix that the material's file name lacks
            let texture = find_file(Path::new("data/texture"), &stem, "rsb")
                .or_else(|| find_file(Path::new("data/texture"), &format!("TGA{stem}"), "rsb"))
                .and_then(|p| rsb::read(&p).ok())
                .and_then(|t| upload_world_rsb_to_gl(&t))?;
            let (mut verts, mut uvs) = (Vec::new(), Vec::new());
            let n = od.faces.face_indices.len().min(od.faces.texture_indices.len());
            for k in 0..n {
                let (i0, i1, i2) = od.faces.face_indices[k];
                let (t0, t1, t2) = od.faces.texture_indices[k];
                for (i, t) in [(i0, t0), (i1, t1), (i2, t2)] {
                    let v = &object.vertices[i as usize];
                    verts.push([v.x, v.y, v.z]);
                    let uv = &od.texture_vertices.uv_coords[t as usize];
                    uvs.push([uv.u, uv.v]);
                }
            }
            parts.push(FlashPart { verts, uvs, texture });
        }
    }
    println!("loaded muzzle_flash.qob ({} triangles)", parts.iter().map(|p| p.verts.len() / 3).sum::<usize>());
    Some(FlashDraw { parts })
}

fn sprite(stem: &str) -> Option<u32> {
    find_file(Path::new("data/texture"), stem, "rsb").and_then(|p| rsb::read(&p).ok()).and_then(|t| upload_world_rsb_to_gl(&t))
}

// One GL texture per part, from the character's own directory
fn textures_of(c: &crp::Crp, dir: &Path) -> Vec<Option<u32>> {
    let mut cache: HashMap<String, Option<u32>> = HashMap::new();
    c.parts.iter().map(|part| {
        *cache.entry(part.material.clone()).or_insert_with(|| {
            let id = load_character_texture(dir, &part.material);
            if id.is_none() {
                println!("no texture for {}", part.material);
            }
            id
        })
    }).collect()
}

impl Assets {
    pub fn load(args: &Args, characters: &Option<Characters>, model_path: &Path, joined: &Option<Setup>) -> Assets {
        let player_motions = PlayerMotions::load(characters.as_ref().and_then(|c| c.rig.as_ref()).map(|r| &r.skeleton), |slot, default| slot_motion(args, slot, default));

        // The MP5K: definition, model and texture
        let weapon_def = weapon::read(Path::new("data/kit/pw_mp5k.wpn")).ok();
        let weapon_draw = weapon_def.as_ref().and_then(|w| load_model_draw(&w.model, w.muzzle, w.flash_scale));
        // The grenade in the hand (the original's grenade.qob is a flat inventory picture)
        let grenade_draw = load_model_draw("rsw_smokegrenade.qob", [0.0; 3], 0.0);
        // The original's flash model has crossed planes along the barrel and a four-point star
        let flash_draw = load_flash_draw();

        // Bullet holes: the game own sprites, or with --realistic-decals pictures drawn for each surface
        let mut hole_tex: HashMap<(Surface, usize), u32> = HashMap::new();
        if args.realistic_decals {
            for surface in [Surface::Concrete, Surface::Wood, Surface::Metal, Surface::Earth, Surface::Soft] {
                for v in 0..HOLE_VARIANTS {
                    let seed = 0x1000 + (v as u64) * 7919 + surface as u64 * 104729;
                    let px = impact::hole_texture(surface, seed);
                    if let Some(id) = upload_world_rgba_to_gl(impact::TEXTURE_SIZE as u32, impact::TEXTURE_SIZE as u32, &px) {
                        hole_tex.insert((surface, v), id);
                    }
                }
            }
        } else {
            for (surface, stem) in [(Surface::Wood, "SP_wood_hit_1"), (Surface::Concrete, "Bullet_hole_simple_1")] {
                let sprite = find_file(Path::new("data/texture"), stem, "rsb").and_then(|p| rsb::read(&p).ok());
                if let Some(t) = sprite {
                    let mut px = t.to_rgba8();
                    impact::key_alpha(&mut px, t.width as usize, t.height as usize);
                    if let Some(id) = upload_world_rgba_to_gl(t.width as u32, t.height as u32, &px) {
                        hole_tex.insert((surface, 0), id);
                    }
                }
            }
        }
        // Soft clouds for dust and a bright sprite for the flash of a spark
        let cloud_tex = sprite("TGAdirt_cloud");
        // The flash sprite sits on a flat color, which would show as a square, so key it out
        let flash_tex = find_file(Path::new("data/texture"), "Sparks1", "rsb").and_then(|p| rsb::read(&p).ok()).and_then(|t| {
            let mut px = t.to_rgba8();
            impact::key_alpha(&mut px, t.width as usize, t.height as usize);
            upload_world_rgba_to_gl(t.width as u32, t.height as u32, &px)
        });
        // A blast: its flash, its smoke, the ring of its shockwave, and the soot it leaves
        let glare_tex = sprite("TGAgrenade_explosion_1");
        let smoke_tex = sprite("TGAgrenade_blast_03");
        let ring_tex = sprite("TGAgrenade_smoke_ring_1");
        for v in 0..SCORCH_VARIANTS {
            let px = impact::hole_texture(Surface::Scorch, 0x5C0 + v as u64 * 3571);
            if let Some(id) = upload_world_rgba_to_gl(impact::TEXTURE_SIZE as u32, impact::TEXTURE_SIZE as u32, &px) {
                hole_tex.insert((Surface::Scorch, v), id);
            }
        }
        // The light of a blast: a round patch of light, laid over each surface it reaches
        let light_tex = upload_rgba_to_gl(blastlight::TEXTURE_SIZE as u32, blastlight::TEXTURE_SIZE as u32, &blastlight::texture());
        if let Some(id) = light_tex {
            unsafe {
                glBindTexture(GL_TEXTURE_2D, id);
                // The patch must not repeat where a surface runs on past it
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as i32);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as i32);
            }
        }

        // Where the weapon sits relative to the right hand. The weapon models are made in the
        // hand bone's own space: the origin is the wrist, forward (-Y) runs along the hand as
        // it hangs in the bind pose, and up (+Z) is the way the body faces. So the gun goes on
        // the hand as it is, and the motions aim it: low and ahead at the ready, level when
        // firing. `--weapon-offset` shifts the gun, in the model's own axes.
        let weapon_attach: Option<(usize, skeleton::Transform)> = (|| {
            let skel = &characters.as_ref()?.rig.as_ref()?.skeleton;
            let hand = skel.bones.iter().position(|b| b.name == "RightHand")?;
            Some((hand, skeleton::Transform {
                rot: [0.0, 0.0, 0.0, 1.0],
                pos: args.weapon_offset.unwrap_or([0.0, 0.0, 0.0]),
            }))
        })();

        let part_textures: Vec<Option<u32>> = characters.as_ref().map_or_else(Vec::new, |c| textures_of(&c.model, model_path.parent().unwrap_or(Path::new("."))));
        // The model's triangle corners, laid out for glDrawArrays
        let character_arrays = characters.as_ref().map_or_else(CharacterArrays::default, |c| CharacterArrays::new(&c.model));
        // What each side wears in a team game, as the server says. Each uniform is its own mesh on
        // the players' skeleton, so one posing dresses them all.
        let side_dress: Vec<Option<Uniform>> = joined.iter()
            .filter(|s| s.bases.iter().all(|b| !b.is_empty()))
            .flat_map(|s| s.uniforms.clone())
            .map(|path| {
                let path = PathBuf::from(path);
                let model = crp::read(&path).map_err(|e| eprintln!("could not load the uniform {}: {e:#}", path.display())).ok()?;
                let skin = characters.as_ref().and_then(|c| c.rig.as_ref()).and_then(|r| model.skin(&r.skeleton));
                let Some(skin) = skin else {
                    eprintln!("the uniform {} does not fit the players' skeleton", path.display());
                    return None;
                };
                let textures = textures_of(&model, path.parent().unwrap_or(Path::new(".")));
                Some(Uniform { arrays: CharacterArrays::new(&model), textures, skin, model })
            })
            .collect();
        // Team members whose faces the models wear: --actor picks one, otherwise
        // they are dealt out in turn
        let mut actors = actor::read_all(Path::new("data/actor"));
        if let Some(name) = &args.actor {
            actors.retain(|a| a.id.eq_ignore_ascii_case(name));
            if actors.is_empty() {
                eprintln!("no actor named {name}");
            }
        }
        // GL textures for each actor's HRT face: (main, blink)
        let faces_dir = Path::new("data/texture/faces");
        let face_textures: Vec<(Option<u32>, Option<u32>)> = actors.iter().map(|a| {
            let load = |p: &Path| rsb::read(p).ok().and_then(|t| upload_world_rsb_to_gl(&t));
            let found = a.hrt_face.as_deref().and_then(|f| actor::find_face_textures(faces_dir, f));
            match found {
                Some((main, blink)) => (load(&main), blink.and_then(|b| load(&b))),
                None => (None, None),
            }
        }).collect();

        Assets {
            player_motions, weapon_def, weapon_draw, grenade_draw, flash_draw, hole_tex, cloud_tex, flash_tex,
            glare_tex, smoke_tex, ring_tex, light_tex, weapon_attach, part_textures, character_arrays,
            side_dress, actors, face_textures,
            body_draws: Vec::new(), held_draws: Vec::new(), held_ejectors: Vec::new(),
        }
    }

    // Each body a kit dresses its character in, as drawn: its corners laid out, and the texture
    // each part wears. Then the weapons in the characters' hands.
    pub fn load_held(&mut self, characters: &Option<Characters>) {
        self.body_draws = characters.as_ref().map_or_else(Vec::new, |c| {
            c.bodies.iter().map(|b| {
                let mut cache: HashMap<String, Option<u32>> = HashMap::new();
                let textures = b.textures.iter().map(|(dir, t)| {
                    *cache.entry(t.clone()).or_insert_with(|| {
                        let id = load_character_texture(dir, t);
                        if id.is_none() {
                            println!("no texture for {t}");
                        }
                        id
                    })
                }).collect();
                (CharacterArrays::new(&b.model), textures)
            }).collect()
        });
        self.held_draws = characters.as_ref().map_or_else(Vec::new, |c| {
            c.weapons.iter().map(|(_, w)| load_model_draw(&w.model, w.muzzle, w.flash_scale)).collect()
        });
        self.held_ejectors = characters.as_ref().map_or_else(Vec::new, |c| c.weapons.iter().map(|(_, w)| casings::Ejector::of(w)).collect());
    }
}
