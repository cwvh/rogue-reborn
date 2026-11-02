//! The characters standing in the level: where they are, how they are posed for hit
//! detection, and the models they wear: the team members' one, and the kits' own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::actor::{self, Actor};
use crate::collision::CollisionWorld;
use crate::combat::{self, Health, NpcState, Placement};
use crate::crp::{self, Crp, Skin};
use crate::mission::Stance;
use crate::motion::{self, Motion};
use crate::skeleton::{self, Skeleton};
use crate::util::find_file;

type V3 = [f32; 3];

/// How a character walks somewhere: creeping, walking or running
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Gait {
    Creep,
    #[default]
    Walk,
    Run,
}

/// A character in the level
#[derive(Clone)]
pub struct Npc {
    pub health: Health,
    /// The model's origin in map space
    pub pos: V3,
    /// Where its feet are
    pub floor: f32,
    /// Degrees about the vertical axis
    pub yaw: f32,
    pub scale: V3,
    /// Staggers each character's idle animation and blinking, so they do not all move as one
    pub stagger: f32,
    /// Which of `Characters::bodies` it wears, or the team members' model
    pub body: Option<usize>,
    /// How it waits, as a mission's characters do, or else idling as `Characters::idle`
    pub stance: Option<Stance>,
    /// Which of `Characters::weapons` is in its hands
    pub weapon: Option<usize>,
    /// Aiming at a player, which it stands (or kneels) to do, and when it last fired
    pub aiming: bool,
    pub last_shot: f32,
    /// When it began to aim, raising its weapon (see `terrorist::raise`)
    pub aim_since: f32,
    /// Walking somewhere, and how
    pub walking: bool,
    pub gait: Gait,
    /// What it knows and holds, as a terrorist: only the game's own copy keeps it
    pub mind: Option<super::terrorist::Mind>,
    /// Its name in the mission, which plans name it by
    pub name: String,
    /// How far through a reload it is, 0 to 1
    pub reload: Option<f32>,
}

impl Default for Npc {
    fn default() -> Npc {
        Npc {
            health: Health::new(), pos: [0.0; 3], floor: 0.0, yaw: 0.0, scale: [1.0; 3], stagger: 0.0,
            body: None, stance: None, weapon: None, aiming: false, last_shot: f32::NEG_INFINITY, aim_since: 0.0, walking: false, gait: Gait::Walk, mind: None,
            name: String::new(), reload: None,
        }
    }
}

/// A skeleton and the skin that follows it
pub struct Rig {
    pub skeleton: Skeleton,
    pub skin: Skin,
}

/// A model a kit dresses a character in (they share the team members' skeleton, so play the
/// same motions), and the textures an actor wears on it
pub struct Body {
    pub model: Crp,
    pub rig: Option<Rig>,
    /// The texture each part wears, the part's own or the actor's in its place, and the
    /// directory it is in
    pub textures: Vec<(PathBuf, String)>,
    /// The model's file and the actor's, which make it this body
    key: (PathBuf, String),
}

/// The models the characters wear, and the motions they play
pub struct Characters {
    /// The team members' model, which players and the characters without a body of their own wear
    pub model: Crp,
    /// Without a skeleton the model stays in its bind pose
    pub rig: Option<Rig>,
    pub bodies: Vec<Body>,
    /// The weapons characters carry: their files in `data/kit`, and what they say
    pub weapons: Vec<(String, crate::weapon::Weapon)>,
    /// By name: the idle motion, injury reactions and deaths
    pub motions: HashMap<String, Motion>,
    /// The name of the idle motion, which also stands in for any that is missing
    pub idle: String,
    /// The motions players' bodies play, for posing them to be shot at
    pub player: crate::body::PlayerMotions,
    /// Those terrorists play with each kind of arms their weapons give them, and which of
    /// them goes with each of `weapons`
    pub arm_sets: Vec<(super::terrorist::Arms, crate::body::PlayerMotions)>,
    pub weapon_arms: Vec<usize>,
}

/// The motions characters play besides idling: flinches from wounds and the falls they die with
pub const REACTION_MOTIONS: [&str; 29] = [
    "thwu_bf", "thwu_bl", "thwu_bb", "thwu_br", "thwu_la", "thwu_ra", "thwu_ll", "thwu_rl",
    "thdu_1", "thdu_2", "thdu_3", "thdu_4", "thdu_e1", "thdu_e2",
    "hw_fr_up2_g", "hw_lf_up2_g", "hw_bk_up2_g", "hw_bk_up2", "hw_rt_up2",
    "thiu_cf", "thiu_cl", "thiu_cb", "thiu_ab", "thiu_hf", "thiu_hb", "thiu_la", "thiu_ra", "thiu_ll", "thiu_rl",
];

/// A motion from the game's data, by name
pub fn load_motion(name: &str) -> Option<Motion> {
    motion::read(&find_file(Path::new("data/motion"), name, "dam").or_else(|| { eprintln!("no motion named {name}"); None })?).ok()
}

/// Where a texture a model wears is: beside it, or else in another character's directory, as
/// an actor's sometimes are (the original finds textures by name wherever they are)
fn texture_dir(beside: &Path, texture: &str) -> PathBuf {
    let t = Path::new(texture);
    let stem = t.file_stem().map_or_else(String::new, |s| s.to_string_lossy().to_string());
    let ext = if t.extension().is_some_and(|e| e.eq_ignore_ascii_case("tga")) { "tga" } else { "rsb" };
    let others = std::fs::read_dir("data/character").into_iter().flatten().flatten().map(|e| e.path());
    std::iter::once(beside.to_path_buf()).chain(others)
        .find(|d| find_file(d, &stem, ext).is_some())
        .unwrap_or_else(|| beside.to_path_buf())
}

/// A model and the skeleton beside it, skinned to it. None when the model cannot be read.
fn load_model(path: &Path) -> Option<(Crp, Option<Rig>)> {
    let model = match crp::read(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("could not load {}: {e:#}", path.display());
            return None;
        }
    };
    let rig = (|| {
        let dir = path.parent()?;
        let stem = path.file_stem()?.to_string_lossy().to_string();
        let skeleton = skeleton::read(&find_file(dir, &stem, "skl").or_else(|| { eprintln!("no skeleton beside {}", path.display()); None })?).ok()?;
        let skin = model.skin(&skeleton).or_else(|| { eprintln!("{} does not match its skeleton", path.display()); None })?;
        Some(Rig { skeleton, skin })
    })();
    Some((model, rig))
}

impl Characters {
    /// Read the model, its skeleton and the motions from the game's data. None when the model
    /// cannot be read. `idle` names the motion characters idle with.
    pub fn load(model_path: &Path, idle: &str) -> Option<Characters> {
        let (model, rig) = load_model(model_path)?;
        println!("loaded {} ({} vertices, {} triangles)", model.name, model.vertices.len(),
            model.parts.iter().map(|p| p.triangles.len()).sum::<usize>());
        let mut motions = HashMap::new();
        let raises = super::terrorist::RAISE_MOTIONS;
        for name in std::iter::once(idle).chain(REACTION_MOTIONS).chain(super::idle::motions()).chain(raises) {
            if motions.contains_key(name) {
                continue;
            }
            if let Some(m) = load_motion(name) {
                motions.insert(name.to_string(), m);
            }
        }
        let player = crate::body::PlayerMotions::load(rig.as_ref().map(|r| &r.skeleton), |_, name| load_motion(name));
        Some(Characters { model, rig, bodies: Vec::new(), weapons: Vec::new(), motions, idle: idle.to_string(), player, arm_sets: Vec::new(), weapon_arms: Vec::new() })
    }

    /// The body a model makes with an actor's textures on it, loaded the first time it is
    /// asked for. None when the model cannot be read.
    pub fn add_body(&mut self, model_path: &Path, actor: Option<&Actor>) -> Option<usize> {
        let key = (model_path.to_path_buf(), actor.map_or_else(String::new, |a| a.id.clone()));
        if let Some(i) = self.bodies.iter().position(|b| b.key == key) {
            return Some(i);
        }
        let (model, rig) = load_model(model_path)?;
        let dir = model_path.parent().unwrap_or(Path::new("."));
        let textures = model.parts.iter().map(|p| {
            let texture = actor.and_then(|a| a.texture_for(&p.material)).unwrap_or(&p.material);
            (texture_dir(dir, texture), texture.to_string())
        }).collect();
        self.bodies.push(Body { model, rig, textures, key });
        Some(self.bodies.len() - 1)
    }

    /// The weapon a file in `data/kit` describes, read the first time it is asked for
    pub fn add_weapon(&mut self, file: &str) -> Option<usize> {
        if let Some(i) = self.weapons.iter().position(|(f, _)| f.eq_ignore_ascii_case(file)) {
            return Some(i);
        }
        let path = find_file(Path::new("data/kit"), &Path::new(file).file_stem()?.to_string_lossy(), "wpn")?;
        match crate::weapon::read(&path) {
            Ok(w) => {
                let arms = super::terrorist::arms(&w);
                let set = self.arm_sets.iter().position(|(a, _)| *a == arms).unwrap_or_else(|| {
                    self.arm_sets.push((arms, self.load_arms(arms)));
                    self.arm_sets.len() - 1
                });
                self.weapons.push((file.to_string(), w));
                self.weapon_arms.push(set);
                Some(self.weapons.len() - 1)
            }
            Err(e) => {
                eprintln!("{e:#}");
                None
            }
        }
    }

    /// The players' motions with a terrorist's arms in place of theirs
    fn load_arms(&self, arms: super::terrorist::Arms) -> crate::body::PlayerMotions {
        let reload = arms.reload.unwrap_or(["", ""]);
        let skeleton = self.rig.as_ref().map(|r| &r.skeleton);
        let mut set = crate::body::PlayerMotions::load(skeleton, |slot, default| {
            let name = match slot {
                "idle" => arms.hold[0],
                "crouch_idle" => arms.hold[1],
                "fire" => arms.fire[0],
                "reload" | "reload_empty" => reload[0],
                "crouch_reload" | "crouch_reload_empty" => reload[1],
                _ => default,
            };
            (!name.is_empty()).then(|| load_motion(name)).flatten()
        });
        set.crouch_fire = load_motion(arms.fire[1]).map(|mut m| {
            if let Some(skel) = skeleton {
                skel.plant(&mut m, 0.0);
            }
            m
        });
        set
    }

    /// The motions a character's body plays: its weapon's arms, else the players'
    pub fn arms_of(&self, npc: &Npc) -> &crate::body::PlayerMotions {
        npc.weapon.and_then(|w| self.weapon_arms.get(w)).and_then(|&k| self.arm_sets.get(k)).map_or(&self.player, |(_, m)| m)
    }

    /// Seconds a character's reload takes with a weapon, standing or kneeling: its motion's
    /// length
    pub fn reload_seconds(&self, weapon: Option<usize>, kneeling: bool) -> Option<f32> {
        let m = weapon.and_then(|w| self.weapon_arms.get(w)).and_then(|&k| self.arm_sets.get(k))?.1.reload(crate::sim::movement::Reloading { progress: 0.0, empty: false }, kneeling)?;
        Some(m.frames.max(1) as f32 / 30.0)
    }

    /// The model a character wears and its rig
    pub fn body(&self, npc: &Npc) -> (&Crp, Option<&Rig>) {
        match npc.body.and_then(|b| self.bodies.get(b)) {
            Some(b) => (&b.model, b.rig.as_ref()),
            None => (&self.model, self.rig.as_ref()),
        }
    }

    /// How many frames a motion lasts, one if it is not loaded
    pub fn frames(&self, motion: &str) -> f32 {
        self.motions.get(motion).map_or(1, |m| m.frames) as f32
    }

    /// Where a character stands, for shots and blasts
    pub fn placement(&self, npc: &Npc) -> Placement {
        Placement {
            pos: [npc.pos[0], if self.body(npc).1.is_some() { npc.floor } else { npc.pos[1] }, npc.pos[2]],
            yaw_deg: npc.yaw,
            scale: npc.scale,
        }
    }

    /// The frame an idle character is on at `now`
    pub fn idle_frame(npc: &Npc, now: f32) -> f32 {
        now * 30.0 + npc.stagger * 9.0
    }

    /// The model's vertices posed for what the character is doing: waiting in its stance,
    /// idling at `idle_frame` without one, reacting to a hit or dead. Empty when there is no
    /// skeleton to pose it with.
    pub fn pose(&self, npc: &Npc, now: f32, idle_frame: f32) -> Vec<V3> {
        let (model, Some(rig)) = self.body(npc) else { return Vec::new() };
        match self.bones(npc, now, idle_frame) {
            Some(bones) => rig.skin.apply(&bones),
            None => model.vertices.clone(),
        }
    }

    /// The skeleton posed as `pose` poses the model, in the model's space. None without a
    /// skeleton, or a motion to play.
    pub fn bones(&self, npc: &Npc, now: f32, idle_frame: f32) -> Option<Vec<skeleton::Transform>> {
        let skeleton = &self.body(npc).1?.skeleton;
        let standing = matches!(npc.health.state, NpcState::Idle);
        let (root, rots) = if npc.walking && standing {
            self.walking_pose(npc, skeleton, now)?
        } else if npc.aiming && standing {
            self.aiming_pose(npc, skeleton, now, idle_frame)?
        } else {
            self.waiting_pose(npc, now, idle_frame)?
        };
        Some(skeleton.pose(root, &rots))
    }

    /// Aiming, the gun up as a player's is when they fire, still between shots; first raising
    /// it (see `terrorist::raise`), standing up with it or easing into aiming from waiting
    fn aiming_pose(&self, npc: &Npc, skeleton: &Skeleton, now: f32, idle_frame: f32) -> Option<(V3, Vec<[f32; 4]>)> {
        let firing = now - npc.last_shot < 0.15;
        let kneeling = super::terrorist::kneels(npc.stance);
        let arms = self.arms_of(npc);
        // Reloading, its weapon's reload plays through, whole
        let reload = npc.reload.and_then(|t| Some((t, arms.reload(crate::sim::movement::Reloading { progress: t, empty: false }, kneeling)?)));
        if let Some((t, m)) = reload {
            let (root, rots) = m.sample(t.clamp(0.0, 1.0) * (m.frames.max(1) - 1) as f32);
            return Some(([0.0, root[1], 0.0], rots));
        }
        let doing = crate::body::Doing {
            movement: crate::sim::movement::Movement::Fire,
            frame: 0.0, crouched: kneeling, reloading: None, firing: true, flinch: None, peek: None,
            hip_turn: 0.0, changing: None, shuffle: None, clock: if firing { now } else { 0.0 },
        };
        let pose = crate::body::pose(arms, skeleton, &doing)?;
        let up = ([0.0, pose.root[1], 0.0], pose.rotations);
        let (motion, seconds) = super::terrorist::raise(npc.stance, |m| self.frames(m));
        let t = now - npc.aim_since;
        if t >= seconds {
            return Some(up);
        }
        match motion.and_then(|m| self.motions.get(m)) {
            Some(m) => {
                let (root, rots) = m.sample((t * 30.0).min(m.frames.max(1) as f32 - 1.0));
                Some(([0.0, root[1], 0.0], rots))
            }
            None => {
                let (mut root, mut rots) = self.waiting_pose(npc, now, idle_frame)?;
                let w = t / seconds;
                for (q, a) in rots.iter_mut().zip(&up.1) {
                    *q = motion::slerp(*q, *a, w);
                }
                root[1] += (up.0[1] - root[1]) * w;
                Some(([0.0, root[1], 0.0], rots))
            }
        }
    }

    /// Walking or running as a player does, its weapon up if it is alert; creeping is a walk at
    /// half its pace
    fn walking_pose(&self, npc: &Npc, skeleton: &Skeleton, now: f32) -> Option<(V3, Vec<[f32; 4]>)> {
        use crate::sim::movement::Movement;
        let (movement, pace) = match npc.gait {
            Gait::Creep => (Movement::Walk, 0.5),
            Gait::Walk => (Movement::Walk, 1.0),
            Gait::Run => (Movement::Run, 1.0),
        };
        // The motion loops, at the pace players' play at
        let arms = self.arms_of(npc);
        let frames = arms.get(movement, false).map_or(1, |m| m.frames.max(1)) as f32;
        let doing = crate::body::Doing {
            movement,
            frame: (now * 30.0 * super::ANIM_PACE * pace) % frames, crouched: false, reloading: None, firing: npc.aiming, flinch: None, peek: None,
            hip_turn: 0.0, changing: None, shuffle: None, clock: 0.0,
        };
        let pose = crate::body::pose(arms, skeleton, &doing)?;
        Some(([0.0, pose.root[1], 0.0], pose.rotations))
    }

    /// Waiting in its stance or idling, reacting to a hit or dead: where the root goes and how
    /// each bone turns
    fn waiting_pose(&self, npc: &Npc, now: f32, idle_frame: f32) -> Option<(V3, Vec<[f32; 4]>)> {
        let playing = |motion, frame, looping| super::idle::Playing { motion, frame, looping };
        let (now_playing, from) = match (npc.health.state, npc.stance) {
            (NpcState::Idle, Some(stance)) => super::idle::at(stance, npc.stagger.to_bits() as u64, now, |m| self.frames(m)),
            (NpcState::Idle, None) => (playing(self.idle.as_str(), idle_frame, true), None),
            (NpcState::Reacting { motion, start }, _) => (playing(motion, (now - start) * 30.0, false), None),
            (NpcState::Dead { motion, start }, _) => (playing(motion, (now - start) * 30.0, false), None),
        };
        let sample = |p: super::idle::Playing| {
            let mot = self.motions.get(p.motion).or_else(|| self.motions.get(self.idle.as_str()))?;
            let frames = mot.frames.max(1) as f32;
            let f = if p.looping { p.frame % frames } else { p.frame.min(frames - 1.0) };
            let (root, rots) = mot.sample(f);
            Some((root, rots, mot.keys[0].root))
        };
        let (mut root, mut rots, k0) = sample(now_playing)?;
        // Waiting, one motion blends in from the one before
        if let Some((before, weight)) = from.and_then(|(b, w)| Some((sample(b)?, w))) {
            for (q, b) in rots.iter_mut().zip(&before.1) {
                *q = motion::slerp(*q, *b, weight);
            }
            root[1] += (before.0[1] - root[1]) * weight;
        }
        // Hit and death motions carry the body along with them; waiting stays in place
        let waiting = matches!(npc.health.state, NpcState::Idle);
        let (dx, dz) = if now_playing.looping || waiting { (0.0, 0.0) } else { (root[0] - k0[0], root[2] - k0[2]) };
        Some(([dx, root[1], dz], rots))
    }

    /// Where a shot from `origin` along the unit vector `dir` strikes the character, if it does
    pub fn raycast(&self, npc: &Npc, now: f32, origin: V3, dir: V3) -> Option<combat::Hit> {
        let (model, rig) = self.body(npc);
        let rig = rig?;
        let verts = self.pose(npc, now, Self::idle_frame(npc, now));
        combat::raycast_character(model, &rig.skeleton, &rig.skin, &verts, &self.placement(npc), origin, dir)
    }
}

/// How far apart the members of a team stand, side by side across where it starts
const TEAM_SPACING: f32 = 70.0;

/// The kit a member wears, and their actor, if the game has them
fn dress(member: &crate::mission::Member) -> (crate::kit::Kit, Option<Actor>) {
    let file = |dir: &str, name: &str| {
        let name = Path::new(name);
        find_file(Path::new(dir), &name.file_stem()?.to_string_lossy(), &name.extension()?.to_string_lossy())
    };
    let kit = file("data/kit", &member.kit).and_then(|k| crate::kit::read(&k).ok()).unwrap_or_default();
    let actor = file("data/actor", &member.actor).and_then(|a| actor::read(&a).ok());
    if kit.model.is_none() {
        eprintln!("{} wears the team members' model: no model for the kit {}", member.name, member.kit);
    }
    (kit, actor)
}

/// Stand each of a mission's terrorists and hostages where their team starts: at one of its places, picked
/// with `seed`, facing its heading (256ths of a turn, 0 along +z, as examples/mission-headings.rs
/// finds), the members side by side where there is room. Each wears the model their kit names
/// with their actor's textures, which become `characters`' bodies, holds the kit's weapon, and
/// waits in their stance.
/// Armed terrorists see as far as the mission says, and patrol its zones. Hostages carry
/// nothing, so they only wait.
pub fn place_terrorists(teams: &[crate::mission::Team], seed: u64, setting: &crate::mission::Setting, world: &CollisionWorld, characters: &mut Characters) -> Vec<Npc> {
    let sight = setting.sight;
    // Hostages pick their places apart, so terrorists start where they did before hostages
    let (mut rng, mut hostage_rng) = (seed | 1, seed.rotate_left(17) | 1);
    let mut npcs = Vec::new();
    for (team_index, team) in teams.iter().enumerate() {
        let orders = std::rc::Rc::new(super::terrorist::Orders { plan: team.plan.clone(), reactions: team.reactions.clone(), zones: setting.zones.clone() });
        let rng = if team.terrorists { &mut rng } else { &mut hostage_rng };
        let pick = (crate::util::next_rng(rng) >> 33) as usize % team.starts.len();
        let (at, heading) = team.starts[pick];
        let yaw = heading * 360.0 / 256.0;
        let (sin, cos) = yaw.to_radians().sin_cos();
        let n = team.members.len();
        for (k, member) in team.members.iter().enumerate() {
            let across = (k as f32 - (n - 1) as f32 / 2.0) * TEAM_SPACING;
            // Beside the place, unless a wall is there
            let beside = [at[0] + cos * across, at[1], at[2] - sin * across];
            let clear = |q: V3| world.wall_clearance(&crate::collision::Player::new([q[0], world.ground_at(q[0], q[2], q[1] + 100.0).unwrap_or(q[1]), q[2]])) >= 0.0;
            let p = if clear(beside) || !clear(at) { beside } else { at };
            let floor = world.ground_at(p[0], p[2], p[1] + 100.0).unwrap_or(p[1]);
            let (kit, actor) = dress(member);
            let body = kit.model.and_then(|m| characters.add_body(&m, actor.as_ref()));
            let weapon = kit.weapon.as_ref().and_then(|(w, _)| characters.add_weapon(w));
            // Armed, they fight
            let mind = weapon.and_then(|w| {
                let (_, wpn) = &characters.weapons[w];
                let (skill, self_control) = actor.as_ref().map_or((0.5, 0.5), |a| (a.assault / 100.0, a.self_control / 100.0));
                let magazines = kit.weapon.as_ref().map_or(1, |(_, n)| *n);
                let mut mind = super::terrorist::Mind::new(member.alertness, skill, self_control, sight, wpn.gun?, wpn.magazine, magazines);
                if let Some((file, n)) = &kit.sidearm {
                    let side = characters.add_weapon(file).and_then(|s| Some((s, characters.weapons[s].1.gun?, characters.weapons[s].1.magazine)));
                    if let Some((s, gun, magazine)) = side {
                        mind = mind.with_sidearm(s, gun, magazine, *n);
                    }
                }
                mind.member = k;
                Some(mind.with_orders(team_index, orders.clone()))
            });
            let scale = actor.as_ref().map_or([1.0; 3], |a| a.scale);
            let mut npc = Npc {
                health: actor.as_ref().map_or_else(Health::new, |a| Health::with_stamina(a.stamina)),
                pos: p,
                floor,
                yaw,
                scale,
                stagger: npcs.len() as f32 * 0.73,
                body,
                stance: Some(member.stance),
                weapon,
                mind,
                name: member.name.clone(),
                ..Npc::default()
            };
            let feet_to_origin = -characters.body(&npc).0.bounds().0[1];
            npc.pos[1] = floor + feet_to_origin * scale[1];
            npcs.push(npc);
        }
    }
    npcs
}

/// Stand a character at each spawn point (a point on the map's floor) but those in `skip`,
/// dealing the team members' faces out in turn. `yaw` says which way the character at a
/// spawn point faces, given its index and position. Also returns which team member each one is.
pub fn place_npcs(
    spawns: &[V3],
    skip: &[usize],
    world: &CollisionWorld,
    model: &Crp,
    actors: &[Actor],
    yaw: &dyn Fn(usize, V3) -> f32,
) -> (Vec<Npc>, Vec<Option<usize>>) {
    let feet_to_origin = -model.bounds().0[1];
    let (mut npcs, mut faces) = (Vec::new(), Vec::new());
    for (n, (i, p)) in spawns.iter().enumerate().filter(|(i, _)| !skip.contains(i)).enumerate() {
        let floor = world.ground_at(p[0], p[2], p[1] + 100.0).unwrap_or(p[1]);
        let actor = (!actors.is_empty()).then(|| n % actors.len());
        let scale_y = actor.map_or(1.0, |a| actors[a].scale[1]);
        npcs.push(Npc {
            health: Health::new(),
            pos: [p[0], floor + feet_to_origin * scale_y, p[2]],
            floor,
            yaw: yaw(i, *p),
            scale: actor.map_or([1.0; 3], |a| actors[a].scale),
            stagger: i as f32 * 0.73,
            ..Npc::default()
        });
        faces.push(actor);
    }
    (npcs, faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{Params, Player};
    use crate::mission::{self, Difficulty};

    #[test]
    fn every_missions_terrorists_stand_on_a_floor_clear_of_the_walls_dressed_by_their_kit() {
        let Some(mut characters) = Characters::load(Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc") else { return };
        let (mut placed, mut in_walls, mut sidearms) = (0, Vec::new(), 0);
        for level in 1..=19 {
            let map_path = format!("data/map/rm{level:02}/rm{level:02}.map");
            let Some((_, teams)) = mission::teams(Path::new(&map_path), Difficulty::Medium) else { continue };
            let level_map = crate::map::read(Path::new(&map_path)).unwrap();
            let world = CollisionWorld::from_map(&level_map, Params::default());
            let npcs = place_terrorists(&teams, 7, &mission::Setting::default(), &world, &mut characters);
            let members: Vec<_> = teams.iter().flat_map(|t| &t.members).collect();
            assert_eq!(npcs.len(), members.len());
            for (n, m) in npcs.iter().zip(&members) {
                assert_eq!(n.stance, Some(m.stance));
                // Some carry nothing, as a security guard lying dead on rm01
                assert_eq!(n.weapon.is_some(), dress(m).0.weapon.is_some(), "rm{level:02}: {}", m.name);
                if let (Some(side), Some(mind)) = (dress(m).0.sidearm, &n.mind) {
                    assert!(mind.sidearm.is_some(), "rm{level:02}: {} has no {}", m.name, side.0);
                    sidearms += 1;
                }
                let body = &characters.bodies[n.body.unwrap_or_else(|| panic!("rm{level:02}: {} wears no model of their own", m.name))];
                assert!(body.rig.is_some(), "{}", m.kit);
                for (dir, t) in &body.textures {
                    let t = Path::new(t);
                    let ext = if t.extension().is_some_and(|e| e.eq_ignore_ascii_case("tga")) { "tga" } else { "rsb" };
                    assert!(find_file(dir, &t.file_stem().unwrap().to_string_lossy(), ext).is_some(), "{} has no {}", dir.display(), t.display());
                }
            }
            for n in &npcs {
                let floor = world.ground_at(n.pos[0], n.pos[2], n.floor + 1.0);
                assert!(floor.is_some_and(|f| (f - n.floor).abs() < 1.0), "rm{level:02}: no floor under {:?}", n.pos);
                let feet = Player::new([n.pos[0], n.floor, n.pos[2]]);
                if world.wall_clearance(&feet) < -5.0 {
                    in_walls.push(format!("rm{level:02} {:?}", n.pos));
                }
                placed += 1;
            }
        }
        assert!(placed == 0 || in_walls.len() * 50 < placed, "{} of {placed} in walls: {in_walls:?}", in_walls.len());
        assert!(placed == 0 || sidearms > 0, "no one carries a sidearm");
    }

    #[test]
    fn a_terrorist_holds_fires_and_reloads_as_its_weapon_has_it() {
        use super::super::terrorist::arms;
        let read = |f: &str| crate::weapon::read(&Path::new("data/kit").join(f)).ok();
        let Some(ak) = read("tw_ak47.wpn") else { return };
        assert_eq!((arms(&ak).hold, arms(&ak).fire, arms(&ak).reload), (["rwru_l1", "rwrc_l1"], ["rwru_l9", "rwrc_l9"], Some(["ter_mag_change", "twm_mp3"])));
        // An Uzi is held and reloaded as a pistol is
        let uzi = read("tw_uzi.wpn").unwrap();
        assert_eq!((arms(&uzi).hold, arms(&uzi).fire, arms(&uzi).reload), (["rwru_p1", "rwrc_p1"], ["rwru_p9", "rwrc_p9"], Some(["ter_pis_mag_change", "mg_w1_cr"])));
        assert_eq!(arms(&read("sw_de357.wpn").unwrap()), arms(&uzi));
        assert_eq!(arms(&read("tw_dragunov.wpn").unwrap()), arms(&ak));

        // In the game: the weapons that are held alike share their motions, all of them there
        let Some(mut chars) = Characters::load(Path::new("data/character/team_hrt/blue_M_a.CRP"), "up_none_cyc") else { return };
        let [ak, ak74, pistol] = ["tw_ak47.wpn", "tw_ak74.wpn", "sw_de357.wpn"].map(|w| chars.add_weapon(w));
        assert_eq!(chars.weapon_arms[ak.unwrap()], chars.weapon_arms[ak74.unwrap()]);
        assert_ne!(chars.weapon_arms[ak.unwrap()], chars.weapon_arms[pistol.unwrap()]);
        for (_, set) in &chars.arm_sets {
            assert!(set.idle.is_some() && set.crouch_idle.is_some() && set.fire.is_some() && set.crouch_fire.is_some() && set.reload.is_some() && set.crouch_reload.is_some());
        }
        // A reload takes as long as its motion
        let pistol_reload = chars.reload_seconds(pistol, false).unwrap();
        assert_eq!(pistol_reload, load_motion("ter_pis_mag_change").unwrap().frames as f32 / 30.0);
        assert!(chars.reload_seconds(None, false).is_none());

        // Aiming with a pistol is not aiming with a rifle, and reloading is neither
        let npc = |weapon: Option<usize>, reload: Option<f32>| Npc { weapon, reload, aiming: true, stance: Some(crate::mission::Stance::Stand), ..Npc::default() };
        let bones = |n: &Npc| chars.bones(n, 10.0, 0.0).unwrap();
        let hand = |b: &Vec<crate::skeleton::Transform>| format!("{:?}", b[b.len() / 2]);
        let (rifle, handgun, reloading) = (bones(&npc(ak, None)), bones(&npc(pistol, None)), bones(&npc(ak, Some(0.5))));
        assert_ne!(hand(&rifle), hand(&handgun));
        assert_ne!(hand(&rifle), hand(&reloading));
    }
}
