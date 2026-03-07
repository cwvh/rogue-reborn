use rogue_reborn::map;
use std::path::Path;
use glfw::{Action, Context, Key, WindowEvent, CursorMode};

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
            -yaw_rad.sin() * pitch_rad.cos(),
            pitch_rad.sin(),
            -yaw_rad.cos() * pitch_rad.cos(),
        ]
    }

    fn get_right(&self) -> [f32; 3] {
        let yaw_rad = self.yaw.to_radians();
        [
            yaw_rad.cos(),
            0.0,
            -yaw_rad.sin(),
        ]
    }
}

struct Mesh {
    vertices: Vec<[f32; 3]>,
    indices: Vec<[u32; 3]>,
    center: [f32; 3],
    radius: f32,
}

fn main() {
    // Load the map
    let level = map::read(Path::new("data/map/m00/citystreet_large.map"))
    // let level = map::read(Path::new("data/map/rm02/rm02.map"))
        .expect("Failed to load map");

    // Build a flattened mesh from the MAP geometry
    let mesh = build_mesh(&level);

    // Initialize GLFW
    let mut glfw = glfw::init_no_callbacks().unwrap();
    let (mut window, events) = glfw
        .create_window(800, 600, "Wireframe Viewer", glfw::WindowMode::Windowed)
        .expect("Failed to create GLFW window");
    window.make_current();
    window.set_key_polling(true);
    window.set_cursor_pos_polling(true);
    window.set_framebuffer_size_polling(true);
    window.set_scroll_polling(true);
    gl::load_with(|s| window.get_proc_address(s) as *const _);

    // OpenGL setup
    unsafe {
        glViewport(0, 0, 1024, 768);
        glClearColor(0.1, 0.1, 0.1, 1.0);
        glEnable(0x0B71); // GL_DEPTH_TEST
        glPolygonMode(0x0408, 0x1B01); // GL_FRONT_AND_BACK, GL_LINE
    }

    // Camera setup: start back from the map center looking in -Z
    let mut camera = Camera::new();
    camera.position = [
        mesh.center[0],
        mesh.center[1],
        mesh.center[2] + mesh.radius * 2.0,
    ];
    let mut last_cursor_pos: Option<(f64, f64)> = None;
    window.set_cursor_mode(CursorMode::Disabled);

    let mut keys = std::collections::HashSet::new();

    // Base movement speed scales with map size
    let base_speed = (mesh.radius / 200.0).max(0.01);

    while !window.should_close() {
        glfw.poll_events();
        for (_, event) in glfw::flush_messages(&events) {
            match event {
                WindowEvent::Key(key, _, action, _) => {
                    match (key, action) {
                        (Key::Escape, Action::Press) | (Key::Q, Action::Press) => window.set_should_close(true), // Escape or 'q' to quit
                        (k, Action::Press) => {
                            keys.insert(k);
                        }
                        (k, Action::Release) => {
                            keys.remove(&k);
                        }
                        _ => {}
                    }
                }
                WindowEvent::CursorPos(xpos, ypos) => {
                    if let Some((last_x, last_y)) = last_cursor_pos {
                        let dx = xpos - last_x;
                        let dy = ypos - last_y;
                        camera.yaw -= dx as f32 * 0.1;
                        camera.pitch -= dy as f32 * 0.1;
                        camera.pitch = camera.pitch.clamp(-89.0, 89.0);
                    }
                    last_cursor_pos = Some((xpos, ypos));
                }
                WindowEvent::FramebufferSize(width, height) => {
                    unsafe { glViewport(0, 0, width, height); }
                }
                WindowEvent::Scroll(_x, y) => {
                    let scroll = y as f32;
                    if scroll != 0.0 {
                        let dir = camera.get_forward();
                        let zoom_speed = base_speed * 20.0;
                        camera.position[0] += dir[0] * zoom_speed * scroll;
                        camera.position[1] += dir[1] * zoom_speed * scroll;
                        camera.position[2] += dir[2] * zoom_speed * scroll;
                    }
                }
                _ => {}
            }
        }

        // WASD movement (hold LeftShift to move faster)
        let mut speed = base_speed;
        if keys.contains(&Key::LeftShift) {
            speed *= 5.0;
        }
        let right = camera.get_right();
        let yaw_rad = camera.yaw.to_radians();
        let fwd_flat = [-yaw_rad.sin(), 0.0, -yaw_rad.cos()];
        if keys.contains(&Key::W) {
            camera.position[0] += fwd_flat[0] * speed;
            camera.position[2] += fwd_flat[2] * speed;
        }
        if keys.contains(&Key::S) {
            camera.position[0] -= fwd_flat[0] * speed;
            camera.position[2] -= fwd_flat[2] * speed;
        }
        if keys.contains(&Key::A) {
            camera.position[0] -= right[0] * speed;
            camera.position[2] -= right[2] * speed;
        }
        if keys.contains(&Key::D) {
            camera.position[0] += right[0] * speed;
            camera.position[2] += right[2] * speed;
        }

        // Compute dynamic aspect and clip planes
        let (w, h) = window.get_size();
        let aspect = (w as f32 / h.max(1) as f32).max(0.1);
        let z_near = (mesh.radius / 1000.0).max(0.05);
        let z_far = (mesh.radius * 10.0 + 100.0).max(z_near + 1.0);

        unsafe {
            glClear(0x00004000 | 0x00000100); // GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT
            glMatrixMode(0x1701); // GL_PROJECTION
            glLoadIdentity();
            glu_perspective(60.0, aspect, z_near, z_far);

            glMatrixMode(0x1700); // GL_MODELVIEW
            glLoadIdentity();
            apply_camera(&camera);

            // Draw all triangles in wireframe
            glBegin(0x0004); // GL_TRIANGLES
            for tri in &mesh.indices {
                let a = mesh.vertices[tri[0] as usize];
                let b = mesh.vertices[tri[1] as usize];
                let c = mesh.vertices[tri[2] as usize];
                glVertex3f(a[0], a[1], a[2]);
                glVertex3f(b[0], b[1], b[2]);
                glVertex3f(c[0], c[1], c[2]);
            }
            glEnd();
        }

        window.swap_buffers();
    }
}

fn build_mesh(level: &map::Map) -> Mesh {
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::new();

    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for object in &level.geometries.objects {
        let base = vertices.len() as u32;

        // Append object vertices
        for v in &object.vertices {
            let p = [v.x, v.y, v.z];
            for i in 0..3 {
                if p[i] < min[i] {
                    min[i] = p[i];
                }
                if p[i] > max[i] {
                    max[i] = p[i];
                }
            }
            vertices.push(p);
        }

        // Append triangle indices from all object data groups
        for od in &object.object_datas {
            for &(i0, i1, i2) in &od.faces.face_indices {
                indices.push([
                    base + i0 as u32,
                    base + i1 as u32,
                    base + i2 as u32,
                ]);
            }
        }
    }

    // Fallback if empty
    if vertices.is_empty() {
        return Mesh {
            vertices,
            indices,
            center: [0.0, 0.0, 0.0],
            radius: 10.0,
        };
    }

    let center = [
        (min[0] + max[0]) * 0.5,
        (min[1] + max[1]) * 0.5,
        (min[2] + max[2]) * 0.5,
    ];
    let dx = max[0] - center[0];
    let dy = max[1] - center[1];
    let dz = max[2] - center[2];
    let radius = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0);

    Mesh {
        vertices,
        indices,
        center,
        radius,
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

// Minimal gluPerspective replacement for OpenGL 1.x
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
