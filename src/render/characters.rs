//! Drawing the characters: those at the spawn points, the other players, the player's own body,
//! the dead, and the grenades in the air.

use std::collections::HashMap;
use rogue_reborn::{blastlight, body, casings, combat, crp, map, motion, sim};
use rogue_reborn::lighting::CharacterLight;
use rogue_reborn::net::animation::Doing;
use rogue_reborn::net::client::{Frame, Seen};
use rogue_reborn::sim::characters::{Characters, Npc, Rig};
use rogue_reborn::sim::movement::{Movement, Reloading};
use rogue_reborn::sim::peek::Peek;
use crate::args::Args;
use crate::opengl::*;
use crate::player::{BodyKey, BodyState};
use crate::render::assets::{Assets, Uniform};
use crate::render::draw::{CharacterArrays, FlashSpot, WeaponDraw, draw_character, draw_weapon};
use crate::render::world::WorldDrawn;
use crate::scene::{NPC_OWNERS, Scene};
use crate::session::Session;
use crate::state::{Timing, Toggles};
use crate::view::PlayerView;

// What the frame's drawing notes as it goes, for the passes after it: where the muzzle flashes
// are and where the guns of those that fired are, and the dice for the flashes' pictures
pub struct FrameOutput {
    pub flash_spots: Vec<FlashSpot>,
    pub casing_spots: Vec<(usize, casings::Ejector, [f32; 16])>,
    flash_rng: u64,
}

impl FrameOutput {
    pub fn new() -> FrameOutput {
        FrameOutput { flash_spots: Vec::new(), casing_spots: Vec::new(), flash_rng: 0x9E37_79B9_7F4A_7C15u64 }
    }

    pub fn flash_roll(&mut self) -> f32 {
        let x = rogue_reborn::util::next_rng(&mut self.flash_rng);
        (x >> 40) as f32 / (1u64 << 24) as f32
    }
}

// What the characters' drawing reads besides the scene and the view
pub struct CharacterPass<'a> {
    pub args: &'a Args,
    pub assets: &'a Assets,
    pub level: &'a map::Map,
    pub sim: &'a sim::Sim,
    pub session: &'a Session,
    pub toggles: &'a Toggles,
    pub timing: &'a Timing,
    pub drawn: &'a WorldDrawn,
    pub frame: Option<&'a Frame>,
    pub me: usize,
    pub game_time: f32,
    pub step_dt: f32,
    pub npc_actor: &'a [Option<usize>],
    pub player_flash: &'a [bool],
    pub npc_flash: &'a [bool],
    pub watched_body: Option<usize>,
    pub draw_feet: [f32; 3],
    pub player_ejector: Option<&'a casings::Ejector>,
}

// What the pass draws with: its reads, the blasts that light characters, and the rounds fired
struct Stage<'a> {
    pass: &'a CharacterPass<'a>,
    blast_lights: &'a [blastlight::BlastLight],
    casing_pending: &'a HashMap<usize, u32>,
}

// What one posed body is doing, as `Stage::draw_body` draws it
// `reloading` is the reload they are in the middle of, if any: its motion and frame
// follow its progress, on the beats of its sound. `firing` says the gun is firing,
// which the arms show whatever the legs do. `crouched` kneels the body for what is
// done in place, and a change of stance plays crouching down or standing up. `flinch`
// is a wound's flinch, its frame and when it began. It takes the whole body standing
// in place and the upper body walking. `hip_turn` is how far the hips would turn
// toward a side step (see `movement::hip_turn`). `peek` takes the whole body while it
// lasts.
struct BodyDraw<'a> {
    side: Option<sim::Side>,
    movement: Movement,
    frame: f32,
    feet: [f32; 3],
    yaw_deg: f32,
    scale: [f32; 3],
    face: Option<u32>,
    hand: Option<&'a Option<WeaponDraw>>,
    grenade: bool,
    flash: bool,
    owner: usize,
    reloading: Option<Reloading>,
    firing: bool,
    crouched: bool,
    limp: bool,
    hip_turn: f32,
    flinch: Option<(&'a motion::Motion, f32, f32)>,
    peek: Option<Peek>,
}

/// The light of the room at `room_at`, weighed at `weigh_at`, in the level's space
fn room_light(lighting: &rogue_reborn::lighting::Lighting, level: &map::Map, visibility: &rogue_reborn::visibility::Visibility, room_at: [f32; 3], weigh_at: [f32; 3]) -> CharacterLight {
    let room = level.rooms.locate(room_at, None).and_then(|i| visibility.number_of(i));
    lighting.character(room, weigh_at)
}

/// The light on a character standing at `feet`: its room is found just above the floor, and
/// the light is weighed at the chest
pub fn standing_light(lighting: &rogue_reborn::lighting::Lighting, level: &map::Map, visibility: &rogue_reborn::visibility::Visibility, feet: [f32; 3]) -> CharacterLight {
    room_light(lighting, level, visibility, [feet[0], feet[1] + 10.0, feet[2]], [feet[0], feet[1] + 100.0, feet[2]])
}

/// What a player on `side` is drawn with: their side's uniform, or else the players' model
fn dress<'a>(side: Option<sim::Side>, sides: &'a [Option<Uniform>], model: &'a crp::Crp, arrays: &'a CharacterArrays,
             textures: &'a [Option<u32>], skin: Option<&'a crp::Skin>) -> (&'a crp::Crp, &'a CharacterArrays, &'a [Option<u32>], Option<&'a crp::Skin>) {
    match side.and_then(|s| sides.get(s.index())).and_then(Option::as_ref) {
        Some(u) => (&u.model, &u.arrays, &u.textures, Some(&u.skin)),
        None => (model, arrays, textures, skin),
    }
}

/// The flinch a living player's latest wound plays at `now`, while it lasts
fn flinch_at<'a>(chars: &'a Characters, p: &sim::PlayerState, now: f32) -> Option<(&'a motion::Motion, f32, f32)> {
    let hit = p.hit.filter(|_| p.respawn_at.is_none())?;
    let m = chars.motions.get(hit.motion)?;
    let frame = (now - hit.start) * 30.0;
    (0.0..m.frames as f32).contains(&frame).then_some((m, frame, hit.start))
}

// A flashing gun lights its shooter. With no falloff only the direction matters, so the
// light sits ahead of the chest, at gun height, in the character's own space.
fn flashing(lit: Option<CharacterLight>, on: bool) -> Option<CharacterLight> {
    lit.map(|l| if on { l.with_muzzle([0.0, 100.0, 60.0]) } else { l })
}

fn tint(lit: &Option<CharacterLight>) -> [f32; 3] {
    lit.as_ref().map_or([1.0; 3], |l| l.tint)
}

impl Stage<'_> {
    // A character shows only when a room it is in is drawn
    fn shown(&self, feet: [f32; 3]) -> bool {
        self.pass.drawn.shows(self.pass.sim, self.pass.level, feet)
    }

    // The light of a blast on a character standing at `feet`: nothing when there is none
    fn glow_on(&self, feet: [f32; 3]) -> [f32; 3] {
        let sim = self.pass.sim;
        let mut sum = [0.0f32; 3];
        let chest = [feet[0], feet[1] + 100.0, feet[2]];
        let blocked = |from: [f32; 3], to: [f32; 3]| {
            let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
            let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            dist > 10.0 && sim.world.raycast(from, [d[0] / dist, d[1] / dist, d[2] / dist], dist - 10.0).is_some()
        };
        for l in self.blast_lights {
            let g = l.glow_at(chest, &blocked);
            for k in 0..3 {
                sum[k] += g[k];
            }
        }
        sum
    }

    // The level's light on a character standing at `feet`, turned `yaw` and scaled, in its own
    // space: its room's ambient light and strongest lights. None with --flat-light.
    fn lit_at(&self, feet: [f32; 3], yaw: f32, scale: [f32; 3]) -> Option<CharacterLight> {
        Some(standing_light(self.pass.session.lighting.as_ref()?, self.pass.level, &self.pass.sim.visibility, feet).local(feet, yaw, scale))
    }

    fn note_flash(&self, wd: &WeaponDraw, out: &mut FrameOutput) {
        let roll = out.flash_roll();
        out.flash_spots.push(FlashSpot::here(wd, roll));
    }

    fn note_casings(&self, owner: usize, ejector: &casings::Ejector, out: &mut FrameOutput) {
        if self.casing_pending.contains_key(&owner) {
            let mut matrix = [0f32; 16];
            unsafe { glGetFloatv(GL_MODELVIEW_MATRIX, matrix.as_mut_ptr()) };
            out.casing_spots.push((owner, *ejector, matrix));
        }
    }

    // Characters at the spawn points, posed for what they are doing
    fn draw_npcs(&self, out: &mut FrameOutput) {
        let CharacterPass { args, assets, sim, toggles, timing, frame, game_time, npc_actor, npc_flash, .. } = *self.pass;
        let Some(chars) = &sim.characters else { return };
        unsafe {
            let c = &chars.model;
            glDisable(GL_TEXTURE_2D);
            let clock = if timing.fixed_step { game_time as f32 } else { timing.start_time.elapsed().as_secs_f32() };
            for (i, spot) in sim.npcs.iter().enumerate() {
                let mut moved;
                let spot = match frame.and_then(|v| v.npcs.get(i)) {
                    Some(&(pos, yaw)) => {
                        moved = spot.clone();
                        (moved.pos[0], moved.pos[2], moved.yaw) = (pos[0], pos[2], yaw);
                        &moved
                    }
                    None => spot,
                };
                if !self.shown([spot.pos[0], spot.floor, spot.pos[2]]) {
                    continue;
                }
                let idle = args.anim_frame.unwrap_or_else(|| game_time * args.anim_fps.unwrap_or(30.0) + spot.stagger * 9.0);
                let (model, arrays, textures) = match spot.body.and_then(|b| chars.bodies.get(b).zip(assets.body_draws.get(b))) {
                    Some((b, (arrays, textures))) => (&b.model, arrays, textures.as_slice()),
                    None => (c, &assets.character_arrays, assets.part_textures.as_slice()),
                };
                let bones = chars.bones(spot, game_time, idle);
                let (posed, turns) = match (&bones, chars.body(spot).1) {
                    (Some(bones), Some(rig)) => (rig.skin.apply(bones), rig.skin.turns(bones)),
                    (None, Some(_)) => (model.vertices.clone(), Vec::new()),
                    (_, None) => (Vec::new(), Vec::new()),
                };
                let verts: &[[f32; 3]] = if posed.is_empty() { &model.vertices } else { &posed };
                let y = if posed.is_empty() { spot.pos[1] } else { spot.floor };
                glPushMatrix();
                glTranslatef(spot.pos[0], y, spot.pos[2]);
                glRotatef(spot.yaw, 0.0, 1.0, 0.0);
                glScalef(spot.scale[0], spot.scale[1], spot.scale[2]);
                // Eyes shut for 0.15 s every 4 s, staggered per model; the dead keep them shut
                let blinking = args.eyes_closed || spot.health.is_dead()
                    || (toggles.blink_enabled && (clock + spot.stagger) % 4.0 < 0.15);
                let face = npc_actor[i].map(|a| assets.face_textures[a]).and_then(|(main, blink)| {
                    if blinking { blink.or(main) } else { main }
                });
                let held = spot.weapon.and_then(|w| assets.held_draws.get(w)).and_then(Option::as_ref);
                let glowing = npc_flash.get(i).copied().unwrap_or(false) && held.is_some_and(|w| w.flash_scale > 0.0) && !spot.health.is_dead();
                let lit = flashing(self.lit_at([spot.pos[0], y, spot.pos[2]], spot.yaw, spot.scale), glowing);
                draw_character(model, arrays, verts, &turns, textures, face, toggles.textured, self.glow_on([spot.pos[0], y, spot.pos[2]]), lit.as_ref());
                // Their weapon in the right hand, as the players hold theirs, or on the
                // floor once they have dropped it dying
                if let (Some(bones), Some(wd), Some((hand, off))) = (&bones, held, &assets.weapon_attach) {
                    glPushMatrix();
                    let lying = match chars.dropped_weapon(spot, game_time, *hand, off) {
                        Some(lying) => {
                            glMultMatrixf(lying.as_ptr());
                            true
                        }
                        None => {
                            glMultMatrixf(bones[*hand].matrix().as_ptr());
                            glMultMatrixf(off.matrix().as_ptr());
                            false
                        }
                    };
                    draw_weapon(wd, toggles.textured, tint(&lit));
                    if !lying && npc_flash.get(i).copied().unwrap_or(false) {
                        self.note_flash(wd, out);
                    }
                    if let (false, Some(e)) = (lying, spot.weapon.and_then(|w| assets.held_ejectors.get(w)).and_then(Option::as_ref)) {
                        self.note_casings(NPC_OWNERS + i, e, out);
                    }
                    glPopMatrix();
                }
                glPopMatrix();
            }
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
    }

    // How a posed body is drawn, for the other players and for the player's own
    // `body` is what the drawing remembers of this body from frame to frame.
    fn draw_body(&self, chars: &Characters, d: BodyDraw, body: &mut BodyState, out: &mut FrameOutput) -> bool {
        let CharacterPass { assets, toggles, game_time, player_ejector, .. } = *self.pass;
        let BodyDraw { side, movement, frame, feet, yaw_deg, scale, face, hand, grenade, flash, owner, reloading, firing, crouched, limp, hip_turn, flinch, peek } = d;
        let Some(Rig { skeleton: skel, skin }) = &chars.rig else { return false };
        let moving = assets.player_motions.moving(movement, peek, flinch.is_some());
        // Crouching down or standing up plays in place, at its own pace; moving cuts it short
        let since = body.since_stance_change(crouched, game_time);
        let changing = since.filter(|_| !moving)
            .and_then(|t| assets.player_motions.stance_change(crouched).map(|m| (m, t * 30.0)))
            .filter(|(m, f)| *f < m.frames.saturating_sub(1) as f32);
        // Standing in place, the feet step round once the body has turned far enough
        let shuffle = body.shuffle(yaw_deg, !moving && changing.is_none(), game_time)
            .and_then(|at| assets.player_motions.turn(crouched).map(|m| (m, (game_time - at) * 30.0, at)))
            .filter(|(m, f, _)| *f < m.frames.saturating_sub(1) as f32);
        let turn = body.turn_hips(if moving { hip_turn } else { 0.0 }, game_time);
        let doing = body::Doing {
            movement, frame, crouched, limp, reloading, firing, flinch: flinch.map(|(m, f, _)| (m, f)), peek, hip_turn: turn,
            changing, shuffle: shuffle.map(|(m, f, _)| (m, f)), clock: game_time,
        };
        let Some(body::Pose { mut root, rotations: mut rots, aiming, peeking, shuffling }) = body::pose(&assets.player_motions, skel, &doing) else { return false };
        // Raising the gun to fire is a cut, as in the original, so the first shots never
        // leave a gun still on its way up. Once firing, the legs still ease.
        let raising = aiming && !body.fade.key().is_some_and(|k| k.aiming);
        let ease = if raising { 0.0 } else { motion::CROSSFADE_SECONDS };
        let key = BodyKey { movement, reload: reloading.map(|r| r.empty), aiming, crouched, changing: changing.is_some(), flinch: flinch.map(|f| f.2.to_bits()), shuffle: shuffle.filter(|_| shuffling).map(|s| s.2.to_bits()), peek: peeking };
        body.fade.apply(key, game_time, ease, &mut root, &mut rots);
        let pose = skel.pose([0.0, root[1], 0.0], &rots);
        let (model, arrays, textures, worn) = dress(side, &assets.side_dress, &chars.model, &assets.character_arrays, &assets.part_textures, Some(skin));
        let verts = worn.unwrap_or(skin).apply(&pose);
        let turns = worn.unwrap_or(skin).turns(&pose);
        unsafe {
            glPushMatrix();
            glTranslatef(feet[0], feet[1], feet[2]);
            // The body faces where the player looks (model forward is +Z)
            glRotatef(yaw_deg, 0.0, 1.0, 0.0);
            glScalef(scale[0], scale[1], scale[2]);
            let glowing = flash && hand.and_then(Option::as_ref).is_some_and(|w| w.flash_scale > 0.0);
            let lit = flashing(self.lit_at(feet, yaw_deg, scale), glowing);
            draw_character(model, arrays, &verts, &turns, textures, face, toggles.textured, self.glow_on(feet), lit.as_ref());
            // What bullets hit (B): the players' model posed alike, whatever the uniform,
            // outlined over it
            if toggles.show_hitboxes {
                let verts = skin.apply(&pose);
                glDisable(GL_TEXTURE_2D);
                glDisable(GL_DEPTH_TEST);
                glColor3f(1.0, 0.85, 0.0);
                glBegin(GL_LINES);
                for t in chars.model.parts.iter().flat_map(|p| &p.triangles) {
                    for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                        let (a, b) = (verts[a as usize], verts[b as usize]);
                        glVertex3f(a[0], a[1], a[2]);
                        glVertex3f(b[0], b[1], b[2]);
                    }
                }
                glEnd();
                glEnable(GL_DEPTH_TEST);
                glColor4f(1.0, 1.0, 1.0, 1.0);
            }

            // The MP5K in the right hand, or a grenade until it is thrown
            if let (Some(Some(wd)), Some((hand, off))) = (hand, &assets.weapon_attach) {
                glPushMatrix();
                glMultMatrixf(pose[*hand].matrix().as_ptr());
                glMultMatrixf(off.matrix().as_ptr());
                if grenade {
                    glTranslatef(-wd.center[0], -wd.center[1], -wd.center[2]);
                }
                draw_weapon(wd, toggles.textured, tint(&lit));
                if flash {
                    self.note_flash(wd, out);
                }
                if let (false, Some(e)) = (grenade, player_ejector) {
                    self.note_casings(owner, e, out);
                }
                glPopMatrix();
            }
            glPopMatrix();
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
        true
    }

    // The other players on the server, moving and handling their weapons as they are seen to
    fn draw_others(&self, remote_anim: &mut rogue_reborn::net::animation::Animators, bodies: &mut HashMap<usize, BodyState>, out: &mut FrameOutput) {
        let CharacterPass { args, assets, sim, toggles, frame: view, me, game_time, step_dt, player_flash, watched_body, .. } = *self.pass;
        let Some(chars) = &sim.characters else { return };
        unsafe {
            glDisable(GL_TEXTURE_2D);
            let others = view.map_or(&[][..], |v| &v.players[..]);
            // Alive or dead as the view has them, at the moment it shows, not as the newest snapshot says
            let view_time = view.map_or(game_time, |v| v.time);
            // Not the one watched, through whose eyes the view is
            for &Seen { slot, feet: pos, look, dead, crouched, peek, .. } in others.iter().filter(|s| s.slot != me && Some(s.slot) != watched_body) {
                let visible = self.shown(pos);
                let p = &sim.players[slot];
                let yaw = sim::dequantize(look.0);
                let face = (!assets.actors.is_empty()).then(|| assets.face_textures[slot % assets.actors.len()].0).flatten();
                if dead.is_none() {
                    let doing = Doing {
                        throwing: p.throw.map(|t| (game_time - t.start) / sim::THROW_SECONDS),
                        reloading: p.reloading(game_time),
                        // The server sends others' trigger only while their gun has rounds
                        firing: p.gun_trigger,
                        crouched,
                        wounds: p.wounds,
                        leg_wounds: p.leg_wounds,
                    };
                    let limp = p.leg_wounds > 0;
                    let frames = |m: Movement| assets.player_motions.get(m, doing.crouched, limp).map_or(1, |m| m.frames.max(1)) as f32;
                    let (movement, frame) = remote_anim.update(slot, pos, yaw, step_dt, doing, frames, args.speed_scale.unwrap_or(1.0), args.anim_scale.unwrap_or(1.0));
                    let scale = assets.actors.get(slot % assets.actors.len().max(1)).map_or([1.0; 3], |a| a.scale);
                    let hand = if p.equipped_grenade { p.throw.is_none_or(|t| !t.thrown).then_some(&assets.grenade_draw) } else { Some(&assets.weapon_draw) };
                    let flash = player_flash[slot];
                    // Hidden, the animation still follows them
                    let flinch = flinch_at(chars, p, view_time);
                    let hip_turn = remote_anim.hip_turn(slot, yaw);
                    let body = BodyDraw {
                        side: p.side, movement, frame, feet: pos, yaw_deg: -yaw, scale, face, hand, grenade: p.equipped_grenade, flash, owner: slot,
                        reloading: doing.reloading, firing: doing.firing, crouched: doing.crouched, limp, hip_turn, flinch, peek,
                    };
                    if !visible || self.draw_body(chars, body, bodies.entry(slot).or_default(), out) {
                        continue;
                    }
                } else {
                    // Dead, they are drawn where they fell, with the bodies
                    remote_anim.forget(slot);
                    bodies.remove(&slot);
                    continue;
                }
                if !visible {
                    continue;
                }
                // Without the player animations: a character, idling where they are
                let ghost = Npc {
                    health: combat::Health::new(),
                    pos,
                    floor: pos[1],
                    yaw: -yaw,
                    scale: [1.0; 3],
                    stagger: slot as f32 * 0.73,
                    ..Npc::default()
                };
                let idle = game_time * 30.0 + ghost.stagger * 9.0;
                let (model, arrays, textures, skin) = dress(p.side, &assets.side_dress, &chars.model, &assets.character_arrays, &assets.part_textures, chars.rig.as_ref().map(|r| &r.skin));
                let (posed, turns) = chars.bones(&ghost, view_time, idle).zip(skin).map(|(b, s)| (s.apply(&b), s.turns(&b))).unwrap_or_default();
                let verts: &[[f32; 3]] = if posed.is_empty() { &model.vertices } else { &posed };
                glPushMatrix();
                glTranslatef(ghost.pos[0], ghost.pos[1], ghost.pos[2]);
                glRotatef(ghost.yaw, 0.0, 1.0, 0.0);
                let lit = self.lit_at(ghost.pos, ghost.yaw, ghost.scale);
                draw_character(model, arrays, verts, &turns, textures, face, toggles.textured, self.glow_on(ghost.pos), lit.as_ref());
                glPopMatrix();
            }
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
    }

    // The player's own body, shown in third person
    fn draw_own(&self, pview: &PlayerView, bodies: &mut HashMap<usize, BodyState>, out: &mut FrameOutput) {
        let CharacterPass { args, assets, sim, toggles, timing, me, game_time, player_flash, draw_feet, .. } = *self.pass;
        let (true, true, Some(chars)) = (toggles.first_person, toggles.third_person, &sim.characters) else { return };
        let clock = if timing.fixed_step { game_time as f32 } else { timing.start_time.elapsed().as_secs_f32() };
        let blinking = args.eyes_closed || (toggles.blink_enabled && clock % 4.0 < 0.15);
        let face = (!assets.actors.is_empty()).then(|| assets.face_textures[0]).and_then(|(main, blink)| {
            if blinking { blink.or(main) } else { main }
        });
        let scale = assets.actors.first().map_or([1.0; 3], |a| a.scale);
        let mine = &sim.players[me];
        let hand = if mine.equipped_grenade { mine.throw.is_none_or(|t| !t.thrown).then_some(&assets.grenade_draw) } else { Some(&assets.weapon_draw) };
        let flash = player_flash[me];
        if mine.respawn_at.is_none() {
            let flinch = flinch_at(chars, mine, game_time);
            let body = BodyDraw {
                side: mine.side, movement: pview.movement, frame: pview.player_frame, feet: draw_feet, yaw_deg: -pview.camera.yaw, scale, face, hand,
                grenade: mine.equipped_grenade, flash, owner: me, reloading: mine.reloading(game_time), firing: sim.firing(me), crouched: mine.player.crouched,
                limp: mine.leg_wounds > 0, hip_turn: pview.player_hip_turn, flinch, peek: mine.peek,
            };
            self.draw_body(chars, body, bodies.entry(me).or_default(), out);
        }
    }

    // The bodies of the players who died, falling where they did with the motion the
    // server picked, and lying there for as long as the fall plays or they wait to come back
    fn draw_corpses(&self, corpses: &mut HashMap<usize, (f32, Npc)>) {
        let CharacterPass { assets, sim, toggles, frame: view, me, game_time, .. } = *self.pass;
        let Some(chars) = &sim.characters else { return };
        for (slot, p) in sim.players.iter().enumerate().filter(|(_, p)| p.active) {
            if let (Some(_), Some(hit)) = (p.respawn_at, p.hit) {
                if corpses.get(&slot).is_none_or(|(start, _)| (start - hit.start).abs() > 0.1) {
                    let health = combat::Health { hp: 0.0, state: combat::NpcState::Dead { motion: hit.motion, start: hit.start }, ..Default::default() };
                    let yaw = -sim::dequantize(p.look.0);
                    let scale = assets.actors.get(slot % assets.actors.len().max(1)).map_or([1.0; 3], |a| a.scale);
                    let pos = p.player.pos;
                    corpses.insert(slot, (hit.start, Npc { health, pos, floor: pos[1], yaw, scale, stagger: 0.0, body: None, ..Npc::default() }));
                }
            }
        }
        let view_time = view.map_or(game_time, |v| v.time);
        // The others as the view shows them
        let now_for = |slot: usize| if slot == me { game_time } else { view_time };
        // A body stays while its player is dead, and for a while after they come back
        corpses.retain(|&slot, (start, body)| {
            let combat::NpcState::Dead { motion, .. } = body.health.state else { return false };
            sim.players.get(slot).is_some_and(|p| p.active && p.respawn_at.is_some())
                || now_for(slot) - *start < sim::RESPAWN_SECONDS.max(chars.frames(motion) / 30.0)
        });
        unsafe {
            glDisable(GL_TEXTURE_2D);
            for (&slot, (start, body)) in corpses.iter() {
                // The player's own body only from outside it
                let now = now_for(slot);
                if (slot == me && !(toggles.first_person && toggles.third_person)) || now < *start || !self.shown(body.pos) {
                    continue;
                }
                let face = (!assets.actors.is_empty()).then(|| assets.face_textures[slot % assets.actors.len()].0).flatten();
                let side = sim.players.get(slot).and_then(|p| p.side);
                let (model, arrays, textures, skin) = dress(side, &assets.side_dress, &chars.model, &assets.character_arrays, &assets.part_textures, chars.rig.as_ref().map(|r| &r.skin));
                let (posed, turns) = chars.bones(body, now, 0.0).zip(skin).map(|(b, s)| (s.apply(&b), s.turns(&b))).unwrap_or_default();
                let verts: &[[f32; 3]] = if posed.is_empty() { &model.vertices } else { &posed };
                glPushMatrix();
                glTranslatef(body.pos[0], body.pos[1], body.pos[2]);
                glRotatef(body.yaw, 0.0, 1.0, 0.0);
                glScalef(body.scale[0], body.scale[1], body.scale[2]);
                let lit = self.lit_at(body.pos, body.yaw, body.scale);
                draw_character(model, arrays, verts, &turns, textures, face, toggles.textured, self.glow_on(body.pos), lit.as_ref());
                glPopMatrix();
            }
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
    }

    // Grenades in the air
    fn draw_grenades(&self) {
        let CharacterPass { assets, sim, session, level, toggles, frame: view, .. } = *self.pass;
        let Some(gd) = &assets.grenade_draw else { return };
        let flying: Vec<([f32; 3], f32)> = match view {
            Some(v) => v.grenades.iter().map(|&(_, pos, spin)| (pos, spin)).collect(),
            None => sim.thrown.iter().map(|t| (t.grenade.pos, t.grenade.spin)).collect(),
        };
        unsafe {
            for (pos, spin) in flying {
                glPushMatrix();
                glTranslatef(pos[0], pos[1], pos[2]);
                glRotatef(spin, 1.0, 0.3, 0.0);
                // Model space (forward -Y, up +Z) turned so its length runs the way it spins
                glTranslatef(-gd.center[0], -gd.center[1], -gd.center[2]);
                let light = session.lighting.as_ref().map_or([1.0; 3], |l| room_light(l, level, &sim.visibility, pos, pos).tint);
                draw_weapon(gd, toggles.textured, light);
                glPopMatrix();
            }
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
    }
}

/// Draws the characters, the dead and the grenades in the air, noting into `out` where the
/// muzzle flashes and the casings' guns are
pub fn draw_characters(pass: &CharacterPass, pview: &PlayerView, scene: &mut Scene, out: &mut FrameOutput) {
    let Scene { blast_lights, casing_pending, bodies, remote_anim, corpses, .. } = scene;
    let stage = Stage { pass, blast_lights, casing_pending };
    stage.draw_npcs(out);
    stage.draw_others(remote_anim, bodies, out);
    stage.draw_own(pview, bodies, out);
    stage.draw_corpses(corpses);
    stage.draw_grenades();
}
