//! Software OpenGL 1.x subset (`opengl32`/`glu32`/`wgl`, plus the GDI pixel-format and `SwapBuffers` calls): immediate-mode
//! primitives, matrix stack, Gouraud shading and a depth buffer, rasterised on the CPU into the window's client bitmap.
//! Float arguments arrive in XMM registers on x64; the kernel passes `xmm0..3` in [`Process::xmm`].

use crate::{Process, Ret};
use alloc::vec::Vec;
use loos_win32k::Win32k;

/// First syscall number of this table.
pub const GL_BASE: u32 = 0xa00;
const PI: f64 = 3.141_592_653_589_793;

type M = [f64; 16];

fn sin(x: f64) -> f64 {
    let k = (x / (2.0 * PI)) as i64;
    let mut x = x - k as f64 * 2.0 * PI;
    if x > PI {
        x -= 2.0 * PI;
    } else if x < -PI {
        x += 2.0 * PI;
    }
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    for n in 1..8 {
        term *= -x2 / ((2 * n) as f64 * (2 * n + 1) as f64);
        sum += term;
    }
    sum
}
fn cos(x: f64) -> f64 {
    sin(x + PI / 2.0)
}
fn sqrt(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut g = f64::from_bits((x.to_bits() >> 1) + 0x1ff7_a3be_a91d_9b1b);
    for _ in 0..5 {
        g = 0.5 * (g + x / g);
    }
    g
}
fn tan(x: f64) -> f64 {
    sin(x) / cos(x)
}

const IDENT: M = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Column-major `a * b`.
fn mul(a: &M, b: &M) -> M {
    let mut r = [0.0; 16];
    for c in 0..4 {
        for row in 0..4 {
            r[c * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[c * 4 + k]).sum();
        }
    }
    r
}

fn xform(m: &M, v: [f64; 4]) -> [f64; 4] {
    let mut r = [0.0; 4];
    for (row, o) in r.iter_mut().enumerate() {
        *o = (0..4).map(|k| m[k * 4 + row] * v[k]).sum();
    }
    r
}

#[derive(Clone, Copy)]
struct Vtx {
    x: f64,
    y: f64,
    z: f64,
    c: [f64; 4],
}

/// One GL context: framebuffer, depth buffer, matrix stacks and the current immediate-mode batch.
pub struct Gl {
    w: i32,
    h: i32,
    color: Vec<u32>,
    depth: Vec<f32>,
    clear: [f64; 4],
    clear_depth: f64,
    mode: u32,
    mv: Vec<M>,
    proj: Vec<M>,
    matrix_mode: u32,
    cur: [f64; 4],
    batch: Option<u32>,
    verts: Vec<Vtx>,
    viewport: (i32, i32, i32, i32),
    depth_test: bool,
    hdc: u32,
}

impl Gl {
    fn new(w: i32, h: i32) -> Gl {
        Gl { w, h, color: alloc::vec![0xff00_0000; (w * h) as usize], depth: alloc::vec![1.0; (w * h) as usize], clear: [0.0, 0.0, 0.0, 1.0], clear_depth: 1.0, mode: 0x1700, mv: alloc::vec![IDENT], proj: alloc::vec![IDENT], matrix_mode: 0x1700, cur: [1.0; 4], batch: None, verts: Vec::new(), viewport: (0, 0, w, h), depth_test: false, hdc: 0 }
    }

    fn top(&mut self) -> &mut M {
        let s = if self.matrix_mode == 0x1701 { &mut self.proj } else { &mut self.mv };
        s.last_mut().unwrap()
    }

    fn apply(&mut self, m: &M) {
        let t = *self.top();
        *self.top() = mul(&t, m);
    }

    fn vertex(&mut self, x: f64, y: f64, z: f64) {
        let mvp = mul(self.proj.last().unwrap(), self.mv.last().unwrap());
        let c = xform(&mvp, [x, y, z, 1.0]);
        let (vx, vy, vw, vh) = self.viewport;
        let w = if c[3].abs() < 1e-9 { 1e-9 } else { c[3] };
        let (nx, ny, nz) = (c[0] / w, c[1] / w, c[2] / w);
        self.verts.push(Vtx { x: f64::from(vx) + (nx + 1.0) * 0.5 * f64::from(vw), y: f64::from(vy) + (1.0 - (ny + 1.0) * 0.5) * f64::from(vh), z: if w <= 0.0 { 2.0 } else { (nz + 1.0) * 0.5 }, c: self.cur });
    }

    fn put(&mut self, x: i32, y: i32, z: f64, c: [f64; 4]) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = (y * self.w + x) as usize;
        if self.depth_test {
            if z as f32 >= self.depth[i] {
                return;
            }
            self.depth[i] = z as f32;
        }
        let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        self.color[i] = 0xff00_0000 | q(c[0]) << 16 | q(c[1]) << 8 | q(c[2]);
    }

    fn tri(&mut self, a: Vtx, b: Vtx, c: Vtx) {
        if a.z > 1.5 || b.z > 1.5 || c.z > 1.5 {
            return; // behind the eye: no near-plane clipping in this subset
        }
        let edge = |p: &Vtx, q: &Vtx, x: f64, y: f64| (q.x - p.x) * (y - p.y) - (q.y - p.y) * (x - p.x);
        let area = edge(&a, &b, c.x, c.y);
        if area.abs() < 1e-9 {
            return;
        }
        let min_x = (a.x.min(b.x).min(c.x) as i32).max(0);
        let max_x = (a.x.max(b.x).max(c.x) as i32 + 1).min(self.w - 1);
        let min_y = (a.y.min(b.y).min(c.y) as i32).max(0);
        let max_y = (a.y.max(b.y).max(c.y) as i32 + 1).min(self.h - 1);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let (w0, w1, w2) = (edge(&b, &c, px, py) / area, edge(&c, &a, px, py) / area, edge(&a, &b, px, py) / area);
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * a.z + w1 * b.z + w2 * c.z;
                let col = [0, 1, 2, 3].map(|k| w0 * a.c[k] + w1 * b.c[k] + w2 * c.c[k]);
                self.put(x, y, z, col);
            }
        }
    }

    fn line(&mut self, a: Vtx, b: Vtx) {
        let n = (b.x - a.x).abs().max((b.y - a.y).abs()).max(1.0) as i32;
        for i in 0..=n {
            let t = f64::from(i) / f64::from(n);
            self.put((a.x + (b.x - a.x) * t) as i32, (a.y + (b.y - a.y) * t) as i32, a.z + (b.z - a.z) * t, [0, 1, 2, 3].map(|k| a.c[k] + (b.c[k] - a.c[k]) * t));
        }
    }

    fn end(&mut self) {
        let Some(mode) = self.batch.take() else { return };
        let v = core::mem::take(&mut self.verts);
        match mode {
            0 => v.iter().for_each(|p| self.put(p.x as i32, p.y as i32, p.z, p.c)),
            1 => v.chunks_exact(2).for_each(|p| self.line(p[0], p[1])),
            2 | 3 => {
                for i in 1..v.len() {
                    self.line(v[i - 1], v[i]);
                }
                if mode == 2 && v.len() > 2 {
                    self.line(v[v.len() - 1], v[0]);
                }
            }
            4 => v.chunks_exact(3).for_each(|t| self.tri(t[0], t[1], t[2])),
            5 => {
                for i in 2..v.len() {
                    if i % 2 == 0 { self.tri(v[i - 2], v[i - 1], v[i]) } else { self.tri(v[i - 1], v[i - 2], v[i]) }
                }
            }
            6 | 9 => {
                for i in 2..v.len() {
                    self.tri(v[0], v[i - 1], v[i]);
                }
            }
            7 => v.chunks_exact(4).for_each(|q| {
                self.tri(q[0], q[1], q[2]);
                self.tri(q[0], q[2], q[3]);
            }),
            8 => {
                for q in v.windows(4).step_by(2) {
                    self.tri(q[0], q[1], q[2]);
                    self.tri(q[1], q[3], q[2]);
                }
            }
            _ => {}
        }
    }
}

const NAMES: &[&str] = &[
    "wglCreateContext", "wglMakeCurrent", "wglDeleteContext", "wglGetCurrentContext", "wglGetProcAddress", "ChoosePixelFormat",
    "SetPixelFormat", "GetPixelFormat", "DescribePixelFormat", "SwapBuffers", "glClearColor", "glClear", "glClearDepth", "glViewport",
    "glMatrixMode", "glLoadIdentity", "glPushMatrix", "glPopMatrix", "glTranslatef", "glScalef", "glRotatef", "glOrtho", "glFrustum",
    "gluPerspective", "glBegin", "glEnd", "glVertex2f", "glVertex3f", "glVertex2d", "glVertex3d", "glVertex2i", "glVertex3i",
    "glVertex3fv", "glColor3f", "glColor4f", "glColor3ub", "glColor4ub", "glColor3fv", "glEnable", "glDisable", "glTranslated",
    "glRotated", "glScaled", "glColor3d", "glFlush", "glFinish", "glGetError", "glShadeModel", "glDepthFunc", "glHint", "glPolygonMode",
    "glCullFace", "glFrontFace", "glLineWidth", "glPointSize", "glBlendFunc", "glNormal3f", "glTexCoord2f", "glDepthMask",
    "glLightfv", "glMaterialfv", "glColorMaterial", "glLightModelfv", "glGetString", "glGetIntegerv", "gluLookAt", "glRectf",
];

/// Syscall number of an opengl32/glu32/wgl import.
pub fn lookup(name: &str) -> Option<u32> {
    NAMES.iter().position(|n| *n == name).map(|i| GL_BASE + i as u32)
}

impl Process {
    fn f32x(&self, i: usize) -> f64 {
        f64::from(f32::from_bits(u32::from_le_bytes(self.xmm[i][0..4].try_into().unwrap())))
    }
    fn f64x(&self, i: usize) -> f64 {
        f64::from_bits(u64::from_le_bytes(self.xmm[i][0..8].try_into().unwrap()))
    }

    /// OpenGL, WGL and pixel-format calls.
    pub(crate) fn gl_syscall(&mut self, nr: u32, a: [u64; 4], rsp: u64, w: &mut Win32k) -> Ret {
        let name = NAMES.get((nr - GL_BASE) as usize).copied().unwrap_or("");
        let v = |x: u64| Ret::Value(x);
        match name {
            "wglCreateContext" => {
                let (cw, ch) = w.dc_window(a[0] as u32).and_then(|h| w.get_client_rect(h).ok()).map_or((640, 480), |r| (r.right - r.left, r.bottom - r.top));
                self.gls.push(Some(Gl::new(cw.max(1), ch.max(1))));
                return v(0x40000 + (self.gls.len() as u64 - 1) * 4);
            }
            "wglMakeCurrent" => {
                let i = a[1].checked_sub(0x40000).map(|x| x as usize / 4);
                self.gl_cur = i.filter(|&i| self.gls.get(i).is_some_and(Option::is_some));
                if let Some(g) = self.gl_cur.and_then(|i| self.gls[i].as_mut()) {
                    g.hdc = a[0] as u32;
                }
                return v(1);
            }
            "wglDeleteContext" => {
                if let Some(i) = a[0].checked_sub(0x40000).map(|x| x as usize / 4) {
                    if let Some(s) = self.gls.get_mut(i) {
                        *s = None;
                    }
                }
                return v(1);
            }
            "wglGetCurrentContext" => return v(self.gl_cur.map_or(0, |i| 0x40000 + i as u64 * 4)),
            "ChoosePixelFormat" | "SetPixelFormat" | "GetPixelFormat" | "DescribePixelFormat" => return v(1),
            "SwapBuffers" => {
                let (Some(i), Some(hw)) = (self.gl_cur, w.dc_window(a[0] as u32)) else { return v(0) };
                if let Some(g) = self.gls[i].as_ref() {
                    w.present_pixels(hw, g.w, g.h, &g.color);
                }
                return v(1);
            }
            _ => {}
        }
        let (f, f64v) = (|i| self.f32x(i), |i| self.f64x(i));
        let Some(gi) = self.gl_cur else { return v(0) };
        let st = [self.f32x(0), self.f32x(1), self.f32x(2), self.f32x(3)];
        let dd = [f64v(0), f64v(1), f64v(2), f64v(3)];
        let _ = f;
        let (d5, d6) = (f64::from_bits(self.stack_arg(rsp, 5)), f64::from_bits(self.stack_arg(rsp, 6)));
        let g = self.gls[gi].as_mut().unwrap();
        let mat_translate = |x: f64, y: f64, z: f64| {
            let mut m = IDENT;
            (m[12], m[13], m[14]) = (x, y, z);
            m
        };
        match name {
            "glClearColor" => g.clear = st,
            "glClearDepth" => g.clear_depth = dd[0],
            "glClear" => {
                if a[0] & 0x4000 != 0 {
                    let c = g.clear;
                    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
                    g.color.fill(0xff00_0000 | q(c[0]) << 16 | q(c[1]) << 8 | q(c[2]));
                }
                if a[0] & 0x100 != 0 {
                    g.depth.fill(g.clear_depth as f32);
                }
            }
            "glViewport" => g.viewport = (a[0] as i32, a[1] as i32, a[2] as i32, a[3] as i32),
            "glMatrixMode" => g.matrix_mode = a[0] as u32,
            "glLoadIdentity" => *g.top() = IDENT,
            "glPushMatrix" => {
                let t = *g.top();
                if g.matrix_mode == 0x1701 { g.proj.push(t) } else { g.mv.push(t) }
            }
            "glPopMatrix" => {
                let s = if g.matrix_mode == 0x1701 { &mut g.proj } else { &mut g.mv };
                if s.len() > 1 {
                    s.pop();
                }
            }
            "glTranslatef" => g.apply(&mat_translate(st[0], st[1], st[2])),
            "glTranslated" => g.apply(&mat_translate(dd[0], dd[1], dd[2])),
            "glScalef" | "glScaled" => {
                let s = if name == "glScalef" { st } else { dd };
                let mut m = IDENT;
                (m[0], m[5], m[10]) = (s[0], s[1], s[2]);
                g.apply(&m);
            }
            "glRotatef" | "glRotated" => {
                let s = if name == "glRotatef" { st } else { dd };
                let (ang, mut x, mut y, mut z) = (s[0] * PI / 180.0, s[1], s[2], s[3]);
                let len = sqrt(x * x + y * y + z * z);
                if len > 0.0 {
                    (x, y, z) = (x / len, y / len, z / len);
                    let (c, sn) = (cos(ang), sin(ang));
                    let t = 1.0 - c;
                    let m = [t * x * x + c, t * x * y + sn * z, t * x * z - sn * y, 0.0, t * x * y - sn * z, t * y * y + c, t * y * z + sn * x, 0.0, t * x * z + sn * y, t * y * z - sn * x, t * z * z + c, 0.0, 0.0, 0.0, 0.0, 1.0];
                    g.apply(&m);
                }
            }
            "glOrtho" => {
                let (l, r, b, t, n, f) = (dd[0], dd[1], dd[2], dd[3], d5, d6);
                let mut m = IDENT;
                (m[0], m[5], m[10]) = (2.0 / (r - l), 2.0 / (t - b), -2.0 / (f - n));
                (m[12], m[13], m[14]) = (-(r + l) / (r - l), -(t + b) / (t - b), -(f + n) / (f - n));
                g.apply(&m);
            }
            "glFrustum" => {
                let (l, r, b, t, n, f) = (dd[0], dd[1], dd[2], dd[3], d5, d6);
                let mut m = [0.0; 16];
                (m[0], m[5], m[8], m[9]) = (2.0 * n / (r - l), 2.0 * n / (t - b), (r + l) / (r - l), (t + b) / (t - b));
                (m[10], m[11], m[14]) = (-(f + n) / (f - n), -1.0, -2.0 * f * n / (f - n));
                g.apply(&m);
            }
            "gluPerspective" => {
                let (fovy, aspect, n, f) = (dd[0], dd[1], dd[2], dd[3]);
                let t = 1.0 / tan(fovy * PI / 360.0);
                let mut m = [0.0; 16];
                (m[0], m[5], m[10], m[11], m[14]) = (t / aspect, t, (f + n) / (n - f), -1.0, 2.0 * f * n / (n - f));
                g.apply(&m);
            }
            "gluLookAt" => {} // not in this subset
            "glBegin" => {
                g.batch = Some(a[0] as u32);
                g.verts.clear();
            }
            "glEnd" => g.end(),
            "glVertex2f" => g.vertex(st[0], st[1], 0.0),
            "glVertex3f" => g.vertex(st[0], st[1], st[2]),
            "glVertex2d" => g.vertex(dd[0], dd[1], 0.0),
            "glVertex3d" => g.vertex(dd[0], dd[1], dd[2]),
            "glVertex2i" => g.vertex(a[0] as i32 as f64, a[1] as i32 as f64, 0.0),
            "glVertex3i" => g.vertex(a[0] as i32 as f64, a[1] as i32 as f64, a[2] as i32 as f64),
            "glVertex3fv" | "glColor3fv" => {
                let b = self.mem.read_vec(a[0], 12).unwrap_or_else(|_| alloc::vec![0; 12]);
                let f = |k: usize| f64::from(f32::from_bits(u32::from_le_bytes(b[k * 4..k * 4 + 4].try_into().unwrap())));
                let g = self.gls[gi].as_mut().unwrap();
                if name == "glVertex3fv" { g.vertex(f(0), f(1), f(2)) } else { g.cur = [f(0), f(1), f(2), 1.0] }
            }
            "glColor3f" => g.cur = [st[0], st[1], st[2], 1.0],
            "glColor3d" => g.cur = [dd[0], dd[1], dd[2], 1.0],
            "glColor4f" => g.cur = st,
            "glColor3ub" => g.cur = [f64::from(a[0] as u8) / 255.0, f64::from(a[1] as u8) / 255.0, f64::from(a[2] as u8) / 255.0, 1.0],
            "glColor4ub" => g.cur = [f64::from(a[0] as u8) / 255.0, f64::from(a[1] as u8) / 255.0, f64::from(a[2] as u8) / 255.0, f64::from(a[3] as u8) / 255.0],
            "glEnable" | "glDisable" => {
                if a[0] == 0x0b71 {
                    g.depth_test = name == "glEnable";
                }
            }
            "glRectf" => {
                g.batch = Some(7);
                g.verts.clear();
                g.vertex(st[0], st[1], 0.0);
                g.vertex(st[2], st[1], 0.0);
                g.vertex(st[2], st[3], 0.0);
                g.vertex(st[0], st[3], 0.0);
                g.end();
            }
            "glGetString" => return v(0),
            _ => {} // state calls the software path ignores (lighting, blending, culling, hints, ...)
        }
        v(0)
    }
}
