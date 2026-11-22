//! Drawing the weapon held in front of the camera.

use rogue_reborn::{casings, grenade, map, sim};
use crate::args::Args;
use crate::half_view_tan;
use crate::opengl::*;
use crate::render::assets::Assets;
use crate::render::characters::{FrameOutput, standing_light};
use crate::render::draw::{FlashSpot, draw_flash_here, draw_weapon};
use crate::render::effects::MOST_CASINGS_A_FRAME;
use crate::scene::Scene;
use crate::session::Session;
use crate::state::Toggles;

// What the view model's drawing reads
pub struct ViewModelPass<'a> {
    pub args: &'a Args,
    pub assets: &'a Assets,
    pub level: &'a map::Map,
    pub sim: &'a sim::Sim,
    pub session: &'a Session,
    pub toggles: &'a Toggles,
    pub me: usize,
    pub game_time: f32,
    pub aspect: f32,
    pub player_flash: &'a [bool],
    pub player_ejector: Option<&'a casings::Ejector>,
    pub view_to_map: Option<casings::Mat4>,
    // The lobby is up over the game
    pub gate_showing: bool,
}

/// The held weapon in front of the camera, over the scene, and the casings it ejects
pub fn draw_view_model(pass: ViewModelPass, scene: &mut Scene, out: &mut FrameOutput) {
    let ViewModelPass { args, assets, level, sim, session, toggles, me, game_time, aspect, player_flash, player_ejector, view_to_map, gate_showing } = pass;
    // The held weapon in front of the camera, over the scene. A grenade is small, so it is held
    // further out in proportion, drawn back by the wind-up, and gone once it has been thrown.
    let held = if sim.players[me].equipped_grenade { assets.grenade_draw.as_ref().filter(|_| sim.players[me].throw.is_none_or(|t| !t.thrown)) } else { assets.weapon_draw.as_ref() };
    // Without the view model (the default) the gun is not drawn.
    // Its place still says where the casings leave, and only a firing gun needs it.
    let casing_rounds = scene.casing_pending.get(&me).copied().unwrap_or(0);
    let gun_shows = args.viewmodel;
    if let (true, true, false, Some(wd)) = ((gun_shows || (casing_rounds > 0 && !sim.players[me].equipped_grenade)) && sim.players[me].respawn_at.is_none(), toggles.first_person, toggles.third_person, held) {
        let l = wd.length;
        let [ox, oy, oz] = if sim.players[me].equipped_grenade {
            let held_for = sim.players[me].charge_from.map_or(0.0, |from| ((game_time - from) / grenade::CHARGE_SECONDS).clamp(0.0, 1.0));
            let swing = sim.players[me].throw.map_or(0.0, |t| t.charge.max(((game_time - t.start) / sim::THROW_SECONDS / sim::THROW_RELEASE).clamp(0.0, 1.0)));
            let wind = held_for.max(swing);
            [1.7 + 0.9 * wind, -1.2 - 0.6 * wind, 8.0 - 1.8 * wind]
        } else {
            args.viewmodel_offset.unwrap_or([0.45, -0.45, 1.5])
        };
        // Recoil pushes the gun back at the player; a reload dips it out of view
        let kick = ((sim.players[me].flash_until - game_time) / 0.05).clamp(0.0, 1.0) * 0.04 * l;
        let dip = sim.players[me].reload_until.map_or(0.0, |t| {
            let p = (1.0 - (t - game_time) / sim::RELOAD_SECONDS).clamp(0.0, 1.0);
            (p * std::f32::consts::PI).sin() * 0.8 * l
        });
        unsafe {
            if gun_shows {
                // Its own depth range, so the gun never sinks into a wall
                glClear(GL_DEPTH_BUFFER_BIT);
                glMatrixMode(GL_PROJECTION);
                glLoadIdentity();
                glu_perspective(45.0, aspect, l * 0.05, l * 20.0); // narrower than the world, as in most shooters
            } else {
                glMatrixMode(GL_MODELVIEW);
                glPushMatrix();
            }
            glMatrixMode(GL_MODELVIEW);
            glLoadIdentity();
            glTranslatef(ox * l, oy * l - dip, -oz * l + kick);
            // Angle the barrel slightly in toward the middle of the view
            glRotatef(20.0, 0.0, 1.0, 0.0);
            // Model space (forward -Y, up +Z) to view space (forward -Z, up +Y)
            glRotatef(180.0, 0.0, 1.0, 0.0);
            glRotatef(-90.0, 1.0, 0.0, 0.0);
            glTranslatef(-wd.center[0], -wd.center[1], -wd.center[2]);
            if gun_shows {
                // Lit by the room the player stands in
                let light = session.lighting.as_ref().map_or([1.0; 3], |l| {
                    let lit = standing_light(l, level, &sim.visibility, sim.players[me].player.pos);
                    if player_flash[me] && wd.flash_scale > 0.0 { lit.flash_tint() } else { lit.tint }
                });
                draw_weapon(wd, toggles.textured, light);
                if let (true, Some(flash)) = (player_flash[me], assets.flash_draw.as_ref().filter(|_| toggles.textured)) {
                    draw_flash_here(flash, &FlashSpot::here(wd, out.flash_roll()));
                }
            }
            // The casings leave the gun as it is placed here.
            // Its view is narrower than the world's, so they start further from the middle.
            if let (true, false, Some(e), Some(to_map)) = (casing_rounds > 0, gate_showing, player_ejector.filter(|_| !sim.players[me].equipped_grenade), &view_to_map) {
                let mut gun = [0f32; 16];
                glGetFloatv(GL_MODELVIEW_MATRIX, gun.as_mut_ptr());
                let widen = half_view_tan() / 22.5f32.to_radians().tan();
                for _ in 0..casing_rounds.min(MOST_CASINGS_A_FRAME) {
                    scene.casings.eject(&gun, to_map, e, me, widen);
                }
            }
            if !gun_shows {
                glPopMatrix();
            }
            glColor4f(1.0, 1.0, 1.0, 1.0);
        }
    }
    // Rounds fired by guns not drawn this frame leave no casings
    scene.casing_pending.clear();
    out.casing_spots.clear();
}
