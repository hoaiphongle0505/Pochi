//! Small 2D toolkit on top of tiny-skia: logical-pixel canvas with DPI scale, clip mask,
//! global alpha/offset, cached text (ab_glyph) and cached SVG icons (resvg).

use std::collections::HashMap;
use tiny_skia::*;

pub fn rgb(c: u32, a: f32) -> Color {
    Color::from_rgba8(((c >> 16) & 255) as u8, ((c >> 8) & 255) as u8, (c & 255) as u8, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum W { Regular, Semi, Bold }

pub struct Glyph { pub w: u32, pub h: u32, pub x: i32, pub y: i32, pub bmp: Vec<u8> }

pub struct Fonts {
    fonts: Vec<ab_glyph::FontVec>,
    cache: HashMap<(u8, char, u32), Glyph>,
}

fn read_first(paths: &[&str]) -> Option<Vec<u8>> {
    paths.iter().find_map(|p| std::fs::read(p).ok())
}

impl Fonts {
    pub fn load() -> Fonts {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        let f = |n: &str| format!("{}\\Fonts\\{}", windir, n);
        let sets: [Vec<String>; 3] = [
            vec![f("segoeui.ttf"), "/usr/share/fonts/opentype/inter/Inter-Regular.otf".into(), "Z:\\usr\\share\\fonts\\opentype\\inter\\Inter-Regular.otf".into(), f("arial.ttf"), f("tahoma.ttf")],
            vec![f("seguisb.ttf"), "/usr/share/fonts/opentype/inter/Inter-SemiBold.otf".into(), "Z:\\usr\\share\\fonts\\opentype\\inter\\Inter-SemiBold.otf".into(), f("segoeui.ttf"), f("arial.ttf"), f("tahoma.ttf")],
            vec![f("segoeuib.ttf"), "/usr/share/fonts/opentype/inter/Inter-Bold.otf".into(), "Z:\\usr\\share\\fonts\\opentype\\inter\\Inter-Bold.otf".into(), f("arialbd.ttf"), f("tahomabd.ttf"), f("segoeui.ttf")],
        ];
        let mut fonts: Vec<ab_glyph::FontVec> = vec![];
        for set in sets.iter() {
            let refs: Vec<&str> = set.iter().map(|s| s.as_str()).collect();
            let font = read_first(&refs).and_then(|b| ab_glyph::FontVec::try_from_vec(b).ok());
            match font {
                Some(f) => fonts.push(f),
                None => if let Some(b) = fonts.first().map(|f| ab_glyph::Font::font_data(f).to_vec()) { if let Ok(f) = ab_glyph::FontVec::try_from_vec(b) { fonts.push(f); } },
            }
        }
        Fonts { fonts, cache: HashMap::new() }
    }

    fn idx(w: W) -> u8 { match w { W::Regular => 0, W::Semi => 1, W::Bold => 2 } }

    fn font(&self, w: W) -> Option<&ab_glyph::FontVec> { self.fonts.get(Self::idx(w) as usize).or(self.fonts.first()) }

    fn glyph(&mut self, w: W, c: char, px: f32) -> Option<&Glyph> {
        use ab_glyph::{Font, ScaleFont};
        let key = (Self::idx(w), c, (px * 4.0).round() as u32);
        if !self.cache.contains_key(&key) {
            let font = self.font(w)?;
            let sf = font.as_scaled(px);
            let id = font.glyph_id(c);
            let g = id.with_scale_and_position(sf.scale(), ab_glyph::point(0.0, 0.0));
            let glyph = match font.outline_glyph(g) {
                Some(og) => {
                    let b = og.px_bounds();
                    let (gw, gh) = (b.width().ceil() as u32, b.height().ceil() as u32);
                    let mut bmp = vec![0u8; (gw * gh) as usize];
                    og.draw(|x, y, v| { let i = (y * gw + x) as usize; if i < bmp.len() { bmp[i] = (v.clamp(0.0, 1.0) * 255.0) as u8; } });
                    Glyph { w: gw, h: gh, x: b.min.x.floor() as i32, y: b.min.y.floor() as i32, bmp }
                }
                None => Glyph { w: 0, h: 0, x: 0, y: 0, bmp: vec![] },
            };
            if self.cache.len() > 1500 { self.cache.clear(); }
            self.cache.insert(key, glyph);
        }
        self.cache.get(&key)
    }

    fn advance(&self, w: W, prev: Option<char>, c: char, px: f32) -> f32 {
        use ab_glyph::{Font, ScaleFont};
        let Some(font) = self.font(w) else { return 0.0 };
        let sf = font.as_scaled(px);
        let id = font.glyph_id(c);
        let k = prev.map(|p| sf.kern(font.glyph_id(p), id)).unwrap_or(0.0);
        k + sf.h_advance(id)
    }

    fn kern(&self, w: W, prev: char, c: char, px: f32) -> f32 {
        use ab_glyph::{Font, ScaleFont};
        let Some(font) = self.font(w) else { return 0.0 };
        font.as_scaled(px).kern(font.glyph_id(prev), font.glyph_id(c))
    }

    pub fn measure(&mut self, s: &str, px: f32, w: W) -> f32 {
        let mut x = 0.0;
        let mut prev = None;
        for c in s.chars() { x += self.advance(w, prev, c, px); prev = Some(c); }
        x
    }

    pub fn ellipsize(&mut self, s: &str, px: f32, w: W, max: f32) -> String {
        if self.measure(s, px, w) <= max { return s.to_string(); }
        let mut out: String = String::new();
        for c in s.chars() {
            let mut t = out.clone(); t.push(c); t.push('…');
            if self.measure(&t, px, w) > max { break; }
            out.push(c);
        }
        out.push('…');
        out
    }
}

pub struct Icons { cache: HashMap<(&'static str, u32, u32), Pixmap> }
impl Icons {
    pub fn new() -> Icons { Icons { cache: HashMap::new() } }
    /// render an svg (with "currentColor") at `px` device pixels
    pub fn get(&mut self, name: &'static str, svg: &str, px: u32, color: u32) -> Option<&Pixmap> {
        let key = (name, px, color);
        if !self.cache.contains_key(&key) {
            let src = svg.replace("currentColor", &format!("#{:06x}", color));
            let tree = resvg::usvg::Tree::from_str(&src, &resvg::usvg::Options::default()).ok()?;
            let mut pm = Pixmap::new(px.max(1), px.max(1))?;
            let sz = tree.size();
            let sc = px as f32 / sz.width().max(sz.height());
            resvg::render(&tree, Transform::from_scale(sc, sc), &mut pm.as_mut());
            self.cache.insert(key, pm);
        }
        self.cache.get(&key)
    }
}

pub struct Canvas<'a> {
    pub pm: &'a mut Pixmap,
    pub s: f32,
    pub mask: Option<Mask>,
    pub alpha: f32,
    pub ox: f32,
    pub oy: f32,
    pub fonts: &'a mut Fonts,
    pub icons: &'a mut Icons,
}

pub fn rrect_path(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Option<Path> {
    // r = [top-left, top-right, bottom-right, bottom-left]
    let k = 0.552_284_8;
    let m = (w.min(h)) / 2.0;
    let r = r.map(|v| v.min(m).max(0.0));
    let mut pb = PathBuilder::new();
    pb.move_to(x + r[0], y);
    pb.line_to(x + w - r[1], y);
    if r[1] > 0.0 { pb.cubic_to(x + w - r[1] + r[1] * k, y, x + w, y + r[1] - r[1] * k, x + w, y + r[1]); }
    pb.line_to(x + w, y + h - r[2]);
    if r[2] > 0.0 { pb.cubic_to(x + w, y + h - r[2] + r[2] * k, x + w - r[2] + r[2] * k, y + h, x + w - r[2], y + h); }
    pb.line_to(x + r[3], y + h);
    if r[3] > 0.0 { pb.cubic_to(x + r[3] - r[3] * k, y + h, x, y + h - r[3] + r[3] * k, x, y + h - r[3]); }
    pb.line_to(x, y + r[0]);
    if r[0] > 0.0 { pb.cubic_to(x, y + r[0] - r[0] * k, x + r[0] - r[0] * k, y, x + r[0], y); }
    pb.close();
    pb.finish()
}

/// circular arc from `a0` sweeping `sweep` (degrees, clockwise, 0 = 3 o'clock)
pub fn arc_path(cx: f32, cy: f32, r: f32, a0: f32, sweep: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let segs = ((sweep.abs() / 90.0).ceil() as i32).max(1);
    let step = sweep / segs as f32;
    let mut a = a0.to_radians();
    pb.move_to(cx + r * a.cos(), cy + r * a.sin());
    for _ in 0..segs {
        let b = a + step.to_radians();
        let k = 4.0 / 3.0 * ((b - a) / 4.0).tan();
        let (c0, s0, c1, s1) = (a.cos(), a.sin(), b.cos(), b.sin());
        pb.cubic_to(cx + r * (c0 - k * s0), cy + r * (s0 + k * c0), cx + r * (c1 + k * s1), cy + r * (s1 - k * c1), cx + r * c1, cy + r * s1);
        a = b;
    }
    pb.finish()
}

impl<'a> Canvas<'a> {
    fn tf(&self) -> Transform { Transform::from_row(self.s, 0.0, 0.0, self.s, self.ox * self.s, self.oy * self.s) }

    fn paint(&self, c: u32, a: f32) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(rgb(c, a * self.alpha));
        p.anti_alias = true;
        p
    }

    pub fn fill(&mut self, path: &Path, c: u32, a: f32) {
        if a * self.alpha <= 0.002 { return; }
        let p = self.paint(c, a);
        let tf = self.tf();
        self.pm.fill_path(path, &p, FillRule::Winding, tf, self.mask.as_ref());
    }

    pub fn fill_shader(&mut self, path: &Path, shader: Shader<'static>) {
        let mut p = Paint::default();
        p.shader = shader;
        p.anti_alias = true;
        let tf = self.tf();
        self.pm.fill_path(path, &p, FillRule::Winding, tf, self.mask.as_ref());
    }

    pub fn stroke(&mut self, path: &Path, c: u32, a: f32, width: f32) {
        if a * self.alpha <= 0.002 { return; }
        let p = self.paint(c, a);
        let st = Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        let tf = self.tf();
        self.pm.stroke_path(path, &p, &st, tf, self.mask.as_ref());
    }

    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: u32, a: f32) {
        if w <= 0.0 || h <= 0.0 { return; }
        if let Some(p) = rrect_path(x, y, w, h, [r; 4]) { self.fill(&p, c, a); }
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, c: u32, a: f32) {
        if let Some(p) = PathBuilder::from_circle(cx, cy, r) { self.fill(&p, c, a); }
    }

    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: u32, a: f32) {
        if rx <= 0.0 || ry <= 0.0 { return; }
        if let Some(r) = Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0) {
            if let Some(p) = PathBuilder::from_oval(r) { self.fill(&p, c, a); }
        }
    }

    pub fn arc(&mut self, cx: f32, cy: f32, r: f32, a0: f32, sweep: f32, c: u32, a: f32, w: f32) {
        if sweep.abs() < 0.05 { return; }
        if let Some(p) = arc_path(cx, cy, r, a0, sweep) { self.stroke(&p, c, a, w); }
    }

    /// text with baseline at y; returns logical width
    pub fn text(&mut self, x: f32, y: f32, s: &str, size: f32, w: W, c: u32, a: f32) -> f32 {
        let alpha = a * self.alpha;
        let px = size * self.s;
        let (cr, cg, cb) = (((c >> 16) & 255) as f32, ((c >> 8) & 255) as f32, (c & 255) as f32);
        let mut pen = (x + self.ox) * self.s;
        let base = ((y + self.oy) * self.s).round() as i32;
        let pw = self.pm.width() as i32;
        let ph = self.pm.height() as i32;
        let mut prev: Option<char> = None;
        let widx = Fonts::idx(w) as usize;
        let fonts = &mut *self.fonts;
        let pm = &mut *self.pm;
        let mask = self.mask.as_ref();
        let _ = widx;
        for ch in s.chars() {
            if let Some(p) = prev { pen += fonts.kern(w, p, ch, px); }
            let adv = fonts.advance(w, None, ch, px);
            if alpha > 0.002 {
                if let Some(g) = fonts.glyph(w, ch, px) {
                    let gx = pen.round() as i32 + g.x;
                    let gy = base + g.y;
                    let data = pm.data_mut();
                    for row in 0..g.h as i32 {
                        let py = gy + row;
                        if py < 0 || py >= ph { continue; }
                        for col in 0..g.w as i32 {
                            let pxx = gx + col;
                            if pxx < 0 || pxx >= pw { continue; }
                            let mut cov = g.bmp[(row * g.w as i32 + col) as usize] as f32 / 255.0;
                            if cov <= 0.0 { continue; }
                            if let Some(mask) = mask { cov *= mask.data()[(py * pw + pxx) as usize] as f32 / 255.0; }
                            let sa = cov * alpha;
                            if sa <= 0.0 { continue; }
                            let i = ((py * pw + pxx) * 4) as usize;
                            let inv = 1.0 - sa;
                            data[i] = (cr * sa + data[i] as f32 * inv) as u8;
                            data[i + 1] = (cg * sa + data[i + 1] as f32 * inv) as u8;
                            data[i + 2] = (cb * sa + data[i + 2] as f32 * inv) as u8;
                            data[i + 3] = (255.0 * sa + data[i + 3] as f32 * inv) as u8;
                        }
                    }
                }
            }
            pen += adv;
            prev = Some(ch);
        }
        (pen / self.s) - x - self.ox
    }

    pub fn measure(&mut self, s: &str, size: f32, w: W) -> f32 { self.fonts.measure(s, size * self.s, w) / self.s }

    pub fn ellipsize(&mut self, s: &str, size: f32, w: W, max: f32) -> String {
        self.fonts.ellipsize(s, size * self.s, w, max * self.s)
    }

    pub fn text_center(&mut self, cx: f32, y: f32, s: &str, size: f32, w: W, c: u32, a: f32) {
        let tw = self.measure(s, size, w);
        self.text(cx - tw / 2.0, y, s, size, w, c, a);
    }

    pub fn text_right(&mut self, rx: f32, y: f32, s: &str, size: f32, w: W, c: u32, a: f32) {
        let tw = self.measure(s, size, w);
        self.text(rx - tw, y, s, size, w, c, a);
    }

    /// draw a cached svg icon centred in a `size` box at (x, y) top-left
    pub fn icon(&mut self, name: &'static str, svg: &str, x: f32, y: f32, size: f32, color: u32, a: f32) {
        let px = (size * self.s).round() as u32;
        let alpha = a * self.alpha;
        if alpha <= 0.002 { return; }
        let dx = ((x + self.ox) * self.s).round() as i32;
        let dy = ((y + self.oy) * self.s).round() as i32;
        let mask = self.mask.take();
        if let Some(icon) = self.icons.get(name, svg, px, color) {
            let paint = PixmapPaint { opacity: alpha, ..Default::default() };
            self.pm.draw_pixmap(dx, dy, icon.as_ref(), &paint, Transform::identity(), mask.as_ref());
        }
        self.mask = mask;
    }

    /// draw an RGBA (premultiplied) square image into a rounded rect
    pub fn image(&mut self, x: f32, y: f32, size: f32, r: f32, img: &Pixmap) {
        let Some(path) = rrect_path(x, y, size, size, [r; 4]) else { return };
        let sc = size / img.width() as f32;
        let mut p = Paint::default();
        p.anti_alias = true;
        p.shader = Pattern::new(img.as_ref(), SpreadMode::Pad, FilterQuality::Bicubic, self.alpha,
            Transform::from_row(sc, 0.0, 0.0, sc, x, y));
        let tf = self.tf();
        self.pm.fill_path(&path, &p, FillRule::Winding, tf, self.mask.as_ref());
    }
}
