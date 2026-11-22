use rogue_reborn::concussion::Concussion;
use rogue_reborn::net::watch::Watch;
use rogue_reborn::sim::movement::Movement;
use crate::camera::Camera;

// The player's own point of view: the camera and the body it follows, what the keys and the
// mouse hold, and what the player's senses go through (a blast, a death, watching others)
pub struct PlayerView {
    pub camera: Camera,
    // The view's height over the feet, which eases down and up as the player crouches and stands
    pub eye_now: f32,
    // Left Ctrl switches between standing and crouched
    pub crouch_wanted: bool,
    pub movement: Movement,
    pub player_frame: f32,
    pub player_hip_turn: f32,
    // Where the player stood a tick ago, to draw between ticks
    pub prev_feet: [f32; 3],
    pub was_free: bool,
    pub trigger: bool,
    // Right mouse button held: walk instead of the default run
    pub walk_held: bool,
    // A blast leaves the player shaken (the view), dazzled (a white veil) and deafened (muffled sound)
    pub shake: f32,
    pub veil: f32,
    pub deaf: f32,
    // ...and what they hear is dull: how much, and seconds since it began (see `sound::stunned_after`)
    pub stun: (f32, f32),
    // ...and flinches, and sees a grey tunnel for a moment
    pub concussion: Concussion,
    pub hit_marker_until: f32,
    // How this player died: by whom, and where they were looking before
    pub killed_by: Option<Option<usize>>,
    pub alive_look: Option<(f32, f32)>,
    // Dead in a round: whom the player watches (see net::watch), the keys pressed for it, whom
    // the camera is on and the room they were in last, and the eye height eased between the
    // watched one's stances
    pub watch: Watch,
    pub watch_next: bool,
    pub watch_previous: bool,
    pub watch_mode: bool,
    pub camera_on: Option<usize>,
    pub camera_room: Option<usize>,
    pub watch_eye: Option<(Option<usize>, f32)>,
}

impl PlayerView {
    pub fn new(camera: Camera, eye_height: f32, feet: [f32; 3]) -> PlayerView {
        PlayerView {
            camera,
            eye_now: eye_height,
            crouch_wanted: false,
            movement: Movement::Idle,
            player_frame: 0.0,
            player_hip_turn: 0.0,
            prev_feet: feet,
            was_free: false,
            trigger: false,
            walk_held: false,
            shake: 0.0,
            veil: 0.0,
            deaf: 0.0,
            stun: (0.0, f32::MAX),
            concussion: Concussion::default(),
            hit_marker_until: 0.0,
            killed_by: None,
            alive_look: None,
            watch: Watch::default(),
            watch_next: false,
            watch_previous: false,
            watch_mode: false,
            camera_on: None,
            camera_room: None,
            watch_eye: None,
        }
    }
}
