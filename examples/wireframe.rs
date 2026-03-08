use rogue_reborn::map;
use rogue_reborn::rsb;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::time::{Instant, Duration};
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
    fn glScalef(x: f32, y: f32, z: f32);
    fn glTexCoord2f(s: f32, t: f32);
    fn glTexParameteri(target: u32, pname: u32, param: i32);
    fn glGenTextures(n: i32, textures: *mut u32);
    fn glBindTexture(target: u32, texture: u32);
    fn glTexImage2D(
        target: u32,
        level: i32,
        internalformat: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        type_: u32,
        pixels: *const std::ffi::c_void
    );
    fn glDisable(cap: u32);
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
    let map_path = Path::new("data/map/m00/citystreet_large.map");
    let map_num = extract_map_num(map_path).unwrap_or_else(|| String::from("m00"));
    let level = map::read(map_path)
        .expect("Failed to load map");

    // Build a flattened mesh from the MAP geometry
    let mesh = build_mesh(&level);

    // Initialize GLFW
    let mut glfw = glfw::init_no_callbacks().unwrap();
    let (mut window, events) = glfw
        .create_window(1024, 768, "Wireframe Viewer", glfw::WindowMode::Windowed)
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

    // Camera setup: start at mission-provided location
    let mut camera = Camera::new();
    camera.position = [33924.6_f32, 33307.0_f32, 30028.3_f32];   // CSL
    let mut last_cursor_pos: Option<(f64, f64)> = None;
    window.set_cursor_mode(CursorMode::Disabled);

    let mut keys = std::collections::HashSet::new();
    let mut textures: HashMap<String, u32> = HashMap::new();
    let mut textured = false;
    let mut first_person = false;
    let eye_height: f32 = 25.0;

    // Base movement speed scales with map size
    // let base_speed = (mesh.radius / 200.0).max(0.01);
    let base_speed = 5.0;

    while !window.should_close() {
        glfw.poll_events();

        for (_, event) in glfw::flush_messages(&events) {
            match event {
                WindowEvent::Key(key, _, action, _) => {
                    match (key, action) {
                        (Key::Escape, Action::Press) | (Key::Q, Action::Press) => window.set_should_close(true), // Escape or 'q' to quit
                        (Key::T, Action::Press) => {
                            textured = !textured;
                        }
                        (Key::F, Action::Press) => {
                            first_person = !first_person;
                            if first_person {
                                if let Some(gy) = find_ground_y(&level, camera.position[0], camera.position[2], camera.position[1]) {
                                    camera.position[1] = gy + eye_height;
                                }
                            }
                        }
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
                    if !first_person {
                        let scroll = y as f32;
                        if scroll != 0.0 {
                            let dir = camera.get_forward();
                            let zoom_speed = base_speed * 5.0;
                            camera.position[0] += dir[0] * zoom_speed * scroll;
                            camera.position[1] += dir[1] * zoom_speed * scroll;
                            camera.position[2] += dir[2] * zoom_speed * scroll;
                        }
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
        let prev_pos = camera.position;
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

        // Resolve wall collisions in first-person mode
        if first_person {
            let foot_y = camera.position[1] - eye_height;
            // TODO renable wall collision once I get it working
            // let adjusted = resolve_walls(&level, prev_pos, camera.position, foot_y, 30.0);
            // camera.position[0] = adjusted[0];
            // camera.position[2] = adjusted[2];
        }

        // Snap to ground in first-person mode
        if first_person {
            if let Some(ground_y) = find_ground_y(&level, camera.position[0], camera.position[2], camera.position[1]) {
                camera.position[1] = ground_y + eye_height;
            }
        }

        // Compute dynamic aspect and clip planes
        let (w, h) = window.get_size();
        let aspect = (w as f32 / h.max(1) as f32).max(0.1);
        let z_near = (mesh.radius / 1000.0).max(0.05);
        let z_far = (mesh.radius * 10.0 + 100.0).max(z_near + 1.0);

        let mut render_time = Instant::now();
        unsafe {
            glClear(0x00004000 | 0x00000100); // GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT
            glMatrixMode(0x1701); // GL_PROJECTION
            glLoadIdentity();
            glu_perspective(60.0, aspect, z_near, z_far);

            glMatrixMode(0x1700); // GL_MODELVIEW
            glLoadIdentity();
            apply_camera(&camera);
            glScalef(1.0, 1.0, -1.0);

            // Draw either textured geometry or wireframe triangles
            if textured {
                // Textured mode
                glEnable(0x0DE1); // GL_TEXTURE_2D
                glPolygonMode(0x0408, 0x1B02); // GL_FRONT_AND_BACK, GL_FILL

                // Iterate map geometry by material groups
                for object in &level.geometries.objects {
                    for od in &object.object_datas {
                        let mat_index = od.mn as usize;
                        if let Some(tex_id) = ensure_texture(&level.materials, mat_index, &map_num, &mut textures) {
                            // TODO put all faces with same tex-id together so I can bind once
                            glBindTexture(0x0DE1, tex_id); // GL_TEXTURE_2D

                            // Faces and UVs are parallel arrays
                            let face_count = od.faces.face_indices.len().min(od.faces.texture_indices.len());
                            if face_count == 0 {
                                continue;
                            }

                            glBegin(0x0004); // GL_TRIANGLES
                            for i in 0..face_count {
                                let (i0, i1, i2) = od.faces.face_indices[i];
                                let (t0, t1, t2) = od.faces.texture_indices[i];

                                let p0 = &object.vertices[i0 as usize];
                                let p1 = &object.vertices[i1 as usize];
                                let p2 = &object.vertices[i2 as usize];

                                let uv0 = &od.texture_vertices.uv_coords[t0 as usize];
                                let uv1 = &od.texture_vertices.uv_coords[t1 as usize];
                                let uv2 = &od.texture_vertices.uv_coords[t2 as usize];

                                // Flip V to match OpenGL texture origin
                                glTexCoord2f(uv0.u, 1.0 - uv0.v);
                                glVertex3f(p0.x, p0.y, p0.z);

                                glTexCoord2f(uv1.u, 1.0 - uv1.v);
                                glVertex3f(p1.x, p1.y, p1.z);

                                glTexCoord2f(uv2.u, 1.0 - uv2.v);
                                glVertex3f(p2.x, p2.y, p2.z);
                            }
                            glEnd();
                        }
                    }
                }
            } else {
                // Wireframe mode
                glDisable(0x0DE1); // GL_TEXTURE_2D
                glPolygonMode(0x0408, 0x1B01); // GL_FRONT_AND_BACK, GL_LINE

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
        }
        println!("render_time:{:?}", render_time.elapsed());

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

// Walkable-surface detection via Object.collisions.
// Approximates ground height at (x,z) from up-facing collision planes.
fn find_ground_y(level: &map::Map, x: f32, z: f32, current_y: f32) -> Option<f32> {
    let max_slope_deg = 50.0f32;
    let ny_thresh = max_slope_deg.to_radians().cos();

    let mut best_below: Option<f32> = None;
    let mut best_above: Option<f32> = None;

    for object in &level.geometries.objects {
        for face in &object.collisions.faces {
            let mut nx = face.x;
            let mut ny = face.y;
            let mut nz = face.z;
            let mut d = face.distance_origin_to_face;

            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            if len <= 1e-6 {
                continue;
            }
            nx /= len;
            ny /= len;
            nz /= len;
            d /= len;

            if ny < ny_thresh || ny.abs() < 1e-6 {
                continue;
            }

            // Plane in standard form: n dot p + d = 0
            // Solve for y given x,z: y = (-d - nx*x - nz*z) / ny
            let y = (-d - nx * x - nz * z) / ny;

            if y <= current_y {
                if best_below.map_or(true, |v| y > v) {
                    best_below = Some(y);
                }
            } else {
                if best_above.map_or(true, |v| y < v) {
                    best_above = Some(y);
                }
            }
        }
    }

    best_below.or(best_above)
}

// Collision resolution against wall planes from Object.collisions.
fn resolve_walls(level: &map::Map, old_pos: [f32; 3], mut new_pos: [f32; 3], y_ref: f32, radius: f32) -> [f32; 3] {
    // Treat walls as near-vertical planes and push the player back if crossing them.
    let ny_wall_max = 0.3f32;

    for _ in 0..2 {
        let mut corrected = false;

        for object in &level.geometries.objects {
            for face in &object.collisions.faces {
                let mut nx = face.x;
                let mut ny = face.y;
                let mut nz = face.z;
                let mut d = face.distance_origin_to_face;

                let len = (nx * nx + ny * ny + nz * nz).sqrt();
                if len <= 1e-6 {
                    continue;
                }
                nx /= len;
                ny /= len;
                nz /= len;
                d /= len;

                // Only consider near-vertical planes as walls
                if ny.abs() > ny_wall_max {
                    continue;
                }

                // Signed distances to the plane (n·p + d)
                let s_old = nx * old_pos[0] + ny * y_ref + nz * old_pos[2] + d;
                let s_new = nx * new_pos[0] + ny * y_ref + nz * new_pos[2] + d;

                // If we moved from the back side (<= -radius) to front side (> -radius), resolve penetration
                if s_old <= -radius && s_new > -radius {
                    let penetration = s_new + radius;
                    // Push back along the horizontal component of the normal
                    let hx = nx;
                    let hz = nz;
                    let hlen = (hx * hx + hz * hz).sqrt();
                    if hlen > 1e-5 && penetration > 0.0 {
                        new_pos[0] -= (hx / hlen) * penetration;
                        new_pos[2] -= (hz / hlen) * penetration;
                        corrected = true;
                    }
                }
            }
        }

        if !corrected {
            break;
        }
    }

    new_pos
}

// Texture utilities

fn ensure_texture(
    materials: &map::Materials,
    mat_index: usize,
    map_num: &str,
    cache: &mut HashMap<String, u32>,
) -> Option<u32> {
    if mat_index >= materials.materials.len() {
        return None;
    }
    let filename = materials.materials[mat_index].filename.clone();
    if let Some(id) = cache.get(&filename) {
        return Some(*id);
    }

    let resolved = resolve_texture_path(map_num, &filename)?;
    let rsb_tex = match rsb::read(&resolved) {
        Ok(t) => t,
        Err(_) => return None,
    };

    let tex_id = upload_rsb_to_gl(&rsb_tex)?;
    cache.insert(filename, tex_id);
    Some(tex_id)
}

// Find an on-disk RSB path by searching map-local and shared texture dirs,
// converting MAP's .bmp references to .RSB/.rsb.
fn resolve_texture_path(map_num: &str, material_filename: &str) -> Option<PathBuf> {
    // Preserve any subdirectories from the material filename (e.g., "faces/foo.bmp")
    let rel = Path::new(material_filename);

    // Texture name without extension (MAP refers to .bmp)
    let stem = rel.file_stem()?.to_string_lossy().into_owned();

    // Rebuild the relative path keeping parent directories, swapping extension to RSB
    let mut build_rel = |ext: &str| -> PathBuf {
        if let Some(parent) = rel.parent() {
            parent.join(format!("{stem}.{ext}"))
        } else {
            PathBuf::from(format!("{stem}.{ext}"))
        }
    };

    let candidates = [
        build_rel("RSB"),
        build_rel("rsb"),
    ];

    for cand in candidates.iter() {
        // 1) data/<map_num>/...
        let p1 = Path::new("data").join("map").join(map_num).join(cand);
        if p1.exists() {
            return Some(p1);
        }
        // 2) data/texture/...
        let p2 = Path::new("data").join("texture").join(cand);
        if p2.exists() {
            return Some(p2);
        }
    }

    None
}

// Extract the map number (e.g., "m00") from a map path like data/map/m00/citystreet_large.map
fn extract_map_num(path: &Path) -> Option<String> {
    let mut comps = path.components().map(|c| c.as_os_str());
    while let Some(c) = comps.next() {
        if c == "map" {
            if let Some(next) = comps.next() {
                return Some(next.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn upload_rsb_to_gl(tex: &rsb::Rsb) -> Option<u32> {
    let rgba = rsb_to_rgba8(tex);
    if rgba.is_empty() {
        return None;
    }

    let mut id: u32 = 0;
    unsafe {
        glGenTextures(1, &mut id as *mut u32);
        glBindTexture(0x0DE1, id); // GL_TEXTURE_2D

        // Simple filters; can be refined per material address mode
        glTexParameteri(0x0DE1, 0x2801, 0x2601); // GL_TEXTURE_MIN_FILTER, GL_LINEAR
        glTexParameteri(0x0DE1, 0x2800, 0x2601); // GL_TEXTURE_MAG_FILTER, GL_LINEAR

        glTexImage2D(
            0x0DE1,              // GL_TEXTURE_2D
            0,                   // level
            0x1908,              // GL_RGBA
            tex.width as i32,
            tex.height as i32,
            0,                   // border
            0x1908,              // GL_RGBA
            0x1401,              // GL_UNSIGNED_BYTE
            rgba.as_ptr() as *const std::ffi::c_void
        );
    }
    Some(id)
}

fn rsb_to_rgba8(tex: &rsb::Rsb) -> Vec<u8> {
    let size = tex.size();
    let mut out = Vec::with_capacity(size * 4);

    if tex.palette_colors.is_some() {
        // Expect palette indices in tex.pixels
        if let Some(colors) = &tex.palette_colors {
            for px in &tex.pixels {
                let idx = match px {
                    rsb::Pixel::PaletteColorIndex(i) => *i as usize,
                    _ => 0,
                };
                let c = colors.get(idx).cloned().unwrap_or(rsb::PaletteColor { r: 0, g: 0, b: 0, a: 255 });
                out.push(c.r);
                out.push(c.g);
                out.push(c.b);
                out.push(c.a);
            }
        }
        return out;
    }

    // Non-paletted: expand using bitmask
    let bm = &tex.bitmask;
    let rbits = bm.r.max(1);
    let gbits = bm.g.max(1);
    let bbits = bm.b.max(1);
    let abits = bm.a; // alpha may be 0

    for px in &tex.pixels {
        let r = px.r(bm).unwrap_or(0);
        let g = px.g(bm).unwrap_or(0);
        let b = px.b(bm).unwrap_or(0);
        let a = px.a(bm).unwrap_or_else(|| if abits == 0 { 255 } else { 0 });

        out.push(scale_to_u8(r, rbits));
        out.push(scale_to_u8(g, gbits));
        out.push(scale_to_u8(b, bbits));
        // If alpha channel not present, a is already 255
        out.push(if abits == 0 { a as u8 } else { scale_to_u8(a, abits) });
    }

    out
}

fn scale_to_u8(v: u32, bits: u32) -> u8 {
    if bits == 0 {
        return 255;
    }
    let max = (1u32 << bits) - 1;
    (((v * 255) + (max / 2)) / max) as u8
}
