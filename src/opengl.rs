//! The OpenGL calls the viewer uses (fixed-function, declared directly) and helpers for
//! matrices, textures and reading the screen back.

use rogue_reborn::rsb;

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
        glPixelStorei(0x0D05, 1); // GL_PACK_ALIGNMENT
        glReadPixels(0, 0, width, height, 0x1907, 0x1401, // GL_RGB, GL_UNSIGNED_BYTE
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
                    glClipPlane(0x3000 + i, [p[0] as f64, p[1] as f64, p[2] as f64, p[3] as f64].as_ptr());
                    glEnable(0x3000 + i);
                }
                None => glDisable(0x3000 + i),
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

pub fn upload_rsb_to_gl(tex: &rsb::Rsb) -> Option<u32> {
    upload_rgba_to_gl(tex.width, tex.height, &tex.to_rgba8())
}

pub fn upload_rgba_to_gl(width: u32, height: u32, rgba: &[u8]) -> Option<u32> {
    if rgba.len() != (width * height * 4) as usize || rgba.is_empty() {
        return None;
    }

    let mut id = 0;
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
            width as i32,
            height as i32,
            0,                   // border
            0x1908,              // GL_RGBA
            0x1401,              // GL_UNSIGNED_BYTE
            rgba.as_ptr() as *const std::ffi::c_void
        );
    }
    Some(id)
}
