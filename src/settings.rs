//! Settings window (Nusa-style): dark sidebar + scrolling sections. Drawn with the same toolkit
//! as the notch; platform independent so it can be previewed as PNG.

use crate::anim::*;
use crate::app::*;
use crate::gfx::*;
use crate::icons;
use crate::time::{self, now_ms};
use tiny_skia::*;

pub const SW: f32 = 960.0;
pub const SH: f32 = 680.0;
const SIDE: f32 = 270.0;
const CX0: f32 = SIDE + 34.0; // content left
const CW: f32 = SW - SIDE - 68.0; // content width
const TOP: f32 = 30.0;

const BG: u32 = 0x131315;
const SIDEBG: u32 = 0x18181b;
const CARD: u32 = 0x1f1f23;
const LINE: u32 = 0x2c2c31;
const BORDER: u32 = 0x2b2b30;
const MUTED: u32 = 0x98989f;
const WHITE: u32 = 0xf2f2f7;
const BLUE: u32 = 0x0a84ff;
const GREEN: u32 = 0x30d158;
const RED: u32 = 0xff6961;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Field { TodoText, EventTitle }

#[derive(Clone, Debug, PartialEq)]
pub enum SAct {
    Nav(&'static str),
    App(Action),
    Field(Field),
    TodoTime(i32),
    TodoDaily,
    TodoAdd,
    EventDay(i32),
    EventTime(i32),
    EventAdd,
    Platform(&'static str),
    Display(String),
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SOut { App(Action), Platform(&'static str), Display(String), Focus(bool), Close, Redraw }

pub struct Monitor { pub id: String, pub label: String, pub selected: bool }

pub struct SettingsUi {
    pub section: &'static str,
    scroll: f32,
    content_h: f32,
    pub mouse: (f32, f32),
    hits: Vec<((f32, f32, f32, f32), SAct)>,
    knobs: std::collections::HashMap<String, Spring>,
    pub focus: Option<Field>,
    todo_text: String,
    todo_time: Option<(u32, u32)>,
    todo_daily: bool,
    ev_title: String,
    ev_day: i32,
    ev_time: Option<(u32, u32)>,
    pub monitors: Vec<Monitor>,
    pub data_path: String,
    y: f32,
    clip: Option<(f32, Mask)>,
}

const SECTIONS: [(&str, &str, &str, u32); 9] = [
    ("general", "General", "gear", 0x8e8e93),
    ("display", "Display", "monitor", 0x5e5ce6),
    ("appearance", "Appearance", "contrast", 0x48484a),
    ("media", "Music and lyrics", "note", 0xff375f),
    ("todo", "To-do", "check", 0x30d158),
    ("calendar", "Calendar", "calendar", 0xff453a),
    ("pomodoro", "Pomodoro", "timer", 0xff6b35),
    ("water", "Water", "drop", 0x0a84ff),
    ("hud", "HUD & popups", "bell", 0x32ade6),
];

fn icon_svg(name: &str) -> (&'static str, &'static str) {
    match name {
        "gear" => ("gear", icons::GEAR), "monitor" => ("monitor", icons::MONITOR), "contrast" => ("contrast", icons::CONTRAST),
        "note" => ("note", icons::NOTE), "check" => ("check", icons::CHECK), "calendar" => ("calendar", icons::CALENDAR),
        "timer" => ("timer", icons::TIMER), "drop" => ("drop", icons::DROP), "bell" => ("bell", icons::BELL),
        _ => ("info", icons::INFO),
    }
}

fn inside(r: (f32, f32, f32, f32), p: (f32, f32)) -> bool { p.0 >= r.0 && p.0 <= r.0 + r.2 && p.1 >= r.1 && p.1 <= r.1 + r.3 }

fn accent_of(section: &str) -> u32 {
    SECTIONS.iter().find(|x| x.0 == section).map(|x| match x.0 { "general" => 0x7d7aff, "appearance" => 0xbf5af2, _ => x.3 }).unwrap_or(0x7d7aff)
}

impl SettingsUi {
    fn accent(&self) -> u32 { accent_of(self.section) }

    pub fn new(section: &'static str) -> SettingsUi {
        SettingsUi {
            section, scroll: 0.0, content_h: 0.0, mouse: (-1.0, -1.0), hits: vec![], knobs: Default::default(), focus: None,
            todo_text: String::new(), todo_time: None, todo_daily: false, ev_title: String::new(), ev_day: 0, ev_time: None,
            monitors: vec![], data_path: String::new(), y: 0.0, clip: None,
        }
    }

    pub fn goto(&mut self, s: &str) {
        if let Some(x) = SECTIONS.iter().map(|x| x.0).chain(["about"]).find(|x| *x == s) { self.section = x; self.scroll = 0.0; self.focus = None; }
    }

    /// is the point in the draggable title strip (not over a control)
    pub fn is_drag(&self, p: (f32, f32)) -> bool {
        p.1 < 44.0 && !(p.0 < 40.0 && p.1 < 40.0) && !self.hits.iter().any(|(r, _)| inside(*r, p))
    }

    pub fn animating(&mut self, dt: f32, reduced_motion: bool) -> bool {
        let mut a = false;
        for k in self.knobs.values_mut() {
            if reduced_motion { k.snap(k.target); }
            else if k.step(dt) { a = true; }
        }
        a
    }

    pub fn wheel(&mut self, d: f32) -> bool {
        let max = (self.content_h - (SH - TOP - 20.0)).max(0.0);
        let before = self.scroll;
        self.scroll = (self.scroll - d * 60.0).clamp(0.0, max);
        before != self.scroll
    }

    pub fn click(&mut self, p: (f32, f32)) -> Vec<SOut> {
        let Some(act) = self.hits.iter().rev().find(|(r, _)| inside(*r, p)).map(|h| h.1.clone()) else {
            if self.focus.is_some() { self.focus = None; return vec![SOut::Focus(false), SOut::Redraw]; }
            return vec![];
        };
        let mut out = vec![SOut::Redraw];
        let step_time = |t: Option<(u32, u32)>, d: i32| -> Option<(u32, u32)> {
            Some(match t {
                None => { let l = time::local(now_ms()); let m = (l.hour * 60 + l.minute + 15) / 15 * 15; ((m / 60) % 24, m % 60) }
                Some((h, m)) => { let v = ((h * 60 + m) as i32 + d * 15).rem_euclid(1440) as u32; (v / 60, v % 60) }
            })
        };
        match act {
            SAct::Nav(s) => { self.goto(s); out.push(SOut::Focus(false)); }
            SAct::App(a) => out.push(SOut::App(a)),
            SAct::Field(f) => { self.focus = Some(f); out.push(SOut::Focus(true)); }
            SAct::TodoTime(d) => self.todo_time = if d == 0 { None } else { step_time(self.todo_time, d) },
            SAct::TodoDaily => self.todo_daily = !self.todo_daily,
            SAct::TodoAdd => out.extend(self.submit(Field::TodoText)),
            SAct::EventDay(d) => self.ev_day = (self.ev_day + d).clamp(0, 365),
            SAct::EventTime(d) => self.ev_time = if d == 0 { None } else { step_time(self.ev_time, d) },
            SAct::EventAdd => out.extend(self.submit(Field::EventTitle)),
            SAct::Platform(p) => out.push(SOut::Platform(p)),
            SAct::Display(id) => out.push(SOut::Display(id)),
            SAct::Close => out.push(SOut::Close),
        }
        out
    }

    fn submit(&mut self, f: Field) -> Vec<SOut> {
        match f {
            Field::TodoText => {
                if self.todo_text.trim().is_empty() { self.focus = Some(f); return vec![SOut::Focus(true)]; }
                let a = Action::TodoAdd { text: std::mem::take(&mut self.todo_text), time: self.todo_time.take(), daily: std::mem::take(&mut self.todo_daily) };
                vec![SOut::App(a)]
            }
            Field::EventTitle => {
                if self.ev_title.trim().is_empty() { self.focus = Some(f); return vec![SOut::Focus(true)]; }
                let date = time::day_key(time::start_of_local_day(now_ms()) + self.ev_day as i64 * 86_400_000 + 43_200_000);
                let a = Action::EventAdd { title: std::mem::take(&mut self.ev_title), date, time: self.ev_time.take() };
                self.ev_day = 0;
                vec![SOut::App(a)]
            }
        }
    }

    pub fn char_input(&mut self, c: char) -> Vec<SOut> {
        let Some(f) = self.focus else { return vec![] };
        let t = if f == Field::TodoText { &mut self.todo_text } else { &mut self.ev_title };
        match c {
            '\u{8}' => { t.pop(); }
            '\r' | '\n' => return [self.submit(f), vec![SOut::Redraw]].concat(),
            '\u{1b}' => { self.focus = None; return vec![SOut::Focus(false), SOut::Redraw]; }
            c if !c.is_control() => { if t.chars().count() < 200 { t.push(c); } }
            _ => {}
        }
        vec![SOut::Redraw]
    }

    /* =================================== drawing =================================== */

    fn hit(&mut self, cv: &Canvas, x: f32, y: f32, w: f32, h: f32, a: SAct) {
        let r = (x, y + cv.oy, w, h);
        // ignore controls scrolled under the title strip
        if cv.oy != 0.0 && r.1 + r.3 < TOP + 4.0 { return; }
        self.hits.push((r, a));
    }
    fn hov(&self, cv: &Canvas, x: f32, y: f32, w: f32, h: f32) -> bool { inside((x, y + cv.oy, w, h), self.mouse) }

    pub fn draw(&mut self, app: &App, cv: &mut Canvas) {
        self.hits.clear();
        cv.pm.fill(crate::gfx::rgb(BG, 1.0));
        let acc = self.accent();
        // soft accent glow behind the section header
        if let Some(g) = RadialGradient::new(Point::from_xy((SIDE + 260.0) * cv.s, 0.0), Point::from_xy((SIDE + 260.0) * cv.s, 0.0), 420.0 * cv.s,
            vec![GradientStop::new(0.0, rgb(acc, 0.20)), GradientStop::new(0.55, rgb(acc, 0.05)), GradientStop::new(1.0, rgb(acc, 0.0))], SpreadMode::Pad, Transform::identity()) {
            if let Some(p) = rrect_path(SIDE, 0.0, SW - SIDE, 420.0, [0.0; 4]) { cv.fill_shader(&p, g); }
        }
        // sidebar
        cv.rrect(0.0, 0.0, SIDE, SH, 0.0, SIDEBG, 1.0);
        cv.rrect(SIDE, 0.0, 1.0, SH, 0.0, 0x232327, 1.0);
        cv.rrect(26.0, 40.0, 40.0, 40.0, 11.0, 0xdfe9fb, 1.0);
        cv.icon("dog", icons::DOG, 29.0, 42.0, 34.0, 0xffffff, 1.0);
        cv.text(76.0, 58.0, "Pochi", 18.0, W::Bold, WHITE, 1.0);
        cv.text(76.0, 76.0, &format!("Version {}", env!("CARGO_PKG_VERSION")), 12.0, W::Regular, MUTED, 1.0);
        // close dot
        let hov = inside((12.0, 12.0, 20.0, 20.0), self.mouse);
        cv.circle(22.0, 22.0, 7.0, 0xff5f57, 1.0);
        if hov { cv.text_center(22.0, 26.5, "×", 12.0, W::Bold, 0x6b0b05, 1.0); }
        self.hits.push(((10.0, 10.0, 24.0, 24.0), SAct::Close));
        let mut y = 100.0;
        for (id, label, ic, color) in SECTIONS {
            self.nav_item(cv, y, id, label, ic, color);
            y += 46.0;
        }
        self.nav_item(cv, SH - 64.0, "about", "About", "info", 0x636366);

        // content (scrolling, clipped below the title strip)
        let ck = cv.s;
        if self.clip.as_ref().map(|c| c.0 != ck).unwrap_or(true) {
            let mut m = Mask::new(cv.pm.width(), cv.pm.height()).unwrap();
            if let Some(p) = rrect_path(SIDE + 1.0, TOP - 6.0, SW - SIDE, SH - TOP + 6.0, [0.0; 4]) {
                m.fill_path(&p, FillRule::Winding, false, Transform::from_scale(cv.s, cv.s));
            }
            self.clip = Some((ck, m));
        }
        cv.mask = self.clip.as_ref().map(|c| c.1.clone());
        cv.oy = -self.scroll;
        self.y = TOP + 16.0;
        match self.section {
            "general" => self.general(app, cv),
            "display" => self.display(app, cv),
            "appearance" => self.appearance(app, cv),
            "media" => self.media(app, cv),
            "todo" => self.todo(app, cv),
            "calendar" => self.calendar(app, cv),
            "pomodoro" => self.pomodoro(app, cv),
            "water" => self.water(app, cv),
            "hud" => self.hud(app, cv),
            _ => self.about(app, cv),
        }
        self.content_h = self.y - TOP + 30.0;
        cv.oy = 0.0;
        cv.mask = None;
        // scrollbar
        let view = SH - TOP - 20.0;
        if self.content_h > view {
            let h = (view / self.content_h * view).max(30.0);
            let t = self.scroll / (self.content_h - view);
            cv.rrect(SW - 8.0, TOP + 4.0 + t * (view - h), 4.0, h, 2.0, 0x48484c, 1.0);
        }
    }

    fn nav_item(&mut self, cv: &mut Canvas, y: f32, id: &'static str, label: &str, ic: &str, color: u32) {
        let on = self.section == id;
        let hov = inside((14.0, y, SIDE - 28.0, 42.0), self.mouse);
        let acc = accent_of(id);
        if on {
            cv.rrect(14.0, y, SIDE - 28.0, 42.0, 11.0, blend(SIDEBG, acc, 0.16), 1.0);
            cv.rrect(14.0, y + 11.0, 3.0, 20.0, 1.5, acc, 1.0);
        } else if hov { cv.rrect(14.0, y, SIDE - 28.0, 42.0, 11.0, 0x222226, 1.0); }
        let _ = color;
        if let Some(g) = LinearGradient::new(Point::from_xy(0.0, (y + 6.0) * cv.s), Point::from_xy(0.0, (y + 36.0) * cv.s),
            vec![GradientStop::new(0.0, rgb(blend(acc, 0xffffff, 0.18), 1.0)), GradientStop::new(1.0, rgb(blend(acc, 0x000000, 0.12), 1.0))], SpreadMode::Pad, Transform::identity()) {
            if let Some(p) = rrect_path(26.0, y + 6.0, 30.0, 30.0, [8.0; 4]) { cv.fill_shader(&p, g); }
        }
        let (n, svg) = icon_svg(ic);
        cv.icon(n, svg, 32.0, y + 12.0, 18.0, 0xffffff, 1.0);
        cv.text(70.0, y + 26.5, label, 15.0, if on { W::Semi } else { W::Regular }, if on { 0xffffff } else { 0xd8d8de }, 1.0);
        self.hits.push(((14.0, y, SIDE - 28.0, 42.0), SAct::Nav(id)));
    }

    /* ---------- building blocks ---------- */

    fn h1(&mut self, cv: &mut Canvas, t: &str, lead: &str) {
        let acc = self.accent();
        let ic = SECTIONS.iter().find(|x| x.0 == self.section).map(|x| x.2).unwrap_or("info");
        let (n, svg) = icon_svg(ic);
        let top = self.y + 6.0;
        if let Some(g) = LinearGradient::new(Point::from_xy(CX0 * cv.s, (top + cv.oy) * cv.s), Point::from_xy((CX0 + 56.0) * cv.s, (top + 56.0 + cv.oy) * cv.s),
            vec![GradientStop::new(0.0, rgb(blend(acc, 0xffffff, 0.25), 1.0)), GradientStop::new(1.0, rgb(blend(acc, 0x000000, 0.25), 1.0))], SpreadMode::Pad, Transform::identity()) {
            if let Some(p) = rrect_path(CX0 - 3.0, top - 3.0 + 6.0, 62.0, 62.0, [18.0; 4]) { cv.fill(&p, acc, 0.18); }
            if let Some(p) = rrect_path(CX0, top, 56.0, 56.0, [16.0; 4]) { cv.fill_shader(&p, g); }
        }
        cv.icon(n, svg, CX0 + 14.0, top + 14.0, 28.0, 0xffffff, 1.0);
        let tx = CX0 + 74.0;
        let lines = if lead.is_empty() { vec![] } else { wrap(cv, lead, 14.0, CW - 74.0) };
        let th = 30.0 + lines.len() as f32 * 20.0;
        let ty = top + (56.0 - th).max(0.0) / 2.0;
        cv.text(tx, ty + 26.0, t, 26.0, W::Bold, WHITE, 1.0);
        let mut ly = ty + 30.0;
        for line in lines { cv.text(tx, ly + 15.0, &line, 14.0, W::Regular, MUTED, 1.0); ly += 20.0; }
        self.y = (top + 56.0).max(ly) + 10.0;
    }

    fn h2(&mut self, cv: &mut Canvas, t: &str) {
        self.y += 16.0;
        cv.text(CX0 + 4.0, self.y + 12.0, &t.to_uppercase(), 11.5, W::Semi, 0x86868d, 1.0);
        self.y += 22.0;
    }

}

// The card helper above needs the rows twice (measure + draw); closures can't be reused, so the
// sections below call `begin_card` / `row*` / `end_card` instead, which paints the background
// lazily using a remembered rectangle.
impl SettingsUi {
    fn begin_card(&mut self) -> f32 { let t = self.y; t }

    fn end_card(&mut self, cv: &mut Canvas, top: f32, rows: Vec<Row>) {
        // measure
        let mut h = 0.0;
        for r in &rows { h += r.height(cv); }
        card_bg(cv, top, h);
        let mut y = top;
        let n = rows.len();
        for (i, r) in rows.into_iter().enumerate() {
            let rh = r.height(cv);
            self.y = y;
            self.draw_row(cv, r, y, rh);
            if i + 1 < n { cv.rrect(CX0 + 20.0, y + rh - 0.5, CW - 20.0, 1.0, 0.0, LINE, 1.0); }
            y += rh;
        }
        self.y = top + h;
    }
}

pub enum Ctl {
    Switch(String, bool, Action),
    Seg(Vec<(String, bool, SAct)>),
    Stepper(String, SAct, SAct),
    Buttons(Vec<(String, u8, SAct)>), // style: 0 normal, 1 primary, 2 danger
    None,
}

pub struct Row { pub title: String, pub desc: String, pub ctl: Ctl, pub col: bool, pub extra: Option<RowExtra> }
pub enum RowExtra { Dog, Text(String), Radios(Vec<(String, bool, SAct)>) }

impl Row {
    fn new(title: &str, desc: &str, ctl: Ctl) -> Row { Row { title: title.into(), desc: desc.into(), ctl, col: false, extra: None } }
    fn col(title: &str, desc: &str, ctl: Ctl) -> Row { Row { title: title.into(), desc: desc.into(), ctl, col: true, extra: None } }
    fn height(&self, cv: &mut Canvas) -> f32 {
        let desc_lines = if self.desc.is_empty() { 0 } else { wrap(cv, &self.desc, 13.0, CW - 40.0 - if self.col { 0.0 } else { 170.0 }).len() };
        let text_h = if self.title.is_empty() { 0.0 } else { 20.0 + desc_lines as f32 * 18.0 };
        let mut h = if self.col {
            let ctl_h = match &self.ctl { Ctl::Seg(_) => 34.0, Ctl::Buttons(_) => 34.0, Ctl::None => 0.0, _ => 30.0 };
            28.0 + text_h + if text_h > 0.0 && ctl_h > 0.0 { 10.0 } else { 0.0 } + ctl_h
        } else { (28.0 + text_h).max(58.0) };
        if let Some(RowExtra::Radios(r)) = &self.extra { h += r.len() as f32 * 36.0 + 4.0; }
        h
    }
}

fn wrap(cv: &mut Canvas, s: &str, size: f32, max: f32) -> Vec<String> {
    let mut lines = vec![];
    let mut cur = String::new();
    for word in s.split(' ') {
        let t = if cur.is_empty() { word.to_string() } else { format!("{} {}", cur, word) };
        if cv.measure(&t, size, W::Regular) > max && !cur.is_empty() { lines.push(std::mem::take(&mut cur)); cur = word.to_string(); } else { cur = t; }
    }
    if !cur.is_empty() { lines.push(cur); }
    lines
}

impl SettingsUi {
    fn draw_row(&mut self, cv: &mut Canvas, r: Row, y: f32, rh: f32) {
        let x = CX0 + 20.0;
        let mut ty = y + 14.0;
        if !r.title.is_empty() {
            let title_w = if r.col { CW - 40.0 } else { CW - 210.0 };
            let t = cv.ellipsize(&r.title, 15.5, W::Regular, title_w);
            cv.text(x, ty + 16.0, &t, 15.5, W::Regular, WHITE, 1.0);
            ty += 20.0;
            if !r.desc.is_empty() {
                for line in wrap(cv, &r.desc, 13.0, CW - 40.0 - if r.col { 0.0 } else { 170.0 }) {
                    cv.text(x, ty + 15.0, &line, 13.0, W::Regular, MUTED, 1.0);
                    ty += 18.0;
                }
            }
        }
        let right = CX0 + CW - 20.0;
        let cy = if r.col { ty + 10.0 } else { y + rh / 2.0 };
        match r.ctl {
            Ctl::Switch(key, on, act) => {
                let acc = self.accent();
                let k = self.knobs.entry(key).or_insert_with(|| Spring::with(if on { 1.0 } else { 0.0 }, 3.2, 0.72));
                k.set(if on { 1.0 } else { 0.0 });
                let t = clamp01(k.x);
                let sx = right - 54.0;
                cv.rrect(sx, cy - 16.0, 54.0, 32.0, 16.0, blend(0x3a3a3f, acc, t), 1.0);
                cv.circle(sx + 16.0 + 22.0 * k.x, cy, 13.0, 0xffffff, 1.0);
                self.hit(cv, sx, cy - 16.0, 54.0, 32.0, SAct::App(act));
                if let Some(RowExtra::Dog) = r.extra { cv.icon("dog", icons::DOG, sx - 46.0, cy - 18.0, 36.0, 0xffffff, 1.0); }
            }
            Ctl::Seg(opts) => {
                let (sx, sw) = if r.col { (x, CW - 40.0) } else { (right - 300.0, 300.0) };
                let top = if r.col { ty + 10.0 } else { cy - 17.0 };
                cv.rrect(sx, top, sw, 34.0, 10.0, 0x2a2a2f, 1.0);
                let acc = self.accent();
                let n = opts.len() as f32;
                let bw = (sw - 6.0 - (n - 1.0) * 2.0) / n;
                for (i, (label, on, act)) in opts.into_iter().enumerate() {
                    let bx = sx + 3.0 + i as f32 * (bw + 2.0);
                    let hov = self.hov(cv, bx, top + 3.0, bw, 28.0);
                    if on { cv.rrect(bx, top + 3.0, bw, 28.0, 8.0, blend(0x2a2a2f, acc, 0.55), 1.0); } else if hov { cv.rrect(bx, top + 3.0, bw, 28.0, 8.0, 0x3a3a40, 1.0); }
                    let l = cv.ellipsize(&label, 13.5, W::Regular, bw - 8.0);
                    cv.text_center(bx + bw / 2.0, top + 22.0, &l, 13.5, W::Regular, if on { 0xffffff } else { 0xd6d6db }, 1.0);
                    self.hit(cv, bx, top + 3.0, bw, 28.0, act);
                }
            }
            Ctl::Stepper(val, minus, plus) => {
                let vw = cv.measure(&val, 15.0, W::Semi).max(70.0) + 16.0;
                let total = 30.0 + vw + 30.0;
                let sx = if r.col { x } else { right - total };
                let top = if r.col { ty + 10.0 } else { cy - 15.0 };
                for (bx, label, act) in [(sx, "−", minus), (sx + 30.0 + vw, "+", plus)] {
                    let hov = self.hov(cv, bx, top, 30.0, 30.0);
                    cv.circle(bx + 15.0, top + 15.0, 15.0, if hov { blend(0x34343a, self.accent(), 0.35) } else { 0x34343a }, 1.0);
                    cv.text_center(bx + 15.0, top + 20.5, label, 16.0, W::Bold, WHITE, 1.0);
                    self.hit(cv, bx, top, 30.0, 30.0, act);
                }
                cv.text_center(sx + 30.0 + vw / 2.0, top + 20.5, &val, 15.0, W::Semi, WHITE, 1.0);
            }
            Ctl::Buttons(bs) => {
                let mut bx = x;
                let top = if r.col { ty + 10.0 } else { cy - 17.0 };
                if !r.col {
                    let total: f32 = bs.iter().map(|b| cv.measure(&b.0, 14.0, W::Regular) + 28.0 + 8.0).sum::<f32>() - 8.0;
                    bx = right - total;
                }
                for (label, style, act) in bs {
                    let bw = cv.measure(&label, 14.0, W::Regular) + 28.0;
                    let hov = self.hov(cv, bx, top, bw, 34.0);
                    let bg = match style { 1 => if hov { blend(self.accent(), 0xffffff, 0.12) } else { self.accent() }, _ => if hov { 0x48484d } else { 0x3a3a3e } };
                    cv.rrect(bx, top, bw, 34.0, 8.0, bg, 1.0);
                    cv.text_center(bx + bw / 2.0, top + 22.0, &label, 14.0, W::Regular, if style == 2 { RED } else { 0xffffff }, 1.0);
                    self.hit(cv, bx, top, bw, 34.0, act);
                    bx += bw + 8.0;
                }
            }
            Ctl::None => {}
        }
        if let Some(RowExtra::Text(t)) = &r.extra {
            let tw = cv.measure(t, 13.0, W::Regular);
            cv.text(right - tw, cy + 5.0, t, 13.0, W::Regular, MUTED, 1.0);
        }
        if let Some(RowExtra::Radios(list)) = r.extra {
            let mut ry = ty + 6.0;
            for (label, on, act) in list {
                let hov = self.hov(cv, x - 6.0, ry, CW - 28.0, 34.0);
                if hov { cv.rrect(x - 6.0, ry, CW - 28.0, 34.0, 8.0, 0x333336, 1.0); }
                if on { cv.circle(x + 9.0, ry + 17.0, 9.0, self.accent(), 1.0); cv.circle(x + 9.0, ry + 17.0, 3.5, 0xffffff, 1.0); }
                else { cv.arc(x + 9.0, ry + 17.0, 8.0, 0.0, 360.0, 0x6a6a6f, 1.0, 2.0); }
                cv.text(x + 28.0, ry + 22.5, &label, 14.5, W::Regular, WHITE, 1.0);
                self.hit(cv, x - 6.0, ry, CW - 28.0, 34.0, act);
                ry += 36.0;
            }
        }
    }

    fn page_toggle(&mut self, cv: &mut Canvas, app: &App, id: &'static str) {
        self.h2(cv, "Notch");
        let on = app.d.settings.pages.get(id);
        let top = self.begin_card();
        self.end_card(cv, top, vec![Row::new("Show on notch", "Turn off to hide this page from the notch", Ctl::Switch(format!("page-{}", id), on, Action::SetPage(id, !on)))]);
    }

    /* ---------- sections ---------- */

    fn general(&mut self, app: &App, cv: &mut Canvas) {
        let s = &app.d.settings;
        self.h1(cv, "General", "");
        self.h2(cv, "Quick setup");
        let presets = Ctl::Seg(vec![
            ("Focus".into(), false, SAct::App(Action::SetPreset("focus"))),
            ("Work".into(), false, SAct::App(Action::SetPreset("work"))),
            ("Minimal".into(), false, SAct::App(Action::SetPreset("minimal"))),
        ]);
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::col("Presets", "Change which modules are shown and how Pochi behaves. Your tasks, events, and history stay.", presets)]);
        self.h2(cv, "Startup");
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::new("Start with Windows", "Starts Pochi for your Windows account at sign-in; no administrator access is needed.", sw("sww", s.start_with_windows, "startWithWindows"))]);
        self.h2(cv, "Notch");
        let cur = s.size.as_u64().map(|v| v as u32);
        let size = Ctl::Seg([(None, "Compact"), (Some(100), "100%"), (Some(115), "115%"), (Some(130), "130%"), (Some(150), "150%")].iter()
            .map(|(v, l)| (l.to_string(), cur == *v, SAct::App(Action::SetSize(*v)))).collect());
        let idle = Ctl::Seg([(0, "Off"), (60, "Every 1 min"), (300, "Every 5 min"), (900, "Every 15 min")].iter()
            .map(|(v, l)| (l.to_string(), s.idle_after == *v, SAct::App(Action::SetIdle(*v)))).collect());
        let mut dog = Row::new("Pochi mascot", "A little friend that lives in the notch", sw("mascot", s.mascot, "mascot"));
        dog.extra = Some(RowExtra::Dog);
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::col("Size", "Applies instantly, no restart needed", size),
            Row::new("Minimal notch", "A smaller pill when nothing is playing", sw("minimal", s.minimal, "minimal")),
            Row::new("Auto-hide", "Shrink to a thin line at the top edge; hover the top of the screen to bring it back", sw("autohide", s.auto_hide, "autoHide")),
            Row::new("Hide notch", "Hide it completely. Bring it back from the tray icon or with Ctrl + Alt + P", sw("hidden", s.hidden, "hidden")),
            dog,
            Row::col("Pochi says hi", "Now and then Pochi pops up and waves. Hover: its eyes follow you · click: pet it · double-click: open", idle),
        ]);
        self.h2(cv, "Sounds");
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::new("Reminder sounds", "Chimes for Pomodoro, To-do and Water", sw("sounds", s.sounds, "sounds"))]);
        self.h2(cv, "Full screen");
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::new("Hide in full screen", "Games, videos and slides that fill the screen", sw("fs", s.hide_fullscreen, "hideFullscreen"))]);
        self.h2(cv, "Restore");
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::new("Reset preferences", "Restore default settings. To-dos, events, water logs, and focus history are kept.", Ctl::Buttons(vec![
            ("Reset settings".into(), 2, SAct::App(Action::ResetSettings)),
        ]))]);
    }

    fn display(&mut self, app: &App, cv: &mut Canvas) {
        let s = &app.d.settings;
        self.h1(cv, "Display", "");
        self.h2(cv, "Screen");
        let radios: Vec<(String, bool, SAct)> = self.monitors.iter().map(|m| (m.label.clone(), m.selected, SAct::Display(m.id.clone()))).collect();
        let mut r = Row::col("Show the notch on", "", Ctl::None);
        r.extra = Some(RowExtra::Radios(radios));
        let t = self.begin_card();
        self.end_card(cv, t, vec![r]);
        self.h2(cv, "Interaction");
        let delay = Ctl::Seg([(200, "Instantly"), (400, "0.4 s"), (1000, "1 s"), (2000, "2 s")].iter()
            .map(|(v, l)| (l.to_string(), s.collapse_delay == *v, SAct::App(Action::SetCollapse(*v)))).collect());
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::new("Expand on hover", "Turn off to expand only when you click the notch", sw("hover", s.hover_expand, "hoverExpand")),
            Row::col("Collapse after the mouse leaves", "", delay),
        ]);
        self.h2(cv, "Notch pages");
        let rows = [("home", "Home & quick controls"), ("notifications", "Notifications"), ("media", "Music"), ("calendar", "Calendar"), ("water", "Water"), ("todo", "To-do"), ("pomodoro", "Pomodoro")].iter()
            .map(|(id, l)| { let on = s.pages.get(id); Row::new(l, "", Ctl::Switch(format!("page-{}", id), on, Action::SetPage(id, !on))) }).collect();
        let t = self.begin_card();
        self.end_card(cv, t, rows);
    }

    fn appearance(&mut self, app: &App, cv: &mut Canvas) {
        let s = &app.d.settings;
        self.h1(cv, "Appearance", "");
        self.h2(cv, "Notch");
        let colors = Ctl::Seg(vec![
            ("Midnight".into(), s.capsule_color == "midnight", SAct::App(Action::SetCapsuleColor("midnight"))),
            ("Slate".into(), s.capsule_color == "slate", SAct::App(Action::SetCapsuleColor("slate"))),
            ("Plum".into(), s.capsule_color == "plum", SAct::App(Action::SetCapsuleColor("plum"))),
            ("Ocean".into(), s.capsule_color == "ocean", SAct::App(Action::SetCapsuleColor("ocean"))),
        ]);
        let opacity = Ctl::Seg([70, 85, 100].iter().map(|v| (
            format!("{}%", v), s.opacity == *v,
            SAct::App(Action::SetOpacity(*v)),
        )).collect());
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::col("Capsule color", "Choose the notch body tint; page colors still identify each module.", colors),
            Row::col("Transparency", "Lower opacity makes the whole notch less prominent.", opacity),
            Row::new("Reduce motion", "Stops page and pill transitions and automatic Pochi cameos.", sw("reduced-motion", s.reduced_motion, "reducedMotion")),
        ]);
        self.h2(cv, "Clock");
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::col("Time format", "", Ctl::Seg(vec![
            ("12-hour (02:25 PM)".into(), !s.clock24, SAct::App(Action::SetBool("clock24", false))),
            ("24-hour (14:25)".into(), s.clock24, SAct::App(Action::SetBool("clock24", true))),
        ]))]);
        self.h2(cv, "Header");
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::new("Show date", "e.g. Tue 27 Oct 2026", sw("date", s.show_date, "showDate")),
            Row::new("Show battery", "Charging icon or battery percentage", sw("batt", s.show_battery, "showBattery")),
        ]);
    }

    fn media(&mut self, app: &App, cv: &mut Canvas) {
        let s = &app.d.settings;
        let m = &app.media;
        self.h1(cv, "Music and lyrics", "Control whatever is playing on Windows (Spotify, YouTube, browsers, Media Player…) right from the notch.");
        self.h2(cv, "Now playing");
        let (title, desc) = if m.ok { (if m.title.is_empty() { "Unknown title".to_string() } else { m.title.clone() }, format!("{}{}", m.artist, if m.playing { " · playing" } else { " · paused" })) }
            else { ("Nothing playing".into(), "Start music in any app to see it here".into()) };
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::new(&title, &desc, Ctl::None)]);
        self.h2(cv, "Display");
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::new("Show on the collapsed notch", "Tiny artwork and an equalizer while playing", sw("mpill", s.media_pill, "mediaPill")),
            Row::new("Pop open on track change", "The notch expands for a few seconds when a new song starts", sw("mpeek", s.media_peek, "mediaPeek")),
            Row::new("Lyrics", "Fetch synced lyrics from lrclib.net and follow along", sw("lyr", s.lyrics, "lyrics")),
        ]);
        self.page_toggle(cv, app, "media");
    }

    fn text_field(&mut self, cv: &mut Canvas, x: f32, y: f32, w: f32, f: Field, placeholder: &str) {
        let text = if f == Field::TodoText { self.todo_text.clone() } else { self.ev_title.clone() };
        let focused = self.focus == Some(f);
        cv.rrect(x, y, w, 36.0, 8.0, 0x2a2a2f, 1.0);
        if focused { let a = self.accent(); if let Some(p) = rrect_path(x, y, w, 36.0, [8.0; 4]) { cv.stroke(&p, a, 1.0, 1.5); } }
        let caret = focused && (now_ms() / 530) % 2 == 0;
        if text.is_empty() {
            cv.text(x + 12.0, y + 23.5, placeholder, 14.0, W::Regular, 0x7c7c82, 1.0);
            if caret { cv.rrect(x + 11.0, y + 9.0, 1.5, 18.0, 0.5, WHITE, 1.0); }
        } else {
            let mut shown = text.clone();
            while cv.measure(&shown, 14.0, W::Regular) > w - 26.0 && !shown.is_empty() { shown.remove(0); }
            let tw = cv.text(x + 12.0, y + 23.5, &shown, 14.0, W::Regular, WHITE, 1.0);
            if caret { cv.rrect(x + 13.0 + tw, y + 9.0, 1.5, 18.0, 0.5, WHITE, 1.0); }
        }
        self.hit(cv, x, y, w, 36.0, SAct::Field(f));
    }

    fn mini_stepper(&mut self, cv: &mut Canvas, x: f32, y: f32, label: &str, minus: SAct, plus: SAct) -> f32 {
        let vw = cv.measure(label, 13.5, W::Semi).max(44.0) + 12.0;
        for (bx, l, act) in [(x, "−", minus), (x + 26.0 + vw, "+", plus)] {
            let hov = self.hov(cv, bx, y + 5.0, 26.0, 26.0);
            cv.circle(bx + 13.0, y + 18.0, 13.0, if hov { 0x55555a } else { 0x3a3a3e }, 1.0);
            cv.text_center(bx + 13.0, y + 23.0, l, 15.0, W::Bold, WHITE, 1.0);
            self.hit(cv, bx, y + 5.0, 26.0, 26.0, act);
        }
        cv.text_center(x + 26.0 + vw / 2.0, y + 23.0, label, 13.5, W::Semi, WHITE, 1.0);
        52.0 + vw
    }

    fn button(&mut self, cv: &mut Canvas, x: f32, y: f32, label: &str, style: u8, act: SAct) -> f32 {
        let bw = cv.measure(label, 14.0, W::Regular) + 28.0;
        let hov = self.hov(cv, x, y, bw, 36.0);
        let bg = match style { 1 => if hov { blend(self.accent(), 0xffffff, 0.12) } else { self.accent() }, 3 => if hov { 0x48d26c } else { GREEN }, _ => if hov { 0x48484d } else { 0x3a3a3e } };
        cv.rrect(x, y, bw, 36.0, 8.0, bg, 1.0);
        cv.text_center(x + bw / 2.0, y + 23.0, label, 14.0, W::Regular, if style == 2 { RED } else { 0xffffff }, 1.0);
        self.hit(cv, x, y, bw, 36.0, act);
        bw
    }

    fn list_card(&mut self, cv: &mut Canvas, rows: Vec<(bool, Option<String>, String, String, Option<SAct>, SAct)>) {
        // (done, tag, text, meta, toggle, delete)
        let top = self.y;
        let n = rows.len().max(1);
        let h = n as f32 * 50.0;
        card_bg(cv, top, h);
        if rows.is_empty() { cv.text(CX0 + 20.0, top + 31.0, "Nothing here yet.", 14.0, W::Regular, MUTED, 1.0); }
        let len = rows.len();
        for (i, (done, tag, text, meta, toggle, del)) in rows.into_iter().enumerate() {
            let y = top + i as f32 * 50.0;
            let mut x = CX0 + 20.0;
            if let Some(tg) = toggle {
                if done { cv.circle(x + 11.0, y + 25.0, 11.0, GREEN, 1.0); cv.icon("check", icons::CHECK, x + 4.5, y + 18.5, 13.0, 0x000000, 1.0); }
                else { cv.arc(x + 11.0, y + 25.0, 10.0, 0.0, 360.0, 0x66666b, 1.0, 2.0); }
                self.hit(cv, x - 4.0, y + 10.0, 30.0, 30.0, tg);
                x += 34.0;
            }
            let right = CX0 + CW - 20.0;
            let hov = self.hov(cv, right - 32.0, y + 9.0, 32.0, 32.0);
            if hov { cv.rrect(right - 32.0, y + 9.0, 32.0, 32.0, 8.0, 0x3a3a3e, 1.0); }
            cv.icon("trash", icons::TRASH, right - 25.0, y + 16.0, 18.0, if hov { RED } else { MUTED }, 1.0);
            self.hit(cv, right - 32.0, y + 9.0, 32.0, 32.0, del);
            let mut rx = right - 44.0;
            if !meta.is_empty() { let mw = cv.measure(&meta, 12.5, W::Regular); cv.text(rx - mw, y + 29.5, &meta, 12.5, W::Regular, MUTED, 1.0); rx -= mw + 10.0; }
            if let Some(tg) = tag {
                let tw = cv.measure(&tg, 11.5, W::Regular) + 16.0;
                cv.rrect(rx - tw, y + 15.0, tw, 20.0, 10.0, 0x0f3346, 1.0);
                cv.text(rx - tw + 8.0, y + 29.0, &tg, 11.5, W::Regular, 0x64d2ff, 1.0);
                rx -= tw + 10.0;
            }
            let t = cv.ellipsize(&text, 14.5, W::Regular, rx - x);
            let tw = cv.text(x, y + 30.0, &t, 14.5, W::Regular, if done { MUTED } else { WHITE }, 1.0);
            if done { cv.rrect(x, y + 25.0, tw, 1.0, 0.0, MUTED, 1.0); }
            if i + 1 < len { cv.rrect(CX0 + 20.0, y + 49.5, CW - 20.0, 1.0, 0.0, LINE, 1.0); }
        }
        self.y = top + h;
    }

    fn todo(&mut self, app: &App, cv: &mut Canvas) {
        self.h1(cv, "To-do", "Add tasks here or right on the notch. Set a time to get reminded; “Daily” tasks reset every morning.");
        // add form
        let top = self.y;
        card_bg(cv, top, 64.0);
        let fx = CX0 + 14.0;
        let tlabel = self.todo_time.map(|(h, m)| format!("{:02}:{:02}", h, m)).unwrap_or_else(|| "No time".into());
        let right = CX0 + CW - 14.0;
        let add_w = cv.measure("+ Add", 14.0, W::Regular) + 28.0;
        let daily_w = cv.measure("Daily", 13.0, W::Regular) + 24.0;
        let step_w = 52.0 + cv.measure(&tlabel, 13.5, W::Semi).max(44.0) + 12.0;
        let field_w = right - fx - add_w - daily_w - step_w - 30.0;
        self.text_field(cv, fx, top + 14.0, field_w, Field::TodoText, "Task title…");
        let mut x = fx + field_w + 10.0;
        x += self.mini_stepper(cv, x, top + 14.0, &tlabel, SAct::TodoTime(-1), SAct::TodoTime(1)) + 10.0;
        let on = self.todo_daily;
        let hov = self.hov(cv, x, top + 19.0, daily_w, 26.0);
        cv.rrect(x, top + 19.0, daily_w, 26.0, 13.0, if on { self.accent() } else if hov { 0x48484d } else { 0x3a3a3e }, 1.0);
        cv.text_center(x + daily_w / 2.0, top + 36.5, "Daily", 13.0, W::Regular, WHITE, 1.0);
        self.hit(cv, x, top + 19.0, daily_w, 26.0, SAct::TodoDaily);
        self.button(cv, right - add_w, top + 14.0, "+ Add", 1, SAct::TodoAdd);
        self.y = top + 64.0;

        let open: Vec<&crate::data::Todo> = app.d.todos.iter().filter(|t| !t.done).collect();
        let done: Vec<&crate::data::Todo> = app.d.todos.iter().filter(|t| t.done).collect();
        let mk = |t: &crate::data::Todo| {
            let meta = t.remind_at.as_deref().and_then(time::parse_iso).map(|r| {
                if t.daily { time::hm24(r) } else {
                    let k = time::day_key(r);
                    let day = if k == time::day_key(now_ms()) { "Today".to_string() } else if k == time::day_key(now_ms() + 86_400_000) { "Tomorrow".into() } else { let l = time::local(r); format!("{} {}", l.day, time::MONTHS[l.month as usize - 1]) };
                    format!("{} {}", day, time::hm24(r))
                }
            }).unwrap_or_default();
            (t.done, t.daily.then(|| "Daily".to_string()), t.text.clone(), meta, Some(SAct::App(Action::TodoToggle(t.id.clone()))), SAct::App(Action::TodoDelete(t.id.clone())))
        };
        self.h2(cv, &format!("In progress ({})", open.len()));
        let rows = open.iter().map(|t| mk(t)).collect();
        self.list_card(cv, rows);
        if !done.is_empty() {
            self.y += 14.0;
            cv.text(CX0 + 6.0, self.y + 13.0, &format!("Done ({})", done.len()), 14.0, W::Regular, MUTED, 1.0);
            let bw = cv.measure("Clear completed", 13.0, W::Regular) + 20.0;
            let hov = self.hov(cv, CX0 + CW - bw, self.y - 6.0, bw, 28.0);
            cv.rrect(CX0 + CW - bw, self.y - 6.0, bw, 28.0, 7.0, if hov { 0x48484d } else { 0x3a3a3e }, 1.0);
            cv.text_center(CX0 + CW - bw / 2.0, self.y + 12.5, "Clear completed", 13.0, W::Regular, RED, 1.0);
            self.hit(cv, CX0 + CW - bw, self.y - 6.0, bw, 28.0, SAct::App(Action::TodoClearDone));
            self.y += 30.0;
            let rows = done.iter().map(|t| mk(t)).collect();
            self.list_card(cv, rows);
        }
        self.page_toggle(cv, app, "todo");
    }

    fn calendar(&mut self, app: &App, cv: &mut Canvas) {
        let s = &app.d.settings;
        self.h1(cv, "Calendar", "Upcoming events show on the notch Calendar page and you get reminded before they start.");
        let top = self.y;
        card_bg(cv, top, 64.0);
        let fx = CX0 + 14.0;
        let right = CX0 + CW - 14.0;
        let day = match self.ev_day { 0 => "Today".to_string(), 1 => "Tomorrow".into(), d => {
            let l = time::local(time::start_of_local_day(now_ms()) + d as i64 * 86_400_000 + 43_200_000);
            format!("{} {} {}", time::WD[l.weekday as usize], l.day, time::MONTHS[l.month as usize - 1])
        } };
        let tlabel = self.ev_time.map(|(h, m)| format!("{:02}:{:02}", h, m)).unwrap_or_else(|| "All day".into());
        let add_w = cv.measure("+ Add", 14.0, W::Regular) + 28.0;
        let s1 = 52.0 + cv.measure(&day, 13.5, W::Semi).max(44.0) + 12.0;
        let s2 = 52.0 + cv.measure(&tlabel, 13.5, W::Semi).max(44.0) + 12.0;
        let field_w = right - fx - add_w - s1 - s2 - 30.0;
        self.text_field(cv, fx, top + 14.0, field_w, Field::EventTitle, "Event title…");
        let mut x = fx + field_w + 10.0;
        x += self.mini_stepper(cv, x, top + 14.0, &day, SAct::EventDay(-1), SAct::EventDay(1)) + 10.0;
        self.mini_stepper(cv, x, top + 14.0, &tlabel, SAct::EventTime(-1), SAct::EventTime(1));
        self.button(cv, right - add_w, top + 14.0, "+ Add", 1, SAct::EventAdd);
        self.y = top + 64.0;
        self.h2(cv, "Remind me");
        let lead = Ctl::Seg([(0, "At start"), (5, "5 min before"), (10, "10 min"), (15, "15 min"), (30, "30 min")].iter()
            .map(|(v, l)| (l.to_string(), s.event_lead == *v, SAct::App(Action::SetEventLead(*v)))).collect());
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::col("", "", lead)]);
        let today = time::day_key(now_ms());
        let upcoming: Vec<&crate::data::Event> = app.d.events.iter().filter(|e| e.date >= today).collect();
        let past = app.d.events.len() - upcoming.len();
        self.y += 14.0;
        cv.text(CX0 + 6.0, self.y + 13.0, "Upcoming", 14.0, W::Regular, MUTED, 1.0);
        if past > 0 {
            let label = format!("Remove {} past", past);
            let bw = cv.measure(&label, 13.0, W::Regular) + 20.0;
            let hov = self.hov(cv, CX0 + CW - bw, self.y - 6.0, bw, 28.0);
            cv.rrect(CX0 + CW - bw, self.y - 6.0, bw, 28.0, 7.0, if hov { 0x48484d } else { 0x3a3a3e }, 1.0);
            cv.text_center(CX0 + CW - bw / 2.0, self.y + 12.5, &label, 13.0, W::Regular, RED, 1.0);
            self.hit(cv, CX0 + CW - bw, self.y - 6.0, bw, 28.0, SAct::App(Action::EventClearPast));
        }
        self.y += 30.0;
        let rows = upcoming.iter().map(|e| {
            let d = time::parse_day_key(&e.date).map(|(y, m, d)| {
                if e.date == today { "Today".to_string() } else { format!("{}, {} {} {}", time::WD[time::weekday_of(y, m, d) as usize], d, time::MONTHS[m as usize - 1], y) }
            }).unwrap_or_default();
            let meta = format!("{} · {}", d, if e.time.is_empty() { "All day".to_string() } else { e.time.clone() });
            (false, None, e.title.clone(), meta, None, SAct::App(Action::EventDelete(e.id.clone())))
        }).collect();
        self.list_card(cv, rows);
        self.page_toggle(cv, app, "calendar");
    }

    fn bars(&mut self, cv: &mut Canvas, vals: Vec<(String, f32, String, bool)>, color: u32) {
        let top = self.y;
        card_bg(cv, top, 170.0);
        let max = vals.iter().map(|v| v.1).fold(1.0f32, f32::max);
        let n = vals.len() as f32;
        let cw = (CW - 40.0) / n;
        for (i, (x_label, v, label, goal)) in vals.into_iter().enumerate() {
            let cx = CX0 + 20.0 + cw * (i as f32 + 0.5);
            let h = (v / max) * 100.0;
            let key = format!("bar{}{}", self.section, i);
            let k = self.knobs.entry(key).or_insert_with(|| { let mut s = Spring::with(0.0, 1.6, 0.8); s.set(h); s });
            k.set(h);
            let hh = k.x.max(3.0);
            cv.rrect(cx - 19.0, top + 130.0 - hh, 38.0, hh, 6.0, if goal { GREEN } else { color }, 1.0);
            cv.text_center(cx, top + 122.0 - hh, &label, 11.5, W::Regular, 0xcfcfd4, 1.0);
            cv.text_center(cx, top + 152.0, &x_label, 12.0, W::Regular, MUTED, 1.0);
        }
        self.y = top + 170.0;
    }

    fn pomodoro(&mut self, app: &App, cv: &mut Canvas) {
        let pc = &app.d.pomodoro;
        let p = &app.pomo;
        self.h1(cv, "Pomodoro", "Work in focused sessions with short breaks in between.");
        self.h2(cv, "Current");
        let top = self.y;
        card_bg(cv, top, 112.0);
        let tw = cv.text(CX0 + 20.0, top + 44.0, &time::mmss(app.pomo_remaining()), 30.0, W::Bold, WHITE, 1.0);
        cv.text(CX0 + 30.0 + tw, top + 43.0, &format!("{} · round {}/{} · {}", p.phase.label(), p.round, pc.rounds, if p.running { "running" } else { "paused" }), 14.0, W::Regular, MUTED, 1.0);
        let mut x = CX0 + 20.0;
        let by = top + 60.0;
        x += self.button(cv, x, by, if p.running { "Pause" } else { "Start" }, 1, SAct::App(Action::PomoToggle)) + 8.0;
        x += self.button(cv, x, by, "Skip", 0, SAct::App(Action::PomoSkip)) + 8.0;
        self.button(cv, x, by, "Reset", 0, SAct::App(Action::PomoResetAll));
        self.y = top + 112.0;
        self.h2(cv, "Durations");
        let st = |v: i32, unit: &str, w: u8, step: i32, lo: i32, hi: i32| Ctl::Stepper(format!("{} {}", v, unit), SAct::App(Action::PomoSet(w, (v - step).max(lo))), SAct::App(Action::PomoSet(w, (v + step).min(hi))));
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::new("Focus session", "", st(pc.focus, "min", 0, 5, 5, 180)),
            Row::new("Short break", "", st(pc.short, "min", 1, 1, 1, 60)),
            Row::new("Long break", "", st(pc.long, "min", 2, 5, 5, 120)),
            Row::new("Sessions before a long break", "", st(pc.rounds, "", 3, 1, 1, 12)),
            Row::new("Auto-start the next session", "", Ctl::Switch("autostart".into(), pc.auto_start, Action::PomoSet(4, if pc.auto_start { 0 } else { 1 }))),
        ]);
        self.h2(cv, &format!("Last 7 days · {} today", app.pomo_today()));
        let vals = (0..7).rev().map(|i| {
            let t = now_ms() - i * 86_400_000;
            let n = *app.d.pomodoro.stats.get(&time::day_key(t)).unwrap_or(&0);
            (time::WD[time::local(t).weekday as usize].to_string(), n as f32, if n > 0 { n.to_string() } else { String::new() }, false)
        }).collect();
        self.bars(cv, vals, 0xff7a45);
        self.page_toggle(cv, app, "pomodoro");
    }

    fn water(&mut self, app: &App, cv: &mut Canvas) {
        let w = &app.d.water;
        let s = &app.d.settings;
        let today = app.water_today();
        let pct = (today as f32 / w.goal as f32 * 100.0).round() as i32;
        self.h1(cv, "Water", "Pochi reminds you to drink water regularly throughout the day.");
        self.h2(cv, "Today");
        let top = self.y;
        let entries: Vec<String> = app.today_entries().iter().rev().map(|e| format!("{} · {} ml", time::hm24(e.t), e.ml)).collect();
        let pill_rows = if entries.is_empty() { 0.0 } else { 34.0 };
        let ch = 132.0 + pill_rows;
        card_bg(cv, top, ch);
        let tw = cv.text(CX0 + 20.0, top + 42.0, &format!("{} ml", today), 30.0, W::Bold, WHITE, 1.0);
        cv.text(CX0 + 28.0 + tw, top + 41.0, &format!("/ {} ml · {}%", w.goal, pct), 14.0, W::Regular, MUTED, 1.0);
        cv.rrect(CX0 + 20.0, top + 56.0, CW - 40.0, 10.0, 5.0, 0x1d2b3a, 1.0);
        cv.rrect(CX0 + 20.0, top + 56.0, (CW - 40.0) * clamp01(pct as f32 / 100.0), 10.0, 5.0, 0x4cc2ff, 1.0);
        let mut y = top + 78.0;
        if !entries.is_empty() {
            let mut x = CX0 + 20.0;
            for e in entries.iter().take(8) {
                let ew = cv.measure(e, 12.5, W::Regular) + 18.0;
                if x + ew > CX0 + CW - 20.0 { break; }
                cv.rrect(x, y, ew, 24.0, 12.0, 0x343438, 1.0);
                cv.text(x + 9.0, y + 16.5, e, 12.5, W::Regular, 0xc7c7cc, 1.0);
                x += ew + 6.0;
            }
            y += 34.0;
        }
        let mut x = CX0 + 20.0;
        x += self.button(cv, x, y, &format!("Drink {} ml", app.water_sip()), 1, SAct::App(Action::Drink)) + 8.0;
        x += self.button(cv, x, y, "Undo", 0, SAct::App(Action::WaterUndo)) + 8.0;
        self.button(cv, x, y, "Reset today", 2, SAct::App(Action::WaterReset));
        self.y = top + ch;
        self.h2(cv, "Reminders");
        let iv = Ctl::Seg([(15, "15 min"), (30, "30 min"), (45, "45 min"), (60, "1 h"), (90, "1.5 h"), (120, "2 h")].iter()
            .map(|(v, l)| (l.to_string(), w.interval == *v, SAct::App(Action::WaterInterval(*v)))).collect());
        let mut hours = Row::new("Active hours", "Pochi stays quiet outside these hours", Ctl::None);
        hours.extra = None;
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::new("Water reminders", &if w.enabled { format!("Next reminder: {}", time::hm24(app.water_next_at)) } else { "Off".into() }, Ctl::Switch("wen".into(), w.enabled, Action::WaterToggle)),
            Row::col("Remind every", "", iv),
            Row::new("Starts at", "Pochi stays quiet outside the active hours", Ctl::Stepper(format!("{:02}:00", w.start_hour), SAct::App(Action::WaterHours(true, -1)), SAct::App(Action::WaterHours(true, 1)))),
            Row::new("Ends at", "", Ctl::Stepper(format!("{:02}:00", w.end_hour % 24), SAct::App(Action::WaterHours(false, -1)), SAct::App(Action::WaterHours(false, 1)))),
        ]);
        let _ = hours;
        self.h2(cv, "Goal");
        let auto = (w.goal as f32 / app.water_reminders() as f32).round() as i32;
        let cup = Ctl::Seg([(0, format!("Auto ({} ml)", auto)), (100, "100 ml".into()), (200, "200 ml".into()), (250, "250 ml".into()), (350, "350 ml".into())].into_iter()
            .map(|(v, l)| (l, w.cup == v, SAct::App(Action::WaterCup(v)))).collect());
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::new("Daily goal", "About 1.5 – 2.5 L is recommended", Ctl::Stepper(format!("{} ml", w.goal), SAct::App(Action::WaterGoalAbs(w.goal - 250)), SAct::App(Action::WaterGoalAbs(w.goal + 250)))),
            Row::new("Bottle size", &format!("{} refills to reach your goal", ((w.goal as f32 / w.bottle as f32).round() as i32).max(1)), Ctl::Stepper(format!("{} ml", w.bottle), SAct::App(Action::WaterBottleAbs(w.bottle - 50)), SAct::App(Action::WaterBottleAbs(w.bottle + 50)))),
            Row::col("Per drink", &format!("{} reminders a day · Drink adds {} ml", app.water_reminders(), app.water_sip()), cup),
        ]);
        self.h2(cv, "Notch chart");
        let t = self.begin_card();
        self.end_card(cv, t, vec![Row::col("Style", "You can also click the chart on the notch to switch", Ctl::Seg(vec![
            ("Percent ring".into(), s.water_view != "chart", SAct::App(Action::WaterView(false))),
            ("Hourly bars".into(), s.water_view == "chart", SAct::App(Action::WaterView(true))),
        ]))]);
        self.h2(cv, "Last 7 days");
        let vals = (0..7).rev().map(|i| {
            let t = now_ms() - i * 86_400_000;
            let ml: i32 = w.log.get(&time::day_key(t)).map(|v| v.iter().map(|e| e.ml).sum()).unwrap_or(0);
            (time::WD[time::local(t).weekday as usize].to_string(), ml as f32, if ml > 0 { crate::ui::liters(ml) } else { String::new() }, ml >= w.goal)
        }).collect();
        self.bars(cv, vals, 0x4cc2ff);
        self.page_toggle(cv, app, "water");
    }

    fn hud(&mut self, app: &App, cv: &mut Canvas) {
        let s = &app.d.settings;
        self.h1(cv, "HUD & popups", "");
        self.h2(cv, "When a reminder fires");
        let keep = Ctl::Seg([(4, "4 s"), (6, "6 s"), (10, "10 s"), (15, "15 s")].iter()
            .map(|(v, l)| (l.to_string(), s.peek_seconds == *v, SAct::App(Action::SetPeekSeconds(*v)))).collect());
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::new("Pop open the notch", "Expand and jump to the right page when a reminder fires", sw("peek", s.peek, "peek")),
            Row::col("Keep it open for", "", keep),
            Row::new("Windows notifications", "Also show a notification in the corner of the screen", sw("notif", s.sys_notif, "sysNotif")),
        ]);
    }

    fn about(&mut self, _app: &App, cv: &mut Canvas) {
        self.h1(cv, "About", "");
        let top = self.y;
        card_bg(cv, top, 120.0);
        cv.rrect(CX0 + 22.0, top + 22.0, 76.0, 76.0, 20.0, 0xdfe9fb, 1.0);
        cv.icon("dog", icons::DOG, CX0 + 30.0, top + 28.0, 62.0, 0xffffff, 1.0);
        cv.text(CX0 + 120.0, top + 48.0, "Pochi", 22.0, W::Bold, WHITE, 1.0);
        cv.text(CX0 + 120.0, top + 70.0, &format!("Version {}", env!("CARGO_PKG_VERSION")), 14.0, W::Regular, MUTED, 1.0);
        cv.text(CX0 + 120.0, top + 94.0, "Native notch for Windows with Pochi the dog. No browser engine inside.", 14.0, W::Regular, WHITE, 1.0);
        self.y = top + 132.0;
        let path = self.data_path.clone();
        let t = self.begin_card();
        self.end_card(cv, t, vec![
            Row::new("Releases and updates", "Open GitHub for the latest release notes and Windows downloads.", Ctl::Buttons(vec![("View releases".into(), 0, SAct::Platform("updates"))])),
            Row::new("Data is stored at", &path, Ctl::Buttons(vec![("Open folder".into(), 0, SAct::Platform("folder"))])),
            Row::new("Exit Pochi", "Quit the app completely (notch and tray icon)", Ctl::Buttons(vec![("Exit".into(), 2, SAct::Platform("exit"))])),
        ]);
    }
}

fn sw(key: &str, on: bool, setting: &'static str) -> Ctl { Ctl::Switch(key.to_string(), on, Action::SetBool(setting, !on)) }

fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| -> f32 { ((a >> s) & 255) as f32 * (1.0 - t) + ((b >> s) & 255) as f32 * t };
    ((ch(16) as u32) << 16) | ((ch(8) as u32) << 8) | ch(0) as u32
}

fn card_bg(cv: &mut Canvas, top: f32, h: f32) {
    cv.rrect(CX0, top, CW, h, 14.0, CARD, 1.0);
    if let Some(p) = rrect_path(CX0 + 0.5, top + 0.5, CW - 1.0, h - 1.0, [13.5; 4]) { cv.stroke(&p, BORDER, 1.0, 1.0); }
}
