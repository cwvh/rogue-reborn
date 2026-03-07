use rogue_reborn::map;
use std::path::Path;
use glfw::{Action, Context, Key, WindowEvent, MouseButton, CursorMode};

#[link(name = "GL")]
extern "C" {
    fn glViewport(x: i32, y: i32, width: i32, height: i32);
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glEnable(cap: u32);
    fn glPolygonMode(face: u32, mode: u32);
    fn glClear(mask: u32);
    fn glMatrixMode(mode: u32);
    fn glLoadIdentity();
    fn glMultMatrixf(m: *const f32);
    fn glBegin(mode: u32);
    fn glEnd();
    fn glVertex3f(x: f32, y: f32, z: f32);
    fn glRotatef(angle: f32, x: f32, y: f32, z: f32);
    fn glTranslatef(x: f32, y: f32, z: f32);
    fn glSwapBuffers();
}

struct Camera {
    position: [f32; 3],
    yaw: f32,
    pitch: f32,
}

impl Camera {
    fn new() -> Self {
        Self {
            position: [0.0, 0.0, 10.0],
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    fn get_forward(&self) -> [f32; 3] {
        let (yaw_rad, pitch_rad) = (self.yaw.to_radians(), self.pitch.to_radians());
        [
            yaw_rad.cos() * pitch_rad.cos(),
            pitch_rad.sin(),
            yaw_rad.sin() * pitch_rad.cos(),
        ]
    }

    fn get_right(&self) -> [f32; 3] {
        let yaw_rad = self.yaw.to_radians();
        [
            yaw_rad.sin(),
            0.0,
            -yaw_rad.cos(),
        ]
    }
}

struct Cube {
    vertices: Vec<[f32; 3]>,
    faces: Vec<[usize; 4]>,
}

fn main() {
    // Construct a simple cube geometry
    let cube = Cube {
        vertices: vec![
            [-1.0, -1.0, -1.0], // 0
            [ 1.0, -1.0, -1.0], // 1
            [ 1.0,  1.0, -1.0], // 2
            [-1.0,  1.0, -1.0], // 3
            [-1.0, -1.0,  1.0], // 4
            [ 1.0, -1.0,  1.0], // 5
            [ 1.0,  1.0,  1.0], // 6
            [-1.0,  1.0,  1.0], // 7
        ],
        faces: vec![
            [0, 1, 2, 3], // back
            [4, 5, 6, 7], // front
            [0, 1, 5, 4], // bottom
            [2, 3, 7, 6], // top
            [1, 2, 6, 5], // right
            [0, 3, 7, 4], // left
        ],
    };

    // Load the map
    let map = map::read(Path::new("data/map/m00/citystreet_large.map")).expect("Failed to load map");

    // Initialize GLFW
    let mut glfw = glfw::init_no_callbacks().unwrap();
    let (mut window, events) = glfw.create_window(800, 600, "Wireframe Viewer", glfw::WindowMode::Windowed)
        .expect("Failed to create GLFW window");
    window.make_current();
    gl::load_with(|s| window.get_proc_address(s) as *const _);

    // OpenGL setup
    unsafe {
        glViewport(0, 0, 800, 600);
        glClearColor(0.1, 0.1, 0.1, 1.0);
        glEnable(0x0B71); // GL_DEPTH_TEST
        glPolygonMode(0x0408, 0x1B01); // GL_FRONT_AND_BACK, GL_LINE
    }

    // Camera setup
    let mut camera = Camera::new();
    let mut last_cursor_pos = None;
    window.set_cursor_mode(CursorMode::Disabled);

    let mut keys = std::collections::HashSet::new();

    while !window.should_close() {
        glfw.poll_events();
        for (_, event) in glfw::flush_messages(&events) {
            match event {
                WindowEvent::Key(key, _, action, _) => {
                    match (key, action) {
                        (Key::Escape, Action::Press) => window.set_should_close(true), // Escape to quit
                        (k, Action::Press) => { keys.insert(k); },
                        (k, Action::Release) => { keys.remove(&k); },
                        _ => {}
                    }
                }
                WindowEvent::CursorPos(xpos, ypos) => {
                    if let Some((last_x, last_y)) = last_cursor_pos {
                        let dx = xpos - last_x;
                        let dy = ypos - last_y;
                        camera.yaw += dx as f32 * 0.1;
                        camera.pitch -= dy as f32 * 0.1;
                        camera.pitch = camera.pitch.clamp(-89.0, 89.0);
                    }
                    last_cursor_pos = Some((xpos, ypos));
                }
                _ => {}
            }
        }

        // WASD movement
        let speed = 0.1;
        let forward = camera.get_forward();
        let right = camera.get_right();
        if keys.contains(&Key::W) {
            camera.position[0] += forward[0] * speed;
            camera.position[1] += forward[1] * speed;
            camera.position[2] += forward[2] * speed;
        }
        if keys.contains(&Key::S) {
            camera.position[0] -= forward[0] * speed;
            camera.position[1] -= forward[1] * speed;
            camera.position[2] -= forward[2] * speed;
        }
        if keys.contains(&Key::A) {
            camera.position[0] -= right[0] * speed;
            camera.position[2] -= right[2] * speed;
        }
        if keys.contains(&Key::D) {
            camera.position[0] += right[0] * speed;
            camera.position[2] += right[2] * speed;
        }

        unsafe {
            glClear(0x00004000 | 0x00000100); // GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT
            glMatrixMode(0x1701); // GL_PROJECTION
            glLoadIdentity();
            glu_perspective(45.0, 800.0/600.0, 0.1, 100.0);

            glMatrixMode(0x1700); // GL_MODELVIEW
            glLoadIdentity();
            apply_camera(&camera);

            // Draw the cube as wireframe
            for face in &cube.faces {
                glBegin(0x0002); // GL_LINE_LOOP
                for &idx in face {
                    let v = cube.vertices[idx];
                    glVertex3f(v[0], v[1], v[2]);
                }
                glEnd();
            }
        }

        window.swap_buffers();
    }
}

// Camera transformation for OpenGL 1.x
fn apply_camera(camera: &Camera) {
    unsafe {
        // Apply pitch (look up/down), then yaw (look left/right)
        glRotatef(-camera.pitch, 1.0, 0.0, 0.0);
        glRotatef(-camera.yaw, 0.0, 1.0, 0.0);
        glTranslatef(-camera.position[0], -camera.position[1], -camera.position[2]);
    }
}

// Minimal gluPerspective and gluLookAt replacements for OpenGL 1.x
fn glu_perspective(fov_y: f32, aspect: f32, z_near: f32, z_far: f32) {
    let f = 1.0 / (fov_y.to_radians() / 2.0).tan();
    let nf = 1.0 / (z_near - z_far);
    unsafe {
        glLoadIdentity();
        let mut mat = [0.0f32; 16];
        mat[0] = f / aspect;
        mat[5] = f;
        mat[10] = (z_far + z_near) * nf;
        mat[11] = -1.0;
        mat[14] = (2.0 * z_far * z_near) * nf;
        glMultMatrixf(mat.as_ptr());
    }
}

fn glu_look_at(
    eye_x: f32, eye_y: f32, eye_z: f32,
    center_x: f32, center_y: f32, center_z: f32,
    up_x: f32, up_y: f32, up_z: f32
) {
    // Compute forward vector
    let fx = center_x - eye_x;
    let fy = center_y - eye_y;
    let fz = center_z - eye_z;
    let flen = (fx * fx + fy * fy + fz * fz).sqrt();
    let fx = fx / flen;
    let fy = fy / flen;
    let fz = fz / flen;

    // Compute up vector
    let up_len = (up_x * up_x + up_y * up_y + up_z * up_z).sqrt();
    let upx = up_x / up_len;
    let upy = up_y / up_len;
    let upz = up_z / up_len;

    // Compute side vector = forward x up
    let sx = fy * upz - fz * upy;
    let sy = fz * upx - fx * upz;
    let sz = fx * upy - fy * upx;
    let slen = (sx * sx + sy * sy + sz * sz).sqrt();
    let sx = sx / slen;
    let sy = sy / slen;
    let sz = sz / slen;

    // Recompute up as: side x forward
    let ux = sy * fz - sz * fy;
    let uy = sz * fx - sx * fz;
    let uz = sx * fy - sy * fx;

    let mat = [
        sx,  ux,  -fx,  0.0,
        sy,  uy,  -fy,  0.0,
        sz,  uz,  -fz,  0.0,
        0.0, 0.0,  0.0, 1.0,
    ];

    unsafe {
        glMultMatrixf(mat.as_ptr());
        glTranslatef(-eye_x, -eye_y, -eye_z);
    }
}