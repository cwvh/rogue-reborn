//! The view: where it is and which way it looks.

use rogue_reborn::sim::movement;
use rogue_reborn::sound;
use crate::opengl::*;

pub struct Camera {
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            position: [0.0, 0.0, 10.0],
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    pub fn get_forward(&self) -> [f32; 3] {
        movement::forward(self.yaw, self.pitch)
    }

    pub fn get_right(&self) -> [f32; 3] {
        movement::right(self.yaw)
    }
}

// The listener for positional sounds, in map space (camera space has Z flipped)
pub fn listener_of(camera: &Camera) -> sound::Listener {
    let r = camera.get_right();
    sound::Listener {
        pos: [camera.position[0], camera.position[1], -camera.position[2]],
        right: [r[0], r[1], -r[2]],
    }
}

// Camera transformation for OpenGL 1.x
pub fn apply_camera(camera: &Camera) {
    unsafe {
        // Apply pitch (look up/down), then yaw (look left/right)
        glRotatef(-camera.pitch, 1.0, 0.0, 0.0);
        glRotatef(-camera.yaw, 0.0, 1.0, 0.0);
        glTranslatef(-camera.position[0], -camera.position[1], -camera.position[2]);
    }
}

 // Minimal gluPerspective replacement for OpenGL 1.x
