//! The OpenGL calls the viewer uses (fixed-function, declared directly) and helpers for
//! matrices, textures and reading the screen back.

use rogue_reborn::rsb;

// Primitive modes for glBegin and glDrawArrays
pub const GL_POINTS: u32 = 0x0000;
pub const GL_LINES: u32 = 0x0001;
pub const GL_LINE_LOOP: u32 = 0x0002;
pub const GL_TRIANGLES: u32 = 0x0004;
pub const GL_TRIANGLE_STRIP: u32 = 0x0005;
pub const GL_TRIANGLE_FAN: u32 = 0x0006;
pub const GL_QUADS: u32 = 0x0007;

// Comparison functions
pub const GL_LESS: u32 = 0x0201;
pub const GL_EQUAL: u32 = 0x0202;
pub const GL_LEQUAL: u32 = 0x0203;
pub const GL_GREATER: u32 = 0x0204;
pub const GL_ALWAYS: u32 = 0x0207;

// Stencil operations
pub const GL_ZERO: u32 = 0x0000;
pub const GL_KEEP: u32 = 0x1E00;
pub const GL_REPLACE: u32 = 0x1E01;
pub const GL_INCR: u32 = 0x1E02;
pub const GL_DECR: u32 = 0x1E03;

// Blend factors
pub const GL_ONE: u32 = 0x0001;
pub const GL_SRC_ALPHA: u32 = 0x0302;
pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 0x0303;

// Clear bits
pub const GL_COLOR_BUFFER_BIT: u32 = 0x4000;
pub const GL_DEPTH_BUFFER_BIT: u32 = 0x0100;
pub const GL_STENCIL_BUFFER_BIT: u32 = 0x0400;

// Capabilities for glEnable and glDisable
pub const GL_ALPHA_TEST: u32 = 0x0BC0;
pub const GL_BLEND: u32 = 0x0BE2;
pub const GL_CULL_FACE: u32 = 0x0B44;
pub const GL_DEPTH_TEST: u32 = 0x0B71;
pub const GL_POLYGON_OFFSET_FILL: u32 = 0x8037;
pub const GL_SCISSOR_TEST: u32 = 0x0C11;
pub const GL_STENCIL_TEST: u32 = 0x0B90;
pub const GL_CLIP_PLANE0: u32 = 0x3000;

// Faces, winding and polygon modes
pub const GL_BACK: u32 = 0x0405;
pub const GL_FRONT_AND_BACK: u32 = 0x0408;
pub const GL_CW: u32 = 0x0900;
pub const GL_LINE: u32 = 0x1B01;
pub const GL_FILL: u32 = 0x1B02;

// Matrix modes, matrix queries and viewport
pub const GL_MODELVIEW: u32 = 0x1700;
pub const GL_PROJECTION: u32 = 0x1701;
pub const GL_MODELVIEW_MATRIX: u32 = 0x0BA6;
pub const GL_PROJECTION_MATRIX: u32 = 0x0BA7;
pub const GL_VIEWPORT: u32 = 0x0BA2;

// Client arrays
pub const GL_VERTEX_ARRAY: u32 = 0x8074;
pub const GL_COLOR_ARRAY: u32 = 0x8076;
pub const GL_TEXTURE_COORD_ARRAY: u32 = 0x8078;

// Textures and pixel formats
pub const GL_TEXTURE_2D: u32 = 0x0DE1;
pub const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
pub const GL_TEXTURE_WRAP_S: u32 = 0x2802;
pub const GL_TEXTURE_WRAP_T: u32 = 0x2803;
pub const GL_LINEAR: u32 = 0x2601;
pub const GL_CLAMP_TO_EDGE: u32 = 0x812F;
pub const GL_RGB: u32 = 0x1907;
pub const GL_RGBA: u32 = 0x1908;

// Data types and pixel store
pub const GL_UNSIGNED_BYTE: u32 = 0x1401;
pub const GL_FLOAT: u32 = 0x1406;
pub const GL_PACK_ALIGNMENT: u32 = 0x0D05;
pub const GL_DEPTH_BITS: u32 = 0x0D56;
pub const GL_STENCIL_BITS: u32 = 0x0D57;

#[cfg_attr(windows, link(name = "opengl32"))]
#[cfg_attr(target_os = "macos", link(name = "OpenGL", kind = "framework"))]
#[cfg_attr(not(any(windows, target_os = "macos")), link(name = "GL"))]
unsafe extern "C" {
    pub fn glViewport(x: i32, y: i32, width: i32, height: i32);
    pub fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    pub fn glEnable(cap: u32);
    pub fn glClipPlane(plane: u32, equation: *const f64);
    pub fn glScissor(x: i32, y: i32, width: i32, height: i32);
    pub fn glStencilFunc(func: u32, reference: i32, mask: u32);
    pub fn glStencilOp(fail: u32, zfail: u32, zpass: u32);
    pub fn glClearStencil(s: i32);
    pub fn glColorMask(r: u8, g: u8, b: u8, a: u8);
    pub fn glDepthFunc(func: u32);
    pub fn glDepthRange(near: f64, far: f64);
    pub fn glGetFloatv(pname: u32, params: *mut f32);
    pub fn glGetIntegerv(pname: u32, params: *mut i32);
    pub fn glPolygonMode(face: u32, mode: u32);
    pub fn glClear(mask: u32);
    pub fn glMatrixMode(mode: u32);
    pub fn glLoadIdentity();
    pub fn glMultMatrixf(m: *const f32);
    pub fn glBegin(mode: u32);
    pub fn glEnd();
    pub fn glEnableClientState(array: u32);
    pub fn glDisableClientState(array: u32);
    pub fn glVertexPointer(size: i32, kind: u32, stride: i32, pointer: *const std::ffi::c_void);
    pub fn glTexCoordPointer(size: i32, kind: u32, stride: i32, pointer: *const std::ffi::c_void);
    pub fn glColorPointer(size: i32, kind: u32, stride: i32, pointer: *const std::ffi::c_void);
    pub fn glDrawArrays(mode: u32, first: i32, count: i32);
    pub fn glPointSize(size: f32);
    pub fn glVertex3f(x: f32, y: f32, z: f32);
    pub fn glRotatef(angle: f32, x: f32, y: f32, z: f32);
    pub fn glTranslatef(x: f32, y: f32, z: f32);
    pub fn glScalef(x: f32, y: f32, z: f32);
    pub fn glPushMatrix();
    pub fn glLineWidth(width: f32);
    pub fn glDepthMask(flag: u8);
    pub fn glPolygonOffset(factor: f32, units: f32);
    pub fn glColor4f(r: f32, g: f32, b: f32, a: f32);
    pub fn glBlendFunc(sfactor: u32, dfactor: u32);
    pub fn glAlphaFunc(func: u32, reference: f32);
    pub fn glPopMatrix();
    pub fn glColor3f(r: f32, g: f32, b: f32);
    pub fn glTexCoord2f(s: f32, t: f32);
    pub fn glTexParameteri(target: u32, pname: u32, param: i32);
    pub fn glGenTextures(n: i32, textures: *mut u32);
    pub fn glBindTexture(target: u32, texture: u32);
    pub fn glTexImage2D(
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
    pub fn glDisable(cap: u32);
    pub fn glCullFace(mode: u32);
    pub fn glFrontFace(mode: u32);
    pub fn glReadPixels(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: u32,
        type_: u32,
        pixels: *mut std::ffi::c_void
    );
    pub fn glPixelStorei(pname: u32, param: i32);
}

// Read the current back buffer as top-down RGB8
pub fn read_framebuffer(width: i32, height: i32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut bottom_up = vec![0u8; w * h * 3];
    unsafe {
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glReadPixels(0, 0, width, height, GL_RGB, GL_UNSIGNED_BYTE,
            bottom_up.as_mut_ptr() as *mut std::ffi::c_void);
    }
    let mut out = Vec::with_capacity(bottom_up.len());
    for row in bottom_up.chunks_exact(w * 3).rev() {
        out.extend_from_slice(row);
    }
    out
}

pub fn glu_perspective(fov_y: f32, aspect: f32, z_near: f32, z_far: f32) {
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


// 2D debug text (stroke lines) overlay

/// A column-major 4x4 matrix times a vector
pub fn mul_mat4(m: &[f32; 16], v: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|row| (0..4).map(|col| m[col * 4 + row] * v[col]).sum())
}

/// Cut the drawing at these planes (a, b, c, d), keeping a*x + b*y + c*z + d >= 0, and
/// stop cutting at any plane not given
pub unsafe fn set_clip_planes(planes: &[[f32; 4]]) {
    unsafe {
        for i in 0..6u32 {
            match planes.get(i as usize) {
                Some(p) => {
                    glClipPlane(GL_CLIP_PLANE0 + i, [p[0] as f64, p[1] as f64, p[2] as f64, p[3] as f64].as_ptr());
                    glEnable(GL_CLIP_PLANE0 + i);
                }
                None => glDisable(GL_CLIP_PLANE0 + i),
            }
        }
    }
}

pub fn ortho_2d(left: f32, right: f32, bottom: f32, top: f32) {
    // Column-major orthographic matrix
    let rl = right - left;
    let tb = top - bottom;
    let mut m = [0.0f32; 16];
    m[0]  = 2.0 / rl;
    m[5]  = 2.0 / tb;
    m[10] = -1.0;
    m[12] = -(right + left) / rl;
    m[13] = -(top + bottom) / tb;
    m[15] = 1.0;
    unsafe { glMultMatrixf(m.as_ptr()); }
}

static WORLD_GAMMA: std::sync::OnceLock<[u8; 256]> = std::sync::OnceLock::new();

/// Set once at startup: the brightening `upload_world_*` give to textures. Without it they change nothing.
pub fn set_world_gamma(gamma: f32) {
    if gamma != 1.0 {
        let _ = WORLD_GAMMA.set(rogue_reborn::lighting::gamma_table(gamma));
    }
}

/// Like `upload_rgba_to_gl`, for what is drawn in the 3D world: the gamma brightens its color
pub fn upload_world_rgba_to_gl(width: u32, height: u32, rgba: &[u8]) -> Option<u32> {
    match WORLD_GAMMA.get() {
        Some(table) => {
            let mut px = rgba.to_vec();
            for p in px.chunks_exact_mut(4) {
                for c in &mut p[..3] {
                    *c = table[*c as usize];
                }
            }
            upload_rgba_to_gl(width, height, &px)
        }
        None => upload_rgba_to_gl(width, height, rgba),
    }
}

pub fn upload_world_rsb_to_gl(tex: &rsb::Rsb) -> Option<u32> {
    upload_world_rgba_to_gl(tex.width, tex.height, &tex.to_rgba8())
}

pub fn upload_rgba_to_gl(width: u32, height: u32, rgba: &[u8]) -> Option<u32> {
    if rgba.len() != (width * height * 4) as usize || rgba.is_empty() {
        return None;
    }

    let mut id = 0;
    unsafe {
        glGenTextures(1, &mut id as *mut u32);
        glBindTexture(GL_TEXTURE_2D, id);

        // Simple filters; can be refined per material address mode
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);

        glTexImage2D(
            GL_TEXTURE_2D,
            0,                   // level
            GL_RGBA as i32,
            width as i32,
            height as i32,
            0,                   // border
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            rgba.as_ptr() as *const std::ffi::c_void
        );
    }
    Some(id)
}
