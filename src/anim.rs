//! Time-based springs and tweens. Everything animates in real time (dt in seconds),
//! so animations look the same at any frame rate.

#[derive(Clone, Copy, Debug)]
pub struct Spring { pub x: f32, pub v: f32, pub target: f32, pub k: f32, pub c: f32 }

impl Spring {
    pub fn new(x: f32, k: f32, c: f32) -> Spring { Spring { x, v: 0.0, target: x, k, c } }
    /// stiffness / damping preset; zeta < 1 overshoots
    pub fn with(x: f32, freq: f32, zeta: f32) -> Spring {
        let k = (std::f32::consts::TAU * freq).powi(2);
        Spring::new(x, k, 2.0 * zeta * k.sqrt())
    }
    pub fn set(&mut self, t: f32) { self.target = t; }
    pub fn tune(&mut self, freq: f32, zeta: f32) {
        self.k = (std::f32::consts::TAU * freq).powi(2);
        self.c = 2.0 * zeta * self.k.sqrt();
    }
    pub fn snap(&mut self, t: f32) { self.x = t; self.target = t; self.v = 0.0; }
    pub fn step(&mut self, dt: f32) -> bool {
        if self.settled() { self.x = self.target; self.v = 0.0; return false; }
        // semi-implicit Euler with substeps for stability
        let n = ((dt / 0.004).ceil() as i32).clamp(1, 16);
        let h = dt / n as f32;
        for _ in 0..n {
            let a = -self.k * (self.x - self.target) - self.c * self.v;
            self.v += a * h;
            self.x += self.v * h;
        }
        true
    }
    pub fn settled(&self) -> bool {
        let scale = self.target.abs().max(1.0);
        (self.x - self.target).abs() < 0.002 * scale && self.v.abs() < 0.02 * scale
    }
}

pub fn clamp01(v: f32) -> f32 { v.clamp(0.0, 1.0) }
pub fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
pub fn ease_out(t: f32) -> f32 { let t = clamp01(t); 1.0 - (1.0 - t).powi(3) }
pub fn ease_in_out(t: f32) -> f32 { let t = clamp01(t); if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 } }
/// smooth 0..1 window between a and b
pub fn smooth(a: f32, b: f32, x: f32) -> f32 { let t = clamp01((x - a) / (b - a)); t * t * (3.0 - 2.0 * t) }
