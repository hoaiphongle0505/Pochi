//! The notch: state machine, animation and drawing. Platform independent so it can be
//! rendered to PNG on any OS for previews/tests.

use crate::anim::*;
use crate::app::*;
use crate::gfx::*;
use crate::icons;
use crate::time::{self, now_ms};
use tiny_skia::*;

pub const WIN_W: f32 = 648.0;
pub const WIN_H: f32 = 226.0;
const CX: f32 = WIN_W / 2.0;
const EW: f32 = 560.0;
const EH: f32 = 188.0;
const FX: f32 = CX - EW / 2.0;
const PX: f32 = FX + 46.0; // page left
const PY: f32 = 40.0; // page top
const PW: f32 = EW - 92.0; // page width
const PH: f32 = 124.0;

const MUTED: u32 = 0x8e8e93;
const CHIP: u32 = 0x232325;
const CHIP_H: u32 = 0x303033;
const CARD: u32 = 0x18181a;
const LINE: u32 = 0x262629;
const WATER: u32 = 0x4cc2ff;
const GREEN: u32 = 0x34c759;
const WHITE: u32 = 0xffffff;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FieldId { Todo, Event }

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Nav(i32),
    Goto(&'static str),
    App(Action),
    WaterSub,
    WaterEdit(bool),
    WaterIntervalCycle,
    FormOpen(FieldId),
    FormCancel,
    FormSave,
    FormDaily,
    FormTime(i32),
    FormDay(i32),
    Field(FieldId),
    Media(&'static str),
    Seek(f32, f32),
    Lyrics,
    EditTodos,
    Pet,
    AlertAction,
    AlertSnooze,
    AlertOpen,
}

/// what the platform layer should do after an input event
#[derive(Clone, Debug, PartialEq)]
pub enum Out { App(Action), Media(String), WantFocus(bool), Changed }

struct Hit { r: (f32, f32, f32, f32), act: Act }

pub struct Alert { kind: &'static str, page: &'static str, title: String, body: String, t0: f32, until: f32, done_at: f32, snoozed: bool }

#[derive(Default)]
pub struct Form { pub open: bool, pub text: String, pub time: Option<(u32, u32)>, pub daily: bool, pub day: i32 }

struct Cameo {
    on: bool,
    elapsed: f32,
    eyes: Spring,  // 0 = slit, 1 = open
    happy: Spring, // 0..1 crossfade to ^ ^
    paw: Spring,   // 0 hidden .. 1 shown
    overlay: Spring,
    look_x: Spring,
    look_y: Spring,
    small: Spring,
    sleep: bool,
    pet_until: f32,
    clock: f32,
    hearts: Vec<(f32, f32, f32)>,
    alert_until: f32,
}

pub struct Ui {
    pub expanded: bool,
    hovering: bool,
    hover_at: f32,
    leave_at: f32,
    peek_until: f32,
    clock: f32, // seconds since start
    pub mouse: (f32, f32),
    w: Spring,
    h: Spring,
    r: Spring,
    ear: Spring,
    content: Spring,
    pill_alpha: Spring,
    pub page: &'static str,
    prev_page: &'static str,
    page_t: f32, // 0..1 transition
    page_dir: f32,
    water_sub: Spring,
    pub water_edit: bool,
    pub todo_form: Form,
    pub event_form: Form,
    pub focus: Option<FieldId>,
    edit_todos: bool,
    todo_scroll: f32,
    tweens: std::collections::HashMap<String, Spring>,
    hits: Vec<Hit>,
    cameo: Cameo,
    last_active: f32,
    pub battery: Option<(f32, bool)>,
    show_lyrics: bool,
    lyric_line: String,
    lyric_swap: f32,
    pub art: Option<Pixmap>,
    art_key: String,
    art_accent: Option<u32>,
    muted: bool,
    pill_w: f32,
    pub content_rect: (f32, f32, f32, f32),
    tick_flash: f32,
    shadow: Option<(f32, Pixmap)>,
    pub alert: Option<Alert>,
    clip_cache: std::collections::HashMap<String, Mask>,
}

fn accent_of(page: &str) -> u32 {
    match page { "media" => 0xff375f, "calendar" => 0xff5a5f, "water" => WATER, "todo" => GREEN, _ => 0xff453a }
}

fn inside(r: (f32, f32, f32, f32), p: (f32, f32)) -> bool { p.0 >= r.0 && p.0 <= r.0 + r.2 && p.1 >= r.1 && p.1 <= r.1 + r.3 }

impl Ui {
    pub fn new(app: &App) -> Ui {
        let page = PAGE_IDS.iter().copied().find(|p| *p == app.d.settings.page_id()).unwrap_or("water");
        Ui {
            expanded: false, hovering: false, hover_at: -1.0, leave_at: -1.0, peek_until: -1.0, clock: 0.0,
            mouse: (-1.0, -1.0),
            w: Spring::with(170.0, 2.4, 0.78), h: Spring::with(34.0, 2.4, 0.78), r: Spring::with(22.0, 2.4, 0.8), ear: Spring::with(12.0, 2.4, 0.8),
            content: Spring::with(0.0, 3.2, 1.0), pill_alpha: Spring::with(1.0, 4.0, 1.0),
            page, prev_page: page, page_t: 1.0, page_dir: 1.0,
            water_sub: Spring::with(if app.d.settings.water_view == "chart" { 1.0 } else { 0.0 }, 2.0, 0.9),
            water_edit: false, todo_form: Form::default(), event_form: Form::default(), focus: None,
            edit_todos: false, todo_scroll: 0.0, tweens: Default::default(), hits: vec![],
            cameo: Cameo {
                on: false, elapsed: 0.0, eyes: Spring::with(0.0, 3.0, 0.62), happy: Spring::with(0.0, 4.0, 0.9),
                paw: Spring::with(0.0, 2.6, 0.6), overlay: Spring::with(0.0, 1.8, 1.0), look_x: Spring::with(0.0, 2.5, 0.8),
                look_y: Spring::with(0.0, 2.5, 0.8), small: Spring::with(0.0, 3.0, 0.8), sleep: false, pet_until: -1.0, clock: 0.0,
                hearts: vec![], alert_until: -1.0,
            },
            last_active: 0.0, battery: None, show_lyrics: true, lyric_line: String::new(), lyric_swap: -10.0,
            art: None, art_key: String::new(), art_accent: None, muted: false, pill_w: 170.0,
            content_rect: (0.0, 0.0, 0.0, 0.0), tick_flash: 0.0, shadow: None, clip_cache: Default::default(), alert: None,
        }
    }

    fn visible_pages(app: &App) -> Vec<&'static str> {
        PAGE_IDS.iter().copied().filter(|p| app.d.settings.pages.get(p)).collect()
    }

    fn busy(&self) -> bool { self.todo_form.open || self.event_form.open || self.water_edit || self.focus.is_some() }

    /* ============================ state changes ============================ */

    pub fn expand(&mut self) {
        if self.expanded || self.alert.as_ref().is_some_and(|a| a.kind == "water" && a.done_at < 0.0) { return; }
        self.end_cameo();
        self.expanded = true;
        self.last_active = self.clock;
        // re-run value animations from zero on every open
        self.tweens.clear();
    }

    pub fn collapse(&mut self, force: bool) {
        if !self.expanded || (!force && self.busy()) { return; }
        self.expanded = false;
        self.focus = None;
        self.last_active = self.clock;
    }

    fn go(&mut self, app: &mut App, id: &'static str, dir: f32) {
        if id == self.page && !self.water_edit { return; }
        self.prev_page = self.page;
        self.page = id;
        self.page_t = 0.0;
        self.page_dir = dir;
        self.water_edit = false;
        self.todo_form.open = false;
        self.event_form.open = false;
        self.focus = None;
        self.edit_todos = false;
        for k in self.tweens.keys().cloned().collect::<Vec<_>>() { if !k.starts_with("dot") { self.tweens.remove(&k); } }
        app.action(&Action::LastPage(id));
    }

    pub fn peek(&mut self, app: &App, page: &'static str, force: bool, quick: bool) {
        if self.alert.as_ref().is_some_and(|a| a.kind == "water" && a.done_at < 0.0) { return; }
        let typing = self.focus.is_some();
        if app.d.settings.pages.get(page) && (page != self.page || self.water_edit) && !typing {
            self.prev_page = self.page;
            self.page = page;
            self.page_t = 1.0;
            self.water_edit = false;
        }
        if !app.d.settings.peek && !force { return; }
        self.expand();
        let secs = if quick { 3.5 } else { app.d.settings.peek_seconds as f32 };
        self.peek_until = self.clock + secs;
    }

    /// compact reminder popup (water/to-do/pomodoro/event)
    pub fn alert(&mut self, app: &App, kind: &'static str, page: &'static str, title: String, body: String) {
        if self.alert.as_ref().is_some_and(|a| a.kind == "water" && a.done_at < 0.0) { return; }
        if self.expanded {
            // already open: just jump to the page
            self.peek(app, page, true, false);
            return;
        }
        self.end_cameo();
        let secs = (app.d.settings.peek_seconds as f32).max(6.0) + 2.0;
        self.alert = Some(Alert { kind, page, title, body, t0: self.clock, until: self.clock + secs, done_at: -1.0, snoozed: false });
    }

    pub fn mood_happy(&mut self) { if self.cameo.on { self.cameo.pet_until = self.cameo.clock + 1.8; } }

    /* ============================ input ============================ */

    /// is the point over the visible notch body (for hover / click-through decisions)
    pub fn hit_body(&self, p: (f32, f32)) -> bool {
        let w = self.w.x.max(10.0);
        let h = self.h.x.max(6.0) + if self.expanded { 0.0 } else { 3.0 };
        p.0 >= CX - w / 2.0 && p.0 <= CX + w / 2.0 && p.1 >= -1.0 && p.1 <= h
    }

    pub fn mouse_move(&mut self, p: (f32, f32), app: &App) -> bool {
        let before = self.mouse;
        self.mouse = p;
        let over = self.hit_body(p) || (self.expanded && self.hovering && p.1 < EH + 4.0 && (p.0 - CX).abs() < EW / 2.0);
        if over && !self.hovering {
            self.hovering = true;
            self.hover_at = self.clock;
            self.leave_at = -1.0;
            if self.cameo.on {
                if self.cameo.sleep { self.cameo.alert_until = self.cameo.clock + 0.9; self.cameo.sleep = false; }
            } else { self.last_active = self.clock; }
            let _ = app;
        } else if !over && self.hovering {
            self.hovering = false;
            self.leave_at = self.clock;
        }
        before != p
    }

    pub fn mouse_leave(&mut self) {
        self.mouse = (-1.0, -1.0);
        if self.hovering { self.hovering = false; self.leave_at = self.clock; }
    }

    pub fn wheel(&mut self, app: &mut App, delta: f32) -> Vec<Out> {
        if !self.expanded { return vec![]; }
        let m = self.mouse;
        if let Some(h) = self.hits.iter().find(|h| h.act == Act::Media("vol") && inside(h.r, m)) {
            let _ = h;
            return vec![Out::Media(if delta > 0.0 { "volup".into() } else { "voldown".into() })];
        }
        if self.page == "todo" && m.0 > PX + 206.0 {
            self.todo_scroll = (self.todo_scroll - delta * 0.25).max(0.0);
            return vec![Out::Changed];
        }
        if self.busy() { return vec![]; }
        let dir = if delta < 0.0 { 1 } else { -1 };
        self.nav(app, dir);
        vec![Out::Changed]
    }

    fn nav(&mut self, app: &mut App, dir: i32) {
        let list = Self::visible_pages(app);
        if list.is_empty() { return; }
        let i = list.iter().position(|p| *p == self.page).unwrap_or(0) as i32;
        let n = list.len() as i32;
        let id = list[((i + dir) % n + n) as usize % n as usize];
        self.go(app, id, dir as f32);
    }

    pub fn dblclick(&mut self, _app: &App) -> bool {
        if self.cameo.on { self.end_cameo(); self.expand(); return true; }
        false
    }

    pub fn click(&mut self, app: &mut App, p: (f32, f32)) -> Vec<Out> {
        self.last_active = self.clock;
        let mut out = vec![Out::Changed];
        if !self.expanded && self.alert.is_some() {
            let act = self.hits.iter().rev().find(|h| inside(h.r, p)).map(|h| h.act.clone());
            match act {
                Some(Act::AlertAction) => {
                    let a = self.alert.as_mut().unwrap();
                    match a.kind {
                        "water" if a.done_at < 0.0 => { out.push(Out::App(Action::Drink)); a.done_at = self.clock; a.until = self.clock + 1.8; }
                        "water" => {}
                        "pomo" => { out.push(Out::App(Action::PomoToggle)); self.alert = None; }
                        _ => self.alert = None,
                    }
                }
                Some(Act::AlertSnooze) => {
                    if let Some(a) = self.alert.as_mut() {
                        if a.kind == "water" && a.done_at < 0.0 {
                            out.push(Out::App(Action::WaterSnooze));
                            a.done_at = self.clock;
                            a.until = self.clock + 1.8;
                            a.snoozed = true;
                        }
                    }
                }
                _ => {
                    if self.alert.as_ref().is_some_and(|a| a.kind == "water" && a.done_at < 0.0) { return out; }
                    let page = self.alert.take().map(|a| a.page).unwrap_or(self.page);
                    if app.d.settings.pages.get(page) { self.prev_page = self.page; self.page = page; self.page_t = 1.0; }
                    self.expand();
                }
            }
            return out;
        }
        if !self.expanded {
            if self.cameo.on && self.hit_body(p) {
                self.pet(app);
                return out;
            }
            if self.hit_body(p) { self.expand(); }
            return out;
        }
        let Some(act) = self.hits.iter().rev().find(|h| inside(h.r, p)).map(|h| h.act.clone()) else {
            if self.focus.is_some() { self.focus = None; out.push(Out::WantFocus(false)); }
            return out;
        };
        match act {
            Act::Nav(d) => self.nav(app, d),
            Act::Goto(id) => {
                let list = Self::visible_pages(app);
                let a = list.iter().position(|x| *x == id).unwrap_or(0);
                let b = list.iter().position(|x| *x == self.page).unwrap_or(0);
                self.go(app, id, if a < b { -1.0 } else { 1.0 });
            }
            Act::App(a) => {
                if a == Action::Drink { self.tweens.remove("bub"); }
                out.push(Out::App(a));
            }
            Act::WaterSub => {
                let v = if self.water_sub.target > 0.5 { 0.0 } else { 1.0 };
                self.water_sub.set(v);
                if v > 0.5 { for k in self.tweens.keys().cloned().collect::<Vec<_>>() { if k.starts_with("wb") { self.tweens.remove(&k); } } }
                out.push(Out::App(Action::WaterView(v > 0.5)));
            }
            Act::WaterEdit(on) => { self.water_edit = on; self.page_t = 0.0; self.page_dir = if on { 1.0 } else { -1.0 }; self.prev_page = self.page; }
            Act::WaterIntervalCycle => {
                const IV: [i32; 7] = [15, 20, 30, 45, 60, 90, 120];
                let i = IV.iter().position(|v| *v == app.d.water.interval).map(|i| (i + 1) % IV.len()).unwrap_or(4);
                out.push(Out::App(Action::WaterInterval(IV[i])));
            }
            Act::FormOpen(f) => {
                let form = if f == FieldId::Todo { &mut self.todo_form } else { &mut self.event_form };
                form.open = !form.open;
                if form.open {
                    *form = Form { open: true, ..Default::default() };
                    self.focus = Some(f);
                    out.push(Out::WantFocus(true));
                } else { self.focus = None; out.push(Out::WantFocus(false)); }
            }
            Act::FormCancel => {
                self.todo_form.open = false;
                self.event_form.open = false;
                self.focus = None;
                out.push(Out::WantFocus(false));
            }
            Act::FormSave => { out.extend(self.save_form()); }
            Act::FormDaily => self.todo_form.daily = !self.todo_form.daily,
            Act::FormTime(d) => {
                let form = if self.page == "todo" { &mut self.todo_form } else { &mut self.event_form };
                form.time = Some(match form.time {
                    None => { let l = time::local(now_ms()); let m = (l.hour * 60 + l.minute + 15) / 15 * 15; ((m / 60) % 24, m % 60) }
                    Some((h, m)) => { let t = ((h * 60 + m) as i32 + d * 15).rem_euclid(24 * 60) as u32; (t / 60, t % 60) }
                });
            }
            Act::FormDay(d) => self.event_form.day = (self.event_form.day + d).clamp(0, 60),
            Act::Field(f) => { self.focus = Some(f); out.push(Out::WantFocus(true)); }
            Act::Media(cmd) => {
                match cmd {
                    "vol" => { self.muted = !self.muted; out.push(Out::Media("mute".into())); }
                    "toggle" => {
                        if app.media.ok {
                            let pos = app.media.position();
                            app.media.playing = !app.media.playing;
                            app.media.pos = pos;
                            app.media.upd = now_ms();
                        }
                        out.push(Out::Media("toggle".into()));
                    }
                    c => out.push(Out::Media(c.into())),
                }
            }
            Act::Seek(x0, w) => {
                if app.media.ok && app.media.end > 0 {
                    let ms = (clamp01((p.0 - x0) / w) * app.media.end as f32) as i64;
                    app.media.pos = ms;
                    app.media.upd = now_ms();
                    out.push(Out::Media(format!("seek {}", ms)));
                }
            }
            Act::Lyrics => self.show_lyrics = !self.show_lyrics,
            Act::EditTodos => self.edit_todos = !self.edit_todos,
            Act::Pet => self.pet(app),
            Act::AlertAction | Act::AlertSnooze | Act::AlertOpen => {}
        }
        out
    }

    fn save_form(&mut self) -> Vec<Out> {
        let mut out = vec![];
        if self.page == "todo" {
            let f = std::mem::take(&mut self.todo_form);
            if f.text.trim().is_empty() { self.todo_form = f; self.focus = Some(FieldId::Todo); return out; }
            out.push(Out::App(Action::TodoAdd { text: f.text, time: f.time, daily: f.daily }));
        } else {
            let f = std::mem::take(&mut self.event_form);
            if f.text.trim().is_empty() { self.event_form = f; self.focus = Some(FieldId::Event); return out; }
            let date = time::day_key(time::start_of_local_day(now_ms()) + f.day as i64 * 86_400_000 + 3_600_000 * 12);
            out.push(Out::App(Action::EventAdd { title: f.text, date, time: f.time }));
        }
        self.focus = None;
        out.push(Out::WantFocus(false));
        out
    }

    /// typed character (already translated by the OS, so IME/Telex input works)
    pub fn char_input(&mut self, c: char) -> Vec<Out> {
        let Some(f) = self.focus else { return vec![] };
        let form = if f == FieldId::Todo { &mut self.todo_form } else { &mut self.event_form };
        match c {
            '\u{8}' => { form.text.pop(); }
            '\r' | '\n' => return self.save_form(),
            '\u{1b}' => {
                self.todo_form.open = false;
                self.event_form.open = false;
                self.focus = None;
                return vec![Out::WantFocus(false), Out::Changed];
            }
            c if !c.is_control() => { if form.text.chars().count() < 200 { form.text.push(c); } }
            _ => {}
        }
        vec![Out::Changed]
    }

    fn pet(&mut self, app: &App) {
        let cm = &mut self.cameo;
        if cm.sleep { cm.sleep = false; cm.alert_until = cm.clock + 0.8; }
        cm.pet_until = cm.clock + 2.2;
        let n = 4;
        for i in 0..n {
            let seed = (cm.clock * 1000.0) as u32 + i * 7919;
            let rx = ((seed.wrapping_mul(2654435761) >> 16) % 1000) as f32 / 1000.0;
            cm.hearts.push((rx * 52.0 - 26.0, cm.clock + i as f32 * 0.12, rx * 40.0 - 20.0));
        }
        let _ = app;
    }

    /* ============================ cameo ============================ */

    fn cameo_allowed(&self, app: &App) -> bool {
        let s = &app.d.settings;
        if !s.mascot || self.expanded || app.pomo.running || self.busy() || self.alert.is_some() { return false; }
        if app.media.ok && app.media.playing && s.media_pill && s.pages.media { return false; }
        true
    }

    fn start_cameo(&mut self) {
        let cm = &mut self.cameo;
        cm.on = true;
        cm.elapsed = 0.0;
        cm.eyes.snap(0.0);
        cm.happy.snap(0.0);
        cm.paw.snap(0.0);
        cm.small.snap(0.0);
        cm.look_x.snap(0.0);
        cm.look_y.snap(0.0);
        cm.overlay.set(1.0);
        let h = time::local(now_ms()).hour;
        cm.sleep = h >= 23 || h < 6;
    }

    fn end_cameo(&mut self) {
        if !self.cameo.on { return; }
        self.cameo.on = false;
        self.cameo.overlay.set(0.0);
        self.cameo.eyes.set(0.0);
        self.cameo.paw.set(0.0);
        self.last_active = self.clock;
    }

    /* ============================ update ============================ */

    /// advance animations. Returns (keep animating at vsync, ms until next needed redraw)
    pub fn update(&mut self, app: &mut App, fonts: &mut Fonts, dt: f32) -> (bool, i64) {
        self.clock += dt;
        self.cameo.clock = self.clock;
        let s = app.d.settings.clone();

        // hover → expand / leave → collapse
        if let Some(a) = &mut self.alert {
            let pending_water = a.kind == "water" && a.done_at < 0.0;
            if self.expanded && !pending_water { self.alert = None; }
            else if self.hovering && a.done_at < 0.0 && !pending_water { a.until = a.until.max(self.clock + 1.5); }
            else if !pending_water && self.clock > a.until { self.alert = None; self.last_active = self.clock; }
        }
        if self.hovering && !self.expanded && s.hover_expand && self.hover_at >= 0.0 && self.alert.is_none() {
            let delay = if self.cameo.on { 2.5 } else { 0.07 };
            if self.clock - self.hover_at > delay { self.expand(); self.hover_at = -1.0; }
        }
        if !self.hovering && self.expanded && self.leave_at >= 0.0 && self.clock - self.leave_at > s.collapse_delay as f32 / 1000.0 {
            if self.peek_until < self.clock { self.collapse(false); }
            self.leave_at = -1.0;
        }
        if self.peek_until >= 0.0 && self.clock > self.peek_until {
            self.peek_until = -1.0;
            if !self.hovering { self.collapse(false); }
        }

        // cameo
        if !self.cameo.on && s.idle_after > 0 && self.cameo_allowed(app) && self.clock - self.last_active > s.idle_after as f32 {
            self.start_cameo();
        }
        if !self.cameo.on && self.clock > 2.5 && self.clock - dt <= 2.5 && s.idle_after > 0 && self.cameo_allowed(app) {
            self.start_cameo(); // say hi after launch
        }
        if self.cameo.on && !self.cameo_allowed(app) { self.end_cameo(); }
        let mut cameo_anim = false;
        if self.cameo.on {
            let held = self.hovering || self.clock < self.cameo.pet_until;
            if !held { self.cameo.elapsed += dt; }
            let e = self.cameo.elapsed;
            let cm = &mut self.cameo;
            let petting = self.clock < cm.pet_until;
            if cm.sleep {
                cm.eyes.set(0.0); cm.happy.set(0.0); cm.paw.set(0.0);
                if e > 4.8 { self.end_cameo(); }
            } else {
                let (eyes, happy, paw, small) = if petting { (1.0, 1.0, 1.0, 0.0) }
                    else if self.hovering { (1.0, 0.0, 0.0, 0.0) }
                    else if e < 0.42 { (0.0, 0.0, 0.0, 0.0) }
                    else if e < 1.32 { (1.0, 0.0, 0.0, 0.0) }
                    else if e < 4.02 { (1.0, 1.0, 1.0, 0.0) }
                    else if e < 5.12 { (1.0, 0.0, 0.0, 1.0) }
                    else if e < 6.0 { (1.0, 0.0, 0.0, 0.0) }
                    else { (0.0, 0.0, 0.0, 0.0) };
                cm.eyes.set(eyes); cm.happy.set(happy); cm.paw.set(paw); cm.small.set(small);
                if e > 6.25 { self.end_cameo(); }
            }
            let cm = &mut self.cameo;
            if self.hovering && !petting {
                let dx = ((self.mouse.0 - CX) / 110.0).clamp(-1.0, 1.0);
                let dy = ((self.mouse.1 - self.h.x) / 50.0).clamp(-1.0, 1.0);
                cm.look_x.set(dx); cm.look_y.set(dy);
            } else { cm.look_x.set(0.0); cm.look_y.set(0.0); }
            cameo_anim = true;
        }
        let cm = &mut self.cameo;
        cm.hearts.retain(|h| cm.clock - h.1 < 1.2);
        for sp in [&mut cm.eyes, &mut cm.happy, &mut cm.paw, &mut cm.overlay, &mut cm.look_x, &mut cm.look_y, &mut cm.small] {
            if sp.step(dt) { cameo_anim = true; }
        }
        if !cm.hearts.is_empty() || self.clock < cm.alert_until { cameo_anim = true; }

        // shell targets
        let playing = app.media.ok && app.media.playing && s.media_pill && s.pages.media;
        let autohide = s.auto_hide && !app.pomo.running && !playing && !self.cameo.on && !self.hovering && !self.expanded;
        let minimal = s.minimal && !app.pomo.running && !playing;
        self.pill_w = self.measure_pill(app, fonts);
        let sf = 1.0;
        let _ = sf;
        if self.expanded {
            self.w.tune(2.3, 0.74); self.h.tune(2.4, 0.76); self.r.tune(2.4, 0.8); self.ear.tune(2.4, 0.8);
            self.w.set(EW); self.h.set(EH); self.r.set(42.0); self.ear.set(20.0);
            self.content.tune(3.4, 1.0);
            // content fades in once the shell is mostly open
            let open = clamp01((self.h.x - 40.0) / (EH - 40.0));
            self.content.set(if open > 0.35 { 1.0 } else { 0.0 });
            self.pill_alpha.set(0.0);
        } else {
            self.w.tune(2.6, 0.95); self.h.tune(2.8, 0.95); self.r.tune(2.8, 0.95); self.ear.tune(2.8, 0.95);
            let (w, h, r, e) = if self.alert.is_some() { (if self.alert.as_ref().is_some_and(|a| a.kind == "water") { 460.0 } else { 360.0 }, 60.0, 28.0, 14.0) }
                else if autohide { (140.0, 6.0, 6.0, 5.0) }
                else if minimal { (self.pill_w.max(110.0), 28.0, 17.0, 10.0) }
                else { (self.pill_w.max(150.0), if self.hovering { 37.0 } else { 34.0 }, 22.0, 12.0) };
            self.w.set(w); self.h.set(h); self.r.set(r); self.ear.set(e);
            self.content.tune(9.0, 1.0);
            self.content.set(0.0);
            self.pill_alpha.set(if autohide || self.cameo.on || self.alert.is_some() { 0.0 } else { 1.0 });
        }
        let mut anim = false;
        for sp in [&mut self.w, &mut self.h, &mut self.r, &mut self.ear, &mut self.content, &mut self.pill_alpha, &mut self.water_sub] {
            if sp.step(dt) { anim = true; }
        }
        for sp in self.tweens.values_mut() { if sp.step(dt) { anim = true; } }
        if self.page_t < 1.0 { self.page_t = (self.page_t + dt / 0.34).min(1.0); anim = true; }
        let alert_anim = self.alert.as_ref().is_some_and(|a| a.kind != "water" || self.clock - a.t0 < 1.6 || (a.done_at >= 0.0 && self.clock - a.done_at < 1.8));
        if cameo_anim || alert_anim { anim = true; }
        if self.hovering && !self.expanded && self.hover_at >= 0.0 { anim = true; }

        // lyrics line change → small slide animation
        if self.expanded && self.page == "media" && self.show_lyrics {
            let l = current_lyric(app);
            if let Some(l) = l { if l != self.lyric_line { self.lyric_line = l; self.lyric_swap = self.clock; } }
            if self.clock - self.lyric_swap < 0.45 { anim = true; }
        }

        // next timed redraw
        let now = now_ms();
        let mut wait = 60_000 - now % 60_000;
        if app.pomo.running { wait = wait.min(1000 - now % 1000); }
        if self.expanded && self.page == "media" && app.media.ok && app.media.playing { wait = wait.min(250); }
        if playing && !self.expanded { wait = wait.min(66); } // pill equalizer ~15 fps
        if self.focus.is_some() { wait = wait.min(530 - now % 530); } // caret blink
        if self.peek_until > 0.0 || self.leave_at > 0.0 { wait = wait.min(50); }
        if s.idle_after > 0 && !self.cameo.on {
            let left = ((s.idle_after as f32 - (self.clock - self.last_active)) * 1000.0) as i64;
            wait = wait.min(left.max(100));
        }
        (anim, wait.max(16))
    }

    fn measure_pill(&self, app: &App, fonts: &mut Fonts) -> f32 {
        let h12 = !app.d.settings.clock24;
        let (t, ap) = time::clock(now_ms(), h12);
        let mut w = 36.0;
        if self.battery.map(|b| b.1).unwrap_or(false) && app.d.settings.show_battery { w += 22.0 + 9.0; }
        w += fonts.measure(&t, 17.0, W::Bold) + if ap.is_empty() { 0.0 } else { 3.0 + fonts.measure(ap, 11.0, W::Bold) };
        let playing = app.media.ok && app.media.playing && app.d.settings.media_pill && app.d.settings.pages.media;
        if app.pomo.running { w += 12.0 + 7.0 + 6.0 + fonts.measure(&time::mmss(app.pomo_remaining()), 13.0, W::Bold); }
        else if playing { w += 12.0 + if app.media.art.is_some() { 26.0 } else { 0.0 } + 18.0; }
        w
    }

    fn tween(&mut self, key: &str, to: f32, from: f32, freq: f32, zeta: f32) -> f32 {
        let sp = self.tweens.entry(key.to_string()).or_insert_with(|| { let mut s = Spring::with(from, freq, zeta); s.set(to); s });
        sp.set(to);
        sp.x
    }

    /* ============================ drawing ============================ */

    pub fn draw(&mut self, app: &App, cv: &mut Canvas) {
        self.hits.clear();
        cv.pm.fill(Color::TRANSPARENT);
        let (w, h, r, e) = (self.w.x, self.h.x, self.r.x, self.ear.x);
        let x0 = CX - w / 2.0;
        let content = clamp01(self.content.x);

        // soft shadow under the expanded notch (pre-rendered once, faded in as it opens)
        let sh = clamp01((h - 120.0) / (EH - 120.0));
        if sh > 0.02 {
            if self.shadow.as_ref().map(|p| p.0 != cv.s).unwrap_or(true) {
                let pw = cv.pm.width();
                let ph = cv.pm.height();
                let mut spm = Pixmap::new(pw, ph).unwrap();
                for i in 0..7 {
                    let g = i as f32 * 3.0;
                    if let Some(p) = rrect_path(FX - g * 0.5, 4.0, EW + g, EH + g * 0.9, [0.0, 0.0, 42.0 + g, 42.0 + g]) {
                        let mut paint = Paint::default();
                        paint.set_color(rgb(0, 0.045));
                        paint.anti_alias = true;
                        spm.fill_path(&p, &paint, FillRule::Winding, Transform::from_scale(cv.s, cv.s), None);
                    }
                }
                self.shadow = Some((cv.s, spm));
            }
            if let Some((_, spm)) = &self.shadow {
                let sx = ((w - EW) / 2.0 * cv.s) as i32;
                let paint = PixmapPaint { opacity: sh, ..Default::default() };
                let sy = (h - EH) * cv.s;
                cv.pm.draw_pixmap(0, 0, spm.as_ref(), &paint, Transform::from_row(w / EW, 0.0, 0.0, 1.0, (CX * cv.s) * (1.0 - w / EW), sy.min(0.0)), None);
                let _ = sx;
            }
        }

        // body + concave "ears" that melt into the screen edge
        let body = rrect_path(x0, -2.0, w, h + 2.0, [0.0, 0.0, r, r]);
        let mut pb = PathBuilder::new();
        let k = 0.552_284_8;
        // left ear
        pb.move_to(x0 - e, 0.0);
        pb.line_to(x0 + 1.0, 0.0);
        pb.line_to(x0 + 1.0, e);
        pb.line_to(x0, e);
        pb.cubic_to(x0, e - e * k, x0 - e + e * k, 0.0, x0 - e, 0.0);
        pb.close();
        // right ear
        let x1 = x0 + w;
        pb.move_to(x1 + e, 0.0);
        pb.line_to(x1 - 1.0, 0.0);
        pb.line_to(x1 - 1.0, e);
        pb.line_to(x1, e);
        pb.cubic_to(x1, e - e * k, x1 + e - e * k, 0.0, x1 + e, 0.0);
        pb.close();
        if let Some(p) = pb.finish() { cv.fill(&p, 0x000000, 1.0); }
        if let Some(b) = &body { cv.fill(b, 0x000000, 1.0); }

        // clip everything else to the body (only needed while the shell is smaller than its content)
        let steady = self.expanded && (w - EW).abs() < 1.0 && (h - EH).abs() < 1.5;
        if !steady { if let Some(b) = &body {
            let mut m = Mask::new(cv.pm.width(), cv.pm.height()).unwrap();
            m.fill_path(b, FillRule::Winding, true, Transform::from_scale(cv.s, cv.s));
            cv.mask = Some(m);
        } }

        // cameo overlay
        if self.cameo.overlay.x > 0.01 { self.draw_cameo(cv, x0, w, h); }

        // reminder popup
        if self.alert.is_some() && !self.expanded { self.draw_alert(app, cv, x0, w, h); }

        // collapsed pill
        let pa = clamp01(self.pill_alpha.x) * (1.0 - content);
        if pa > 0.01 {
            cv.alpha = pa;
            self.draw_pill(app, cv, h);
            cv.alpha = 1.0;
        }

        // expanded content
        if content > 0.01 {
            let rise = (1.0 - content) * -6.0;
            cv.oy = rise;
            cv.alpha = content;
            self.draw_header(app, cv);
            self.draw_nav(app, cv);
            // page transition: old page slides out, new page slides in
            let t = self.page_t;
            if t < 1.0 {
                let out_t = ease_out((t / 0.35).min(1.0));
                if out_t < 1.0 && (self.prev_page != self.page || true) {
                    let keep_edit = self.water_edit;
                    let prev = self.prev_page;
                    cv.ox = -self.page_dir * 14.0 * out_t;
                    cv.alpha = content * (1.0 - out_t);
                    let cur = self.page;
                    if prev == cur { self.water_edit = !keep_edit; }
                    self.page = prev;
                    self.draw_page(app, cv, false);
                    self.page = cur;
                    self.water_edit = keep_edit;
                }
                let in_t = ease_out(((t - 0.12) / 0.88).max(0.0));
                cv.ox = self.page_dir * 20.0 * (1.0 - in_t);
                cv.alpha = content * in_t;
                self.draw_page(app, cv, true);
                cv.ox = 0.0;
            } else {
                self.draw_page(app, cv, true);
            }
            cv.alpha = content;
            self.draw_dots(app, cv);
            cv.alpha = 1.0;
            cv.oy = 0.0;
        }
        cv.mask = None;

        // the active window rect for hit testing (logical)
        self.content_rect = (x0 - e, 0.0, w + 2.0 * e, h);
    }

    fn hit(&mut self, cv: &Canvas, x: f32, y: f32, w: f32, h: f32, act: Act) {
        if cv.alpha < 0.3 { return; }
        self.hits.push(Hit { r: (x + cv.ox, y + cv.oy, w, h), act });
    }

    fn hov(&self, cv: &Canvas, x: f32, y: f32, w: f32, h: f32) -> bool {
        inside((x + cv.ox, y + cv.oy, w, h), self.mouse)
    }

    /* ---------- pill ---------- */

    fn draw_pill(&mut self, app: &App, cv: &mut Canvas, h: f32) {
        let s = &app.d.settings;
        let minimal = s.minimal && !app.pomo.running;
        let fs = if minimal { 14.0 } else { 17.0 };
        let cy = h / 2.0;
        let w = self.pill_w;
        let mut x = CX - w / 2.0 + 18.0;
        if self.battery.map(|b| b.1).unwrap_or(false) && s.show_battery {
            let d = if minimal { 17.0 } else { 22.0 };
            self.bolt_badge(cv, x + d / 2.0, cy, d);
            x += d + 9.0;
        }
        let (t, ap) = time::clock(now_ms(), !s.clock24);
        let base = cy + fs * 0.36;
        x += cv.text(x, base, &t, fs, W::Bold, WHITE, 1.0);
        if !ap.is_empty() { x += 3.0; cv.text(x, base, ap, fs * 0.65, W::Bold, WHITE, 1.0); x += cv.measure(ap, fs * 0.65, W::Bold); }
        let playing = app.media.ok && app.media.playing && s.media_pill && s.pages.media;
        x += 12.0;
        if app.pomo.running {
            let c = app.pomo.phase.color();
            let blink = 0.65 + 0.35 * ((self.clock * 3.9).sin());
            cv.circle(x + 3.5, cy, 3.5, c, blink);
            cv.text(x + 13.0, cy + 4.7, &time::mmss(app.pomo_remaining()), 13.0, W::Bold, WHITE, 1.0);
        } else if playing {
            if let Some(img) = &self.art {
                let img = img.clone();
                cv.image(x, cy - 10.0, 20.0, 5.0, &img);
                x += 26.0;
            }
            let c = self.art_accent.unwrap_or(0xff375f);
            for i in 0..4 {
                let ph = self.clock * 6.3 + [0.2, 0.6, 0.4, 0.8][i] * 6.3;
                let bh = 4.0 + 10.0 * (0.5 + 0.5 * ph.sin());
                cv.rrect(x + i as f32 * 5.0, cy - bh / 2.0, 3.0, bh, 1.5, c, 1.0);
            }
        }
    }

    fn bolt_badge(&mut self, cv: &mut Canvas, cx: f32, cy: f32, d: f32) {
        cv.circle(cx, cy, d / 2.0 + 2.0, 0x30d158, 0.12);
        cv.arc(cx, cy, d / 2.0 - 1.2, 0.0, 360.0, 0x30d158, 1.0, d * 0.11);
        cv.icon("bolt", icons::BOLT, cx - d * 0.27, cy - d * 0.27, d * 0.54, 0x30d158, 1.0);
    }

    /* ---------- reminder popup ---------- */

    fn draw_alert(&mut self, app: &App, cv: &mut Canvas, x0: f32, w: f32, h: f32) {
        let Some(a) = &self.alert else { return };
        let (kind, title, body, t0, until, done_at, snoozed) = (a.kind, a.title.clone(), a.body.clone(), a.t0, a.until, a.done_at, a.snoozed);
        // fade the content in once the capsule has grown
        let grow = clamp01((h - 34.0) / (60.0 - 34.0));
        let persistent = kind == "water" && done_at < 0.0;
        let fade = if persistent { 1.0 } else { smooth(0.0, 0.25, until - self.clock + 0.25) };
        let al = smooth(0.55, 1.0, grow) * fade * (1.0 - clamp01(self.pill_alpha.x));
        if al < 0.01 { return; }
        cv.alpha = al;
        let t = self.clock - t0;
        let done = done_at >= 0.0;
        let color = if done { GREEN } else { match kind { "water" => WATER, "todo" => GREEN, "pomo" => 0xff7a45, _ => 0xff5a5f } };
        let icx = x0 + 34.0;
        let icy = h / 2.0;
        // ripples
        if !done {
            for k in 0..2 {
                let ph = ((t + k as f32 * 0.7) % 1.4) / 1.4;
                cv.circle(icx, icy, 19.0 + ph * 12.0, color, 0.22 * (1.0 - ph));
            }
        }
        cv.circle(icx, icy, 19.0, color, 0.2);
        // shake: short bursts every 1.6 s
        let tp = t % 1.6;
        let ang = if done { 0.0 } else if tp < 0.65 { (tp * 34.0).sin() * 18.0 * (1.0 - tp / 0.65) } else { 0.0 };
        let pop = 1.0 + 0.28 * (-6.0 * t).exp() * (14.0 * t).sin();
        if done {
            let dt = self.clock - done_at;
            let sc = 1.0 + 0.35 * (-7.0 * dt).exp() * (16.0 * dt).sin();
            cv.circle(icx, icy, 12.0 * sc, GREEN, 1.0);
            cv.icon("check", icons::CHECK, icx - 7.0 * sc, icy - 7.0 * sc, 14.0 * sc, 0x002a0c, 1.0);
        } else if kind == "water" {
            // water drop drawn as a path so it can wobble around its base
            let pts: [(f32, f32); 13] = [(0.0, -11.0), (3.0, -6.5), (8.0, -2.0), (8.0, 3.0), (8.0, 7.8), (4.4, 11.0), (0.0, 11.0),
                (-4.4, 11.0), (-8.0, 7.8), (-8.0, 3.0), (-8.0, -2.0), (-3.0, -6.5), (0.0, -11.0)];
            let (sn, cs) = ang.to_radians().sin_cos();
            let sc = 0.95 * pop;
            let tf = |p: (f32, f32)| { let (x, y) = (p.0, p.1 - 8.0); (icx + (x * cs - y * sn) * sc, icy + 1.0 + (x * sn + y * cs + 8.0) * sc - 1.0) };
            let mut pb = PathBuilder::new();
            let p0 = tf(pts[0]); pb.move_to(p0.0, p0.1);
            for i in 0..4 {
                let (a1, a2, a3) = (tf(pts[1 + i * 3]), tf(pts[2 + i * 3]), tf(pts[3 + i * 3]));
                pb.cubic_to(a1.0, a1.1, a2.0, a2.1, a3.0, a3.1);
            }
            pb.close();
            if let Some(p) = pb.finish() {
                let g = LinearGradient::new(Point::from_xy(0.0, (icy - 11.0) * cv.s), Point::from_xy(0.0, (icy + 11.0) * cv.s),
                    vec![GradientStop::new(0.0, rgb(0x8fe0ff, al)), GradientStop::new(1.0, rgb(0x2a8cff, al))], SpreadMode::Pad, Transform::identity());
                if let Some(g) = g { cv.fill_shader(&p, g); }
            }
            cv.ellipse(icx - 3.0 + ang * 0.08, icy + 2.0, 1.8, 3.0, 0xffffff, 0.75);
        } else {
            let (n, svg) = match kind { "todo" => ("bell", icons::BELL), "pomo" => ("stopwatch", icons::STOPWATCH), _ => ("cal", icons::CAL_OUTLINE) };
            let dx = ang * 0.12;
            cv.icon(n, svg, icx - 10.0 * pop + dx, icy - 10.0 * pop, 20.0 * pop, color, 1.0);
        }
        // text
        let tx = x0 + 64.0;
        let btn = if done { String::new() } else {
            match kind { "water" => format!("Drink +{} ml", app.water_sip()), "pomo" => if app.pomo.running { String::new() } else { "Start".into() }, _ => "OK".into() }
        };
        let btn_w = if btn.is_empty() || kind == "water" { 0.0 } else { cv.measure(&btn, 12.5, W::Bold) + 26.0 };
        let water_btn_w = if persistent { cv.measure(&btn, 12.5, W::Bold) + 26.0 } else { 0.0 };
        let snooze_w = if persistent { cv.measure("Snooze", 12.5, W::Bold) + 26.0 } else { 0.0 };
        let action_w = btn_w + water_btn_w + snooze_w + if persistent { 8.0 } else { 0.0 };
        let maxw = w - 64.0 - 16.0 - if action_w > 0.0 { action_w + 10.0 } else { 0.0 };
        let (t1, t2) = if done {
            if snoozed {
                let minutes = ((app.water_next_at - now_ms() + 59_999) / 60_000).max(0);
                let body = if minutes <= WATER_SNOOZE_MINUTES { format!("Reminder in {} minutes", minutes) }
                    else { format!("Next reminder at {}", time::hm24(app.water_next_at)) };
                ("Snoozed".to_string(), body)
            } else { ("Nice! Keep it up".to_string(), format!("{} / {} L today", liters(app.water_today()), liters(app.d.water.goal))) }
        } else if kind == "water" {
            ("Time to drink water".to_string(), format!("{} / {} L · {}%", liters(app.water_today()), liters(app.d.water.goal), (app.water_today() as f32 / app.d.water.goal as f32 * 100.0).round()))
        } else { (title, body) };
        let t1 = cv.ellipsize(&t1, 14.5, W::Bold, maxw);
        let t2 = cv.ellipsize(&t2, 12.0, W::Regular, maxw);
        cv.text(tx, icy - 2.0, &t1, 14.5, W::Bold, WHITE, 1.0);
        cv.text(tx, icy + 15.0, &t2, 12.0, W::Regular, MUTED, 1.0);
        if persistent {
            let bx = x0 + w - 16.0 - water_btn_w - 8.0 - snooze_w;
            let hov = self.hov(cv, bx, icy - 15.0, snooze_w, 30.0);
            cv.rrect(bx, icy - 15.0, snooze_w, 30.0, 15.0, if hov { CHIP_H } else { CHIP }, 1.0);
            cv.text_center(bx + snooze_w / 2.0, icy + 4.5, "Snooze", 12.5, W::Bold, WHITE, 1.0);
            self.hit(cv, bx - 4.0, icy - 18.0, snooze_w + 8.0, 36.0, Act::AlertSnooze);
            let drink_x = bx + snooze_w + 8.0;
            let hov = self.hov(cv, drink_x, icy - 15.0, water_btn_w, 30.0);
            cv.rrect(drink_x, icy - 15.0, water_btn_w, 30.0, 15.0, if hov { lighten(color) } else { color }, 1.0);
            cv.text_center(drink_x + water_btn_w / 2.0, icy + 4.5, &btn, 12.5, W::Bold, 0x00243d, 1.0);
            self.hit(cv, drink_x - 4.0, icy - 18.0, water_btn_w + 8.0, 36.0, Act::AlertAction);
        } else if btn_w > 0.0 {
            let bx = x0 + w - 16.0 - btn_w;
            let hov = self.hov(cv, bx, icy - 15.0, btn_w, 30.0);
            cv.rrect(bx, icy - 15.0, btn_w, 30.0, 15.0, if hov { lighten(color) } else { color }, 1.0);
            cv.text_center(bx + btn_w / 2.0, icy + 4.5, &btn, 12.5, W::Bold, if kind == "water" { 0x00243d } else { WHITE }, 1.0);
            self.hit(cv, bx - 4.0, icy - 18.0, btn_w + 8.0, 36.0, Act::AlertAction);
        }
        // time-left hairline
        if !done && !persistent {
            let total = until - t0;
            let left = clamp01((until - self.clock) / total.max(0.1));
            cv.rrect(x0 + 28.0, h - 3.0, (w - 56.0) * left, 2.0, 1.0, color, 0.55);
        }
        self.hit(cv, x0, 0.0, w, h, Act::AlertOpen);
        // keep action buttons on top of the body hit
        for action in [Act::AlertAction, Act::AlertSnooze] {
            if let Some(i) = self.hits.iter().position(|h| h.act == action) { let hb = self.hits.remove(i); self.hits.push(hb); }
        }
        cv.alpha = 1.0;
    }

    /* ---------- cameo ---------- */

    fn draw_cameo(&mut self, cv: &mut Canvas, x0: f32, w: f32, h: f32) {
        let a = clamp01(self.cameo.overlay.x);
        // glow: blue → violet along the bottom edge, black on top
        let grad = LinearGradient::new(
            Point::from_xy(x0 * cv.s, 0.0), Point::from_xy((x0 + w) * cv.s, 0.0),
            vec![
                GradientStop::new(0.0, rgb(0x12d0ff, a)), GradientStop::new(0.38, rgb(0x2d7bff, a)),
                GradientStop::new(0.62, rgb(0x4b4dff, a)), GradientStop::new(1.0, rgb(0x8a3cff, a)),
            ],
            SpreadMode::Pad, Transform::identity(),
        );
        if let (Some(g), Some(p)) = (grad, rrect_path(x0, -2.0, w, h + 2.0, [0.0, 0.0, self.r.x, self.r.x])) {
            let saved = cv.alpha;
            let tf_s = cv.s;
            let mut paint = Paint::default();
            paint.shader = g;
            paint.anti_alias = true;
            cv.pm.fill_path(&p, &paint, FillRule::Winding, Transform::from_scale(tf_s, tf_s), cv.mask.as_ref());
            let shade = LinearGradient::new(
                Point::from_xy(0.0, 0.0), Point::from_xy(0.0, h * tf_s),
                vec![GradientStop::new(0.22, rgb(0, 1.0)), GradientStop::new(0.62, rgb(0, 0.55)), GradientStop::new(1.0, rgb(0, 0.0))],
                SpreadMode::Pad, Transform::identity(),
            );
            if let Some(sh) = shade {
                let mut paint = Paint::default();
                paint.shader = sh;
                cv.pm.fill_path(&p, &paint, FillRule::Winding, Transform::from_scale(tf_s, tf_s), cv.mask.as_ref());
            }
            cv.alpha = saved;
        }
        let cm = &self.cameo;
        let cy = h / 2.0;
        let lx = cm.look_x.x * 7.0;
        let ly = cm.look_y.x * 2.5;
        let shake = if cm.clock < cm.alert_until { ((cm.clock * 40.0).sin()) * 2.0 } else { 0.0 };
        let hop = if cm.clock < cm.pet_until { -((cm.clock * 14.0).sin().abs()) * 2.0 } else { 0.0 };
        let ex = [CX - 11.0 + lx + shake, CX + 11.0 + lx + shake];
        let ey = cy + ly + hop;
        let open = cm.eyes.x.max(0.0);
        let happy = clamp01(cm.happy.x);
        let small = clamp01(cm.small.x);
        cv.alpha = a;
        for x in ex {
            if cm.sleep {
                if let Some(p) = arc_path(x, ey - 2.6, 4.6, 30.0, 120.0) { cv.stroke(&p, WHITE, 1.0, 2.2); }
                continue;
            }
            // eye: slit (open=0) → tall oval (open=1); glow by stacking soft ovals
            let ry = lerp(1.0, 5.8, open) * (1.0 - 0.15 * small);
            let rx = lerp(4.4, 3.5, clamp01(open)) * (1.0 - 0.12 * small);
            let oa = 1.0 - happy;
            for g in 1..4 {
                let gg = g as f32 * 1.6;
                cv.ellipse(x, ey, rx + gg, ry + gg, 0xa9c8ff, 0.10 * oa);
            }
            cv.ellipse(x, ey, rx, ry, WHITE, oa);
            if happy > 0.01 {
                if let Some(p) = arc_path(x, ey + 2.6, 4.9, 200.0, 140.0) {
                    cv.stroke(&p, 0xa9c8ff, 0.25 * happy, 5.0);
                    cv.stroke(&p, WHITE, happy, 2.6);
                }
            }
        }
        // tongue while petting
        if cm.clock < cm.pet_until {
            if let Some(p) = { let mut pb = PathBuilder::new(); pb.move_to(CX - 1.6, ey + 6.5); pb.quad_to(CX, ey + 11.5, CX + 1.6, ey + 6.5); pb.close(); pb.finish() } {
                cv.fill(&p, 0xff76a8, 1.0);
            }
        }
        // waving paw
        let paw = clamp01(cm.paw.x);
        if paw > 0.01 {
            let wave = if cm.paw.target > 0.5 { (cm.clock * 9.8).sin() * 17.0 } else { 0.0 };
            let px = CX + 26.5;
            let py = ey + 4.6 + (1.0 - paw) * 16.0;
            let rot = wave.to_radians();
            let (sn, cs) = (rot.sin(), rot.cos());
            let pt = |dx: f32, dy: f32| (px + dx * cs - dy * sn, py + dx * sn + dy * cs);
            let (bx, by) = pt(0.0, -1.0);
            cv.ellipse(bx, by, 3.9, 3.5, WHITE, paw);
            for (dx, dy, rr) in [(-3.5, -5.2, 1.45), (-0.9, -6.6, 1.45), (1.9, -6.5, 1.45), (4.0, -4.8, 1.35)] {
                let (tx, ty) = pt(dx, dy);
                cv.circle(tx, ty, rr, WHITE, paw);
            }
        }
        // zZ
        if cm.sleep {
            let t = (cm.clock % 2.4) / 2.4;
            cv.text(CX + 20.0 + t * 3.0, cy - 1.0 - t * 5.0, "z", 8.0, W::Bold, 0xcfe0ff, (1.0 - t) * a);
            let t2 = ((cm.clock + 1.2) % 2.4) / 2.4;
            cv.text(CX + 26.0 + t2 * 3.0, cy - 5.0 - t2 * 5.0, "z", 6.0, W::Bold, 0xcfe0ff, (1.0 - t2) * a);
        }
        // hearts
        let hearts = cm.hearts.clone();
        let clock = cm.clock;
        for (hx, t0, _rot) in hearts {
            let t = clock - t0;
            if t < 0.0 { continue; }
            let p = t / 1.1;
            let al = if p < 0.2 { p / 0.2 } else { 1.0 - (p - 0.2) / 0.8 };
            cv.text(CX + hx, h - 2.0 - p * 24.0, "♥", 10.0 * (0.6 + 0.5 * p.min(1.0)), W::Bold, 0xff7ab0, clamp01(al));
        }
        cv.alpha = 1.0;
    }

    /* ---------- header / nav / dots ---------- */

    fn title_parts(&self) -> (&'static str, &'static str) {
        if self.page == "water" && self.water_edit { return ("Settings", ""); }
        match self.page {
            "calendar" => ("Calendar", " Today"),
            "water" => ("Water", " Reminder"),
            "todo" => ("Todo", "List"),
            "pomodoro" => ("Pomodoro", ""),
            _ => ("", ""),
        }
    }

    fn draw_header(&mut self, app: &App, cv: &mut Canvas) {
        let s = &app.d.settings;
        let accent = self.accent();
        let base = 29.0;
        let mut x = FX + 26.0;
        let media = self.page == "media";
        if media { x = FX + 150.0; }
        if !media {
            let saved = cv.alpha;
            if self.page_t < 1.0 { cv.alpha = saved * smooth(0.0, 1.0, self.page_t); }
            if self.page == "water" && self.water_edit {
                cv.circle(x + 11.0, 23.0, 11.0, 0x2a2a2d, 1.0);
                cv.icon("left", icons::LEFT, x + 5.0, 17.0, 12.0, WHITE, 1.0);
                self.hit(cv, x, 12.0, 22.0, 22.0, Act::WaterEdit(false));
                x += 29.0;
            } else if s.mascot {
                cv.icon("dog", icons::DOG, x - 1.0, 11.0, 24.0, WHITE, 1.0);
                self.hit(cv, x, 11.0, 24.0, 24.0, Act::Pet);
                x += 29.0;
            }
            let (a, b) = self.title_parts();
            x += cv.text(x, base, a, 17.0, W::Bold, WHITE, 1.0);
            if !b.is_empty() { x += cv.text(x, base, b, 17.0, W::Bold, accent, 1.0); }
            cv.alpha = saved;
            x += 16.0;
        }
        // battery, date, time
        if s.show_battery {
            if let Some((lvl, charging)) = self.battery {
                if charging { self.bolt_badge(cv, x + 9.5, 23.0, 19.0); x += 27.0; }
                else {
                    cv.rrect(x, 17.5, 22.0, 11.0, 3.0, 0x8a8a90, 1.0);
                    cv.rrect(x + 1.5, 19.0, 19.0, 8.0, 2.0, 0x000000, 1.0);
                    let c = if lvl <= 0.2 { 0xff453a } else { WHITE };
                    cv.rrect(x + 2.5, 20.0, 17.0 * lvl, 6.0, 1.2, c, 1.0);
                    cv.rrect(x + 22.5, 21.0, 2.0, 4.0, 1.0, 0x8a8a90, 1.0);
                    x += 28.0;
                    x += cv.text(x, base - 1.0, &format!("{}%", (lvl * 100.0).round()), 12.0, W::Regular, MUTED, 1.0) + 8.0;
                }
            }
        }
        let now = now_ms();
        let l = time::local(now);
        if s.show_date {
            let d = format!("{} {} {} {}", time::WD[l.weekday as usize], l.day, time::MONTHS[l.month as usize - 1], l.year);
            x += cv.text(x, base - 1.0, &d, 14.0, W::Regular, WHITE, 1.0) + 8.0;
        }
        let (t, ap) = time::clock(now, !s.clock24);
        x += cv.text(x, base - 1.0, &t, 14.0, W::Bold, WHITE, 1.0);
        if !ap.is_empty() { cv.text(x + 3.0, base - 1.0, ap, 9.5, W::Bold, WHITE, 1.0); }
        // water sub-page dots (top right)
        if self.page == "water" && !self.water_edit {
            let rx = FX + EW - 26.0;
            for i in 0..2 {
                let on = (self.water_sub.target > 0.5) == (i == 1);
                cv.circle(rx - 9.0 + i as f32 * 9.0, 22.0, 2.5, if on { WHITE } else { 0x4a4a4e }, 1.0);
            }
            self.hit(cv, rx - 16.0, 14.0, 22.0, 16.0, Act::WaterSub);
        }
    }

    fn accent(&self) -> u32 {
        if self.page == "media" { self.art_accent.unwrap_or(accent_of("media")) } else { accent_of(self.page) }
    }

    fn draw_nav(&mut self, app: &App, cv: &mut Canvas) {
        if Self::visible_pages(app).len() < 2 { return; }
        for (dir, x) in [(-1, FX + 12.0), (1, FX + EW - 34.0)] {
            let hov = self.hov(cv, x, 80.0, 22.0, 22.0);
            let r = if hov { 12.0 } else { 11.0 };
            cv.circle(x + 11.0, 91.0, r, if hov { 0x3a3a3e } else { 0x2a2a2d }, 1.0);
            let ic = if dir < 0 { icons::LEFT } else { icons::RIGHT };
            cv.icon(if dir < 0 { "left" } else { "right" }, ic, x + 5.0, 85.0, 12.0, 0xe5e5ea, 1.0);
            self.hit(cv, x - 4.0, 76.0, 30.0, 30.0, Act::Nav(dir));
        }
    }

    fn draw_dots(&mut self, app: &App, cv: &mut Canvas) {
        let list = Self::visible_pages(app);
        let ws: Vec<f32> = list.iter().map(|p| {
            let on = *p == self.page;
            let key = format!("dot{}", p);
            self.tween(&key, if on { 18.0 } else { 4.0 }, if on { 18.0 } else { 4.0 }, 3.0, 0.75)
        }).collect();
        let total: f32 = ws.iter().sum::<f32>() + 4.0 * (ws.len() as f32 - 1.0);
        let mut x = CX - total / 2.0;
        let y = EH - 13.0;
        for (i, p) in list.iter().enumerate() {
            let on = *p == self.page;
            cv.rrect(x, y, ws[i], 4.0, 2.0, if on { 0xe6e6e6 } else { 0x4a4a4e }, 1.0);
            self.hit(cv, x - 3.0, y - 6.0, ws[i] + 6.0, 16.0, Act::Goto(p));
            x += ws[i] + 4.0;
        }
    }

    fn chip(&mut self, cv: &mut Canvas, x: f32, y: f32, label: &str, icon: Option<(&'static str, &'static str, u32)>, bg: u32, fg: u32, act: Act) -> f32 {
        let tw = cv.measure(label, 12.5, W::Semi);
        let iw = if icon.is_some() { 14.0 + if label.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
        let w = if label.is_empty() { 30.0 } else { 24.0 + iw + tw };
        let hov = self.hov(cv, x, y, w, 30.0);
        cv.rrect(x, y, w, 30.0, 15.0, if hov { lighten(bg) } else { bg }, 1.0);
        let mut cx = if label.is_empty() { x + 8.0 } else { x + 12.0 };
        if let Some((n, svg, c)) = icon { cv.icon(n, svg, cx, y + 8.0, 14.0, c, 1.0); cx += 20.0; }
        if !label.is_empty() { cv.text(cx, y + 19.5, label, 12.5, W::Semi, fg, 1.0); }
        self.hit(cv, x, y, w, 30.0, act);
        w
    }

    fn draw_page(&mut self, app: &App, cv: &mut Canvas, interactive: bool) {
        let saved_hits = self.hits.len();
        match self.page {
            "water" => if self.water_edit { self.page_water_edit(app, cv) } else { self.page_water(app, cv) },
            "todo" => self.page_todo(app, cv),
            "pomodoro" => self.page_pomodoro(app, cv),
            "calendar" => self.page_calendar(app, cv),
            "media" => self.page_media(app, cv),
            _ => {}
        }
        if !interactive { self.hits.truncate(saved_hits); }
    }


    /// intersect the current mask with a rounded-rect region (cached when there is no shell mask)
    fn push_clip(&mut self, cv: &mut Canvas, key: &str, x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Mask> {
        let saved = cv.mask.take();
        let tf = Transform::from_row(cv.s, 0.0, 0.0, cv.s, cv.ox * cv.s, cv.oy * cv.s);
        let make = |pw: u32, ph: u32| -> Option<Mask> {
            let mut m = Mask::new(pw, ph)?;
            m.fill_path(&rrect_path(x, y, w, h, [r; 4])?, FillRule::Winding, true, tf);
            Some(m)
        };
        let (pw, ph) = (cv.pm.width(), cv.pm.height());
        match &saved {
            None if cv.ox != 0.0 || cv.oy != 0.0 => { cv.mask = make(pw, ph); }
            None => {
                let ck = format!("{}:{}", key, cv.s);
                if !self.clip_cache.contains_key(&ck) {
                    if self.clip_cache.len() > 4 { self.clip_cache.clear(); }
                    if let Some(m) = make(pw, ph) { self.clip_cache.insert(ck.clone(), m); }
                }
                cv.mask = self.clip_cache.get(&ck).cloned();
            }
            Some(base) => {
                let mut m = make(pw, ph);
                if let Some(mm) = m.as_mut() {
                    for (a, b) in mm.data_mut().iter_mut().zip(base.data()) { *a = ((*a as u16 * *b as u16) / 255) as u8; }
                }
                cv.mask = m;
            }
        }
        saved
    }

    fn divider(&mut self, cv: &mut Canvas, x: f32) {
        cv.rrect(x, PY + 9.0, 1.0, PH - 18.0, 0.5, LINE, 1.0);
    }

    /* ---------- water ---------- */

    fn page_water(&mut self, app: &App, cv: &mut Canvas) {
        let wtr = &app.d.water;
        let today = app.water_today();
        let pct = (today as f32 / wtr.goal as f32 * 100.0).round() as i32;
        let lw = PW - 150.0 - 32.0 - 1.0;
        let y0 = PY + 7.0;
        // big number
        let mut x = PX;
        x += cv.text(x, y0 + 28.0, &liters(today), 34.0, W::Bold, WHITE, 1.0);
        cv.text(x + 5.0, y0 + 28.0, &format!("/ {} L", liters(wtr.goal)), 14.0, W::Semi, MUTED, 1.0);
        // reminder toggle
        let tx = PX + lw - 42.0;
        let on = wtr.enabled;
        let knob = self.tween("wtoggle", if on { 1.0 } else { 0.0 }, if on { 1.0 } else { 0.0 }, 3.2, 0.7);
        cv.rrect(tx, y0 + 6.0, 42.0, 24.0, 12.0, blend(0x3a3a3c, 0x1a8cff, clamp01(knob)), 1.0);
        let kx = tx + 2.0 + 18.0 * knob;
        cv.circle(kx + 10.0, y0 + 18.0, 10.0, WHITE, 1.0);
        cv.icon("bell", icons::BELL, kx + 4.5, y0 + 12.5, 11.0, if on { 0x1a8cff } else { 0x8a8a8e }, 1.0);
        self.hit(cv, tx, y0 + 6.0, 42.0, 24.0, Act::App(Action::WaterToggle));
        // subline
        let next = if !wtr.enabled { "Reminders off".to_string() } else if today >= wtr.goal { "Goal reached!".into() } else { format!("Next {}", time::hm24(app.water_next_at)) };
        let sy = y0 + 51.0;
        let mut sx = PX;
        sx += cv.text(sx, sy, "Today · ", 13.0, W::Regular, MUTED, 1.0);
        sx += cv.text(sx, sy, &format!("{}%", pct), 13.0, W::Bold, WATER, 1.0);
        cv.text(sx, sy, &format!(" · {}", next), 13.0, W::Regular, MUTED, 1.0);
        // progress bar
        let by = y0 + 59.0;
        cv.rrect(PX, by, lw, 5.0, 2.5, 0x23272d, 1.0);
        let bw = self.tween("wbar", (pct.min(100) as f32 / 100.0) * lw, 0.0, 1.6, 0.82);
        cv.rrect(PX, by, bw.max(0.0), 5.0, 2.5, WATER, 1.0);
        // buttons
        let byy = y0 + 76.0;
        let w1 = self.chip(cv, PX, byy, "Drink", Some(("drop", icons::DROP, WATER)), 0x11385a, WHITE, Act::App(Action::Drink));
        self.chip(cv, PX + w1 + 7.0, byy, "Settings", Some(("sliders", icons::SLIDERS, WHITE)), CHIP, WHITE, Act::WaterEdit(true));
        // divider + right slider (gauge ↔ chart)
        let dx = PX + lw + 16.0;
        self.divider(cv, dx);
        let rx = dx + 17.0;
        let sub = self.water_sub.x;
        let saved_mask = if sub > 0.001 && sub < 0.999 { Some(self.push_clip(cv, "wslide", rx, PY, 150.0, PH, 14.0)) } else { None };
        let gx = rx - 150.0 * sub;
        if sub < 0.999 { self.gauge(cv, gx + 75.0, PY + PH / 2.0, today as f32 / wtr.goal as f32, pct); }
        if sub > 0.001 { self.water_chart(app, cv, gx + 150.0, PY + 14.0); }
        if let Some(m) = saved_mask { cv.mask = m; }
        self.hit(cv, rx, PY, 150.0, PH, Act::WaterSub);
    }

    fn gauge(&mut self, cv: &mut Canvas, cx: f32, cy: f32, p: f32, pct: i32) {
        let r = 45.0;
        cv.arc(cx, cy, r, 135.0, 270.0, 0x13293d, 1.0, 8.0);
        let v = self.tween("gauge", clamp01(p), 0.0, 1.3, 0.82);
        if v > 0.002 { cv.arc(cx, cy, r, 135.0, 270.0 * v.min(1.0), WATER, 1.0, 8.0); }
        cv.text_center(cx, cy + 6.0, &format!("{}", pct), 29.0, W::Bold, WHITE, 1.0);
        let pw = cv.measure("%", 11.0, W::Regular);
        cv.icon("drop", icons::DROP, cx - (pw + 13.0) / 2.0, cy + 12.0, 11.0, WATER, 1.0);
        cv.text(cx - (pw + 13.0) / 2.0 + 13.0, cy + 21.5, "%", 11.0, W::Regular, 0xdddddd, 1.0);
    }

    fn water_chart(&mut self, app: &App, cv: &mut Canvas, x: f32, y: f32) {
        let wtr = &app.d.water;
        let from = wtr.start_hour;
        let to = wtr.end_hour.max(from + 1);
        let mut buckets = std::collections::HashMap::new();
        for e in app.today_entries() { *buckets.entry(time::local(e.t).hour as i32).or_insert(0) += e.ml; }
        let vals: Vec<(i32, i32)> = (from..to).map(|h| (h, *buckets.get(&h).unwrap_or(&0))).collect();
        let peak = vals.iter().map(|v| v.1).max().unwrap_or(0);
        let max = 400.max(((peak + 199) / 200) * 200);
        let last = app.today_entries().last().map(|e| time::local(e.t).hour as i32);
        let label_w = cv.measure(&format!("{}", max), 9.0, W::Regular).max(cv.measure("0 ml", 9.0, W::Regular)) + 6.0;
        let plot_w = 146.0 - label_w;
        let ph = 88.0;
        let n = vals.len() as f32;
        let gap = 3.0;
        let cw = ((plot_w - gap * (n - 1.0)) / n).max(2.0);
        let mut bubble: Option<(f32, f32, i32)> = None;
        for (i, (hh, ml)) in vals.iter().enumerate() {
            let bx = x + i as f32 * (cw + gap);
            cv.rrect(bx, y, cw, ph, (cw / 2.0).min(6.0), 0x161619, 1.0);
            if *ml > 0 {
                let target = (*ml as f32 / max as f32).max(0.09) * ph;
                let hgt = self.tween(&format!("wb{}", hh), target, 0.0, 1.6, 0.72).max(0.0);
                if let Some(p) = rrect_path(bx, y + ph - hgt, cw, hgt, [(cw / 2.0).min(6.0); 4]) {
                    let g = LinearGradient::new(Point::from_xy(0.0, (y + cv.oy) * cv.s), Point::from_xy(0.0, (y + ph + cv.oy) * cv.s),
                        vec![GradientStop::new(0.0, rgb(0x5fd0ff, cv.alpha)), GradientStop::new(1.0, rgb(0x2a8cff, cv.alpha))], SpreadMode::Pad, Transform::identity());
                    if let Some(g) = g { cv.fill_shader(&p, g); }
                }
                if Some(*hh) == last { bubble = Some((bx + cw / 2.0, y + ph - hgt, *ml)); }
            }
        }
        if let Some((bx, by, ml)) = bubble {
            let t = format!("{} ml", ml);
            let tw = cv.measure(&t, 10.0, W::Bold) + 14.0;
            let pop = self.tween("bub", 1.0, 0.0, 2.4, 0.6);
            let a = clamp01(pop);
            let yy = by - 22.0 + (1.0 - pop) * 6.0;
            let sv = cv.alpha;
            cv.alpha *= a;
            cv.rrect(bx - tw / 2.0, yy, tw, 18.0, 9.0, 0x1d8cf0, 1.0);
            cv.text_center(bx, yy + 13.0, &t, 10.0, W::Bold, WHITE, 1.0);
            cv.alpha = sv;
        }
        let lx = x + plot_w + 6.0;
        cv.text(lx, y + 7.0, &format!("{}", max), 9.0, W::Regular, MUTED, 1.0);
        cv.text(lx, y + ph / 2.0 + 3.0, &format!("{}", max / 2), 9.0, W::Regular, MUTED, 1.0);
        cv.text(lx, y + ph, "0 ml", 9.0, W::Regular, MUTED, 1.0);
        let mut h = from;
        while h < to {
            let i = (h - from) as f32;
            cv.text_center(x + i * (cw + gap) + cw / 2.0, y + ph + 12.0, &format!("{:02}", h), 9.0, W::Regular, MUTED, 1.0);
            h += 3;
        }
    }

    fn page_water_edit(&mut self, app: &App, cv: &mut Canvas) {
        let w = &app.d.water;
        let sip = app.water_sip();
        let per_bottle = ((w.bottle as f32 / sip as f32).round() as i32).max(1);
        let refills = ((w.goal as f32 / w.bottle as f32).round() as i32).max(1);
        let rows_w = PW - 132.0 - 14.0;
        let y0 = PY + 12.0;
        let rb = |ui: &mut Ui, cv: &mut Canvas, x: f32, y: f32, plus: bool, act: Act| {
            let hov = ui.hov(cv, x, y, 24.0, 24.0);
            cv.circle(x + 12.0, y + 12.0, 12.0, if hov { 0x3a3a3e } else { 0x2a2a2d }, 1.0);
            cv.rrect(x + 7.0, y + 11.0, 10.0, 2.0, 1.0, WHITE, 1.0);
            if plus { cv.rrect(x + 11.0, y + 7.0, 2.0, 10.0, 1.0, WHITE, 1.0); }
            ui.hit(cv, x - 2.0, y - 2.0, 28.0, 28.0, act);
        };
        // Bottle
        let ry = y0;
        cv.text(PX, ry + 17.0, "Bottle", 13.5, W::Regular, 0xe5e5ea, 1.0);
        let mut x = PX + 62.0;
        rb(self, cv, x, ry + 2.0, false, Act::App(Action::WaterBottle(-50)));
        x += 24.0;
        cv.text_center(x + 30.0, ry + 18.0, &format!("{} ml", w.bottle), 14.0, W::Bold, WHITE, 1.0);
        x += 60.0;
        rb(self, cv, x, ry + 2.0, true, Act::App(Action::WaterBottle(50)));
        x += 30.0;
        let small = cv.ellipsize(&format!("{} ml each · {}/bottle", sip, per_bottle), 10.5, W::Regular, PX + rows_w - x);
        cv.text(x, ry + 17.0, &small, 10.5, W::Regular, MUTED, 1.0);
        // Goal chips
        let ry = y0 + 37.0;
        cv.text(PX, ry + 17.0, "Goal", 13.5, W::Regular, 0xe5e5ea, 1.0);
        let mut x = PX + 62.0;
        for g in [1500, 2000, 2500, 3000] {
            let on = w.goal == g;
            let label = format!("{:.1} L", g as f32 / 1000.0);
            let tw = cv.measure(&label, 12.5, W::Semi) + 22.0;
            let hov = self.hov(cv, x, ry + 1.0, tw, 26.0);
            cv.rrect(x, ry + 1.0, tw, 26.0, 13.0, if on { 0x1a8cff } else if hov { 0x2a2a2d } else { 0x1c1c1f }, 1.0);
            cv.text(x + 11.0, ry + 18.5, &label, 12.5, W::Semi, if on || hov { WHITE } else { MUTED }, 1.0);
            self.hit(cv, x, ry + 1.0, tw, 26.0, Act::App(Action::WaterGoal(g)));
            x += tw + 6.0;
        }
        // Hours
        let ry = y0 + 74.0;
        cv.text(PX, ry + 17.0, "Hours", 13.5, W::Regular, 0xe5e5ea, 1.0);
        let mut x = PX + 62.0;
        rb(self, cv, x, ry + 2.0, false, Act::App(Action::WaterHours(true, -1)));
        x += 24.0;
        cv.text_center(x + 27.0, ry + 18.0, &format!("{:02}:00", w.start_hour), 14.0, W::Bold, WHITE, 1.0);
        x += 54.0;
        rb(self, cv, x, ry + 2.0, true, Act::App(Action::WaterHours(true, 1)));
        x += 30.0;
        cv.text(x, ry + 17.0, "to", 12.0, W::Regular, MUTED, 1.0);
        x += 18.0;
        rb(self, cv, x, ry + 2.0, false, Act::App(Action::WaterHours(false, -1)));
        x += 24.0;
        cv.text_center(x + 27.0, ry + 18.0, &format!("{:02}:00", w.end_hour % 24), 14.0, W::Bold, WHITE, 1.0);
        x += 54.0;
        rb(self, cv, x, ry + 2.0, true, Act::App(Action::WaterHours(false, 1)));
        // side card
        let sx = PX + PW - 132.0;
        let hov = self.hov(cv, sx, PY + 8.0, 132.0, 82.0);
        cv.rrect(sx, PY + 8.0, 132.0, 82.0, 16.0, if hov { 0x143450 } else { 0x10283c }, 1.0);
        cv.text_center(sx + 66.0, PY + 35.0, &format!("{}", app.water_reminders()), 26.0, W::Bold, 0x64d2ff, 1.0);
        cv.text_center(sx + 66.0, PY + 49.0, &format!("reminders · every {} min", w.interval), 10.0, W::Regular, 0xaac8de, 1.0);
        cv.rrect(sx + 22.0, PY + 56.0, 88.0, 1.0, 0.5, 0x2a4a63, 1.0);
        cv.text_center(sx + 66.0, PY + 74.0, &format!("{}", refills), 18.0, W::Bold, 0x64d2ff, 1.0);
        cv.text_center(sx + 66.0, PY + 86.0, "refills", 10.0, W::Regular, 0xaac8de, 1.0);
        self.hit(cv, sx, PY + 8.0, 132.0, 82.0, Act::WaterIntervalCycle);
        let hov = self.hov(cv, sx, PY + 96.0, 132.0, 24.0);
        cv.rrect(sx, PY + 96.0, 132.0, 24.0, 12.0, if hov { 0x2a2a2d } else { 0x1c1c1f }, 1.0);
        cv.icon("trash", icons::TRASH, sx + 26.0, PY + 101.0, 13.0, 0xd1d1d6, 1.0);
        cv.text(sx + 44.0, PY + 112.5, "Reset today", 11.5, W::Regular, 0xd1d1d6, 1.0);
        self.hit(cv, sx, PY + 96.0, 132.0, 24.0, Act::App(Action::WaterReset));
    }

    /* ---------- todo ---------- */

    fn page_todo(&mut self, app: &App, cv: &mut Canvas) {
        let all = &app.d.todos;
        let done = all.iter().filter(|t| t.done).count();
        let y0 = PY + 14.0;
        let w = cv.text(PX, y0 + 28.0, &format!("{}/{}", done, all.len()), 34.0, W::Bold, WHITE, 1.0);
        let cx = PX + w + 10.0;
        let today = time::day_key(now_ms());
        cv.circle(cx + 15.0, y0 + 15.0, 15.0, if self.hov(cv, cx, y0, 30.0, 30.0) { CHIP_H } else { CHIP }, 1.0);
        cv.icon("cal", icons::CAL_OUTLINE, cx + 7.5, y0 + 7.5, 15.0, 0xcfcfd4, 1.0);
        if app.d.events.iter().any(|e| e.date == today) { cv.circle(cx + 25.0, y0 + 4.0, 4.0, 0x000000, 1.0); cv.circle(cx + 25.0, y0 + 4.0, 3.0, 0x0a84ff, 1.0); }
        self.hit(cv, cx, y0, 30.0, 30.0, Act::Goto("calendar"));
        cv.text(PX, y0 + 50.0, "Task Done Today", 13.0, W::Regular, MUTED, 1.0);
        let by = y0 + 64.0;
        let open = self.todo_form.open;
        let w1 = self.chip(cv, PX, by, "Add Task", Some(("plusc", icons::PLUS_CIRCLE, if open { GREEN } else { WHITE })), if open { 0x12361d } else { CHIP }, if open { GREEN } else { WHITE }, Act::FormOpen(FieldId::Todo));
        let edit = self.edit_todos;
        self.chip(cv, PX + w1 + 7.0, by, "Edit Task", Some(("pencil", icons::PENCIL, if edit { WHITE } else { 0x6c6c70 })), if edit { 0x3a3a3e } else { CHIP }, if edit { WHITE } else { 0x6c6c70 }, Act::EditTodos);
        // card
        let kx = PX + 206.0;
        let kw = PW - 206.0;
        let ky = PY + 6.0;
        cv.rrect(kx, ky, kw, 112.0, 18.0, CARD, 1.0);
        if open {
            self.todo_form_ui(cv, kx + 10.0, ky + 10.0, kw - 20.0, FieldId::Todo);
            return;
        }
        if all.is_empty() {
            cv.icon("tasks", icons::TASKS, kx + kw / 2.0 - 17.0, ky + 26.0, 34.0, 0x3a3a3d, 1.0);
            cv.text_center(kx + kw / 2.0, ky + 82.0, "No tasks.", 13.5, W::Regular, 0x6e6e73, 1.0);
            return;
        }
        let mut sorted: Vec<&crate::data::Todo> = all.iter().filter(|t| !t.done).collect();
        sorted.extend(all.iter().filter(|t| t.done));
        let row = 24.0;
        let max_scroll = ((sorted.len() as f32) * (row + 1.0) - 96.0).max(0.0);
        self.todo_scroll = self.todo_scroll.min(max_scroll);
        let overflow = sorted.len() as f32 * (row + 1.0) > 98.0;
        let saved = if overflow { Some(self.push_clip(cv, "tlist", kx, ky + 6.0, kw, 100.0, 0.0)) } else { None };
        let mut y = ky + 8.0 - self.todo_scroll;
        for t in sorted {
            if y > ky + 112.0 { break; }
            if y + row >= ky {
                let hov = self.hov(cv, kx + 6.0, y, kw - 12.0, row);
                if hov { cv.rrect(kx + 6.0, y, kw - 12.0, row, 8.0, 0x222225, 1.0); }
                let ccx = kx + 18.0;
                let ccy = y + row / 2.0;
                if t.done {
                    cv.circle(ccx, ccy, 8.0, GREEN, 1.0);
                    cv.icon("check", icons::CHECK, ccx - 5.0, ccy - 5.0, 10.0, 0x000000, 1.0);
                } else {
                    cv.arc(ccx, ccy, 7.1, 0.0, 360.0, if hov { GREEN } else { 0x5a5a5e }, 1.0, 1.8);
                }
                self.hit(cv, kx + 6.0, y, 26.0, row, Act::App(Action::TodoToggle(t.id.clone())));
                let mut right = kx + kw - 14.0;
                if self.edit_todos {
                    let hv = self.hov(cv, right - 16.0, y + 4.0, 16.0, 16.0);
                    cv.icon("x", icons::XMARK, right - 15.0, y + 5.0, 14.0, if hv { 0xff6961 } else { MUTED }, 1.0);
                    self.hit(cv, right - 20.0, y, 24.0, row, Act::App(Action::TodoDelete(t.id.clone())));
                    right -= 22.0;
                }
                if let Some(r) = t.remind_at.as_deref().and_then(time::parse_iso) {
                    let s = format!("{}{}", if t.daily { "↻ " } else { "" }, time::hm24(r));
                    let sw = cv.measure(&s, 10.5, W::Regular);
                    cv.text(right - sw, y + 16.0, &s, 10.5, W::Regular, MUTED, 1.0);
                    right -= sw + 8.0;
                } else if t.daily {
                    cv.text(right - 8.0, y + 16.0, "↻", 10.5, W::Regular, MUTED, 1.0);
                    right -= 16.0;
                }
                let tx = kx + 32.0;
                let label = cv.ellipsize(&t.text, 12.5, W::Regular, right - tx);
                let col = if t.done { 0x6e6e73 } else { WHITE };
                let lw = cv.text(tx, y + 16.5, &label, 12.5, W::Regular, col, 1.0);
                if t.done { cv.rrect(tx, y + 12.0, lw, 1.0, 0.5, 0x6e6e73, 1.0); }
            }
            y += row + 1.0;
        }
        if let Some(m) = saved { cv.mask = m; }
    }

    fn todo_form_ui(&mut self, cv: &mut Canvas, x: f32, y: f32, w: f32, f: FieldId) {
        let (text, tm, daily) = {
            let form = if f == FieldId::Todo { &self.todo_form } else { &self.event_form };
            (form.text.clone(), form.time, form.daily)
        };
        // title input
        let focused = self.focus == Some(f);
        if let Some(p) = rrect_path(x, y, w, 30.0, [12.0; 4]) { cv.stroke(&p, if focused { 0xd8d8dc } else { 0x5a5a5e }, 1.0, 1.5); }
        cv.circle(x + 14.0, y + 15.0, 4.5, if f == FieldId::Todo { GREEN } else { 0xff5a5f }, 1.0);
        let tx = x + 26.0;
        let placeholder = if f == FieldId::Todo { "Task title" } else { "Event title" };
        if text.is_empty() {
            cv.text(tx, y + 19.5, placeholder, 13.0, W::Regular, 0x6e6e73, 1.0);
            if focused && (now_ms() / 530) % 2 == 0 { cv.rrect(tx, y + 8.0, 1.5, 15.0, 0.5, WHITE, 1.0); }
        } else {
            // keep the end of long text visible
            let avail = w - 36.0;
            let mut shown = text.clone();
            while cv.measure(&shown, 13.0, W::Regular) > avail && !shown.is_empty() { shown.remove(0); }
            let tw = cv.text(tx, y + 19.5, &shown, 13.0, W::Regular, WHITE, 1.0);
            if focused && (now_ms() / 530) % 2 == 0 { cv.rrect(tx + tw + 1.0, y + 8.0, 1.5, 15.0, 0.5, WHITE, 1.0); }
        }
        self.hit(cv, x, y, w, 30.0, Act::Field(f));
        // time row
        let ry = y + 36.0;
        cv.rrect(x, ry, w, 26.0, 10.0, 0x202023, 1.0);
        cv.icon("clock", icons::CLOCK, x + 8.0, ry + 5.5, 15.0, 0xc7c7cc, 1.0);
        let mut cx = x + 30.0;
        let small_btn = |ui: &mut Ui, cv: &mut Canvas, bx: f32, label: &str, act: Act| {
            let hov = ui.hov(cv, bx, ry + 3.0, 20.0, 20.0);
            cv.circle(bx + 10.0, ry + 13.0, 10.0, if hov { 0x3a3a3e } else { 0x2c2c2f }, 1.0);
            cv.text_center(bx + 10.0, ry + 17.5, label, 13.0, W::Bold, WHITE, 1.0);
            ui.hit(cv, bx, ry, 22.0, 26.0, act);
        };
        small_btn(self, cv, cx, "−", Act::FormTime(-1));
        cx += 24.0;
        let ts = tm.map(|(h, m)| format!("{:02}:{:02}", h, m)).unwrap_or_else(|| "--:--".into());
        cv.text_center(cx + 20.0, ry + 17.5, &ts, 12.5, W::Semi, if tm.is_some() { WHITE } else { 0x8e8e93 }, 1.0);
        cx += 40.0;
        small_btn(self, cv, cx, "+", Act::FormTime(1));
        cx += 28.0;
        if f == FieldId::Todo {
            let lw = cv.measure("Daily", 11.0, W::Semi) + 18.0;
            let dx = x + w - lw - 4.0;
            cv.rrect(dx, ry + 3.0, lw, 20.0, 10.0, if daily { 0x0a84ff } else { 0x2c2c2f }, 1.0);
            cv.text(dx + 9.0, ry + 17.0, "Daily", 11.0, W::Semi, if daily { WHITE } else { 0xaeaeb2 }, 1.0);
            self.hit(cv, dx, ry, lw, 26.0, Act::FormDaily);
        } else {
            let day = self.event_form.day;
            let label = match day { 0 => "Today".to_string(), 1 => "Tomorrow".into(), d => {
                let l = time::local(time::start_of_local_day(now_ms()) + d as i64 * 86_400_000 + 43_200_000);
                format!("{} {} {}", time::WD[l.weekday as usize], l.day, time::MONTHS[l.month as usize - 1])
            } };
            let right = x + w - 4.0;
            small_btn(self, cv, right - 20.0, "+", Act::FormDay(1));
            let lw = cv.measure(&label, 11.5, W::Semi);
            let lx = (right - 26.0 - lw).max(cx);
            cv.text(lx, ry + 17.0, &label, 11.5, W::Semi, WHITE, 1.0);
            small_btn(self, cv, lx - 26.0, "−", Act::FormDay(-1));
        }
        // actions
        let ay = y + 68.0;
        let bw = (w - 8.0) / 2.0;
        let hov = self.hov(cv, x, ay, bw, 26.0);
        cv.rrect(x, ay, bw, 26.0, 13.0, if hov { 0x3a3a3e } else { 0x2c2c2f }, 1.0);
        cv.text_center(x + bw / 2.0, ay + 17.5, "Cancel", 12.5, W::Semi, WHITE, 1.0);
        self.hit(cv, x, ay, bw, 26.0, Act::FormCancel);
        let hov = self.hov(cv, x + bw + 8.0, ay, bw, 26.0);
        cv.rrect(x + bw + 8.0, ay, bw, 26.0, 13.0, if hov { 0x48d26c } else { GREEN }, 1.0);
        cv.text_center(x + bw + 8.0 + bw / 2.0, ay + 17.5, "Save", 12.5, W::Semi, 0x04220e, 1.0);
        self.hit(cv, x + bw + 8.0, ay, bw, 26.0, Act::FormSave);
    }

    /* ---------- pomodoro ---------- */

    fn page_pomodoro(&mut self, app: &App, cv: &mut Canvas) {
        let p = &app.pomo;
        let c = p.phase.color();
        let rem = app.pomo_remaining();
        let total = app.phase_ms(p.phase).max(1);
        let cx = PX + 4.0 + 52.0;
        let cy = PY + PH / 2.0;
        cv.arc(cx, cy, 44.0, 0.0, 360.0, c, 0.18, 9.0);
        let frac = self.tween("pring", clamp01(rem as f32 / total as f32), 0.0, 1.1, 0.9);
        if frac > 0.002 {
            // glow: wide faint strokes under the ring
            for (wd, al) in [(22.0, 0.07), (16.0, 0.12), (12.0, 0.2)] { cv.arc(cx, cy, 44.0, -90.0, 360.0 * frac, c, al, wd); }
            cv.arc(cx, cy, 44.0, -90.0, 360.0 * frac, c, 1.0, 9.0);
        }
        let tilt = if p.running && (now_ms() / 1000) % 2 == 0 { 0.0 } else { 0.0 };
        let _ = tilt;
        cv.icon("stopwatch", icons::STOPWATCH, cx - 23.0, cy - 23.0, 46.0, WHITE, 1.0);
        // right column, centered
        let rx0 = PX + 136.0;
        let rw = PW - 136.0;
        let mid = rx0 + rw / 2.0;
        let ts = time::mmss(rem);
        let tw = cv.measure(&ts, 36.0, W::Bold);
        let tx = mid - (tw + 36.0) / 2.0;
        cv.text(tx, PY + 44.0, &ts, 36.0, W::Bold, WHITE, 1.0);
        let bx = tx + tw + 8.0;
        let hov = self.hov(cv, bx, PY + 16.0, 28.0, 28.0);
        cv.circle(bx + 14.0, PY + 30.0, 14.0, if hov { CHIP_H } else { CHIP }, 1.0);
        cv.icon("reset", icons::RESET, bx + 7.0, PY + 23.0, 14.0, WHITE, 1.0);
        self.hit(cv, bx, PY + 16.0, 28.0, 28.0, Act::App(Action::PomoReset));
        cv.text_center(mid, PY + 64.0, &format!("{} · round {}/{}", p.phase.label(), p.round, app.d.pomodoro.rounds), 13.0, W::Regular, MUTED, 1.0);
        // buttons row
        let l1 = "Short Break";
        let l2 = "Focus Time";
        let w1 = 24.0 + 20.0 + cv.measure(l1, 12.5, W::Semi);
        let w2 = 24.0 + 20.0 + cv.measure(l2, 12.5, W::Semi);
        let total_w = w1 + 64.0 + w2 + 14.0;
        let mut x = mid - total_w / 2.0;
        let by = PY + 76.0;
        x += self.chip(cv, x, by, l1, Some(("coffee", icons::COFFEE, WHITE)), if p.phase == Phase::Short { 0x3a3a3e } else { CHIP }, WHITE, Act::App(Action::PomoPhase(Phase::Short))) + 7.0;
        let hov = self.hov(cv, x, by, 64.0, 30.0);
        cv.rrect(x, by, 64.0, 30.0, 15.0, if hov { lighten(c) } else { c }, 1.0);
        if p.running {
            cv.icon("pause", icons::PAUSE, x + 25.0, by + 8.0, 14.0, WHITE, 1.0);
        } else {
            cv.icon("play", icons::PLAY, x + 25.0, by + 8.0, 14.0, WHITE, 1.0);
        }
        self.hit(cv, x, by, 64.0, 30.0, Act::App(Action::PomoToggle));
        x += 64.0 + 7.0;
        self.chip(cv, x, by, l2, Some(("target", icons::TARGET, WHITE)), if p.phase == Phase::Focus { 0x3a3a3e } else { CHIP }, WHITE, Act::App(Action::PomoPhase(Phase::Focus)));
    }

    /* ---------- calendar ---------- */

    fn page_calendar(&mut self, app: &App, cv: &mut Canvas) {
        let now = now_ms();
        let l = time::local(now);
        let accent = accent_of("calendar");
        let lw = PW - 150.0 - 33.0;
        if self.event_form.open {
            cv.rrect(PX - 4.0, PY + 4.0, lw + 8.0, 116.0, 16.0, CARD, 1.0);
            self.todo_form_ui(cv, PX + 4.0, PY + 12.0, lw - 8.0, FieldId::Event);
        } else {
            let (t, ap) = time::clock(now, !app.d.settings.clock24);
            let w = cv.text(PX, PY + 40.0, &t, 34.0, W::Bold, WHITE, 1.0);
            if !ap.is_empty() { cv.text(PX + w + 5.0, PY + 40.0, ap, 13.0, W::Bold, WHITE, 1.0); }
            cv.text(PX, PY + 60.0, &format!("{}, {} {}", time::WD_LONG[l.weekday as usize], l.day, time::MONTHS_LONG[l.month as usize - 1]), 13.0, W::Regular, MUTED, 1.0);
            let mut evs: Vec<(i64, &crate::data::Event)> = app.d.events.iter().filter_map(|e| {
                let (y, m, d) = time::parse_day_key(&e.date)?;
                let (hh, mm) = parse_hm(&e.time).unwrap_or((23, 59));
                let st = time::from_local(y, m, d, hh, mm);
                (st >= now - 30 * 60_000).then_some((st, e))
            }).collect();
            evs.sort_by_key(|e| e.0);
            let mut y = PY + 72.0;
            if evs.is_empty() {
                cv.rrect(PX, y + 3.0, 3.0, 13.0, 1.5, 0x444444, 1.0);
                cv.text(PX + 10.0, y + 14.0, "No upcoming events", 12.5, W::Regular, MUTED, 1.0);
                y += 19.0;
            }
            for (_, e) in evs.iter().take(2) {
                let today = time::day_key(now);
                let tmr = time::day_key(now + 86_400_000);
                let day = if e.date == today { "Today".to_string() } else if e.date == tmr { "Tomorrow".into() } else {
                    time::parse_day_key(&e.date).map(|(_, m, d)| format!("{} {}", d, time::MONTHS[m as usize - 1])).unwrap_or_default()
                };
                let meta = format!(" · {}{}", day, if e.time.is_empty() { String::new() } else { format!(" {}", e.time) });
                cv.rrect(PX, y + 3.0, 3.0, 13.0, 1.5, accent, 1.0);
                let mw = cv.measure(&meta, 12.5, W::Regular);
                let title = cv.ellipsize(&e.title, 12.5, W::Regular, lw - 10.0 - mw - 18.0);
                let tw = cv.text(PX + 10.0, y + 14.0, &title, 12.5, W::Regular, WHITE, 1.0);
                cv.text(PX + 10.0 + tw, y + 14.0, &meta, 12.5, W::Regular, MUTED, 1.0);
                let hv = self.hov(cv, PX, y, lw, 19.0);
                if hv {
                    cv.icon("x", icons::XMARK, PX + lw - 14.0, y + 3.0, 13.0, MUTED, 1.0);
                    self.hit(cv, PX + lw - 18.0, y, 20.0, 19.0, Act::App(Action::EventDelete(e.id.clone())));
                }
                y += 19.0;
            }
            // add event
            let ay = PY + 112.0;
            if evs.len() < 2 || true {
                let label = "+ Event";
                let tw = cv.measure(label, 11.5, W::Semi) + 16.0;
                let ax = PX + lw - tw;
                let hov = self.hov(cv, ax, PY + 23.0, tw, 22.0);
                cv.rrect(ax, PY + 23.0, tw, 22.0, 11.0, if hov { CHIP_H } else { CHIP }, 1.0);
                cv.text(ax + 8.0, PY + 38.0, label, 11.5, W::Semi, WHITE, 1.0);
                self.hit(cv, ax, PY + 23.0, tw, 22.0, Act::FormOpen(FieldId::Event));
            }
            let _ = ay;
        }
        // divider + month grid
        let dx = PX + lw + 16.0;
        self.divider(cv, dx);
        let gx = dx + 17.0;
        let gy = PY + 10.0;
        cv.text(gx, gy + 11.0, time::MONTHS_LONG[l.month as usize - 1], 11.5, W::Bold, WHITE, 1.0);
        cv.text_right(gx + 150.0, gy + 11.0, &format!("{}", l.year), 11.5, W::Regular, MUTED, 1.0);
        let cw = 150.0 / 7.0;
        for (i, d) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().enumerate() {
            cv.text_center(gx + cw * (i as f32 + 0.5), gy + 26.0, d, 8.5, W::Bold, 0x6d6d72, 1.0);
        }
        let first = time::weekday_of(l.year, l.month, 1);
        let offset = (first + 6) % 7;
        let days = time::days_in_month(l.year, l.month);
        let (py, pm) = if l.month == 1 { (l.year - 1, 12) } else { (l.year, l.month - 1) };
        let prev_days = time::days_in_month(py, pm);
        let cells = ((offset + days + 6) / 7) * 7;
        let evdays: std::collections::HashSet<&str> = app.d.events.iter().map(|e| e.date.as_str()).collect();
        let rows = cells / 7;
        let rh = if rows > 5 { 13.0 } else { 14.0 };
        for i in 0..cells {
            let col = (i % 7) as f32;
            let row = (i / 7) as f32;
            let ccx = gx + cw * (col + 0.5);
            let cy = gy + 31.0 + row * rh;
            let dn = i as i32 - offset as i32 + 1;
            if dn < 1 {
                cv.text_center(ccx, cy + 10.0, &format!("{}", prev_days as i32 + dn), 10.0, W::Regular, 0x3a3a3e, 1.0);
            } else if dn as u32 > days {
                cv.text_center(ccx, cy + 10.0, &format!("{}", dn as u32 - days), 10.0, W::Regular, 0x3a3a3e, 1.0);
            } else {
                let today = dn as u32 == l.day;
                if today { cv.rrect(ccx - 10.0, cy, 20.0, rh, rh / 2.0, accent, 1.0); }
                cv.text_center(ccx, cy + 10.0, &format!("{}", dn), 10.0, if today { W::Bold } else { W::Regular }, WHITE, 1.0);
                let key = format!("{:04}-{:02}-{:02}", l.year, l.month, dn);
                if evdays.contains(key.as_str()) { cv.circle(ccx, cy + rh - 0.5, 1.5, if today { WHITE } else { accent }, 1.0); }
            }
        }
    }

    /* ---------- media ---------- */

    fn page_media(&mut self, app: &App, cv: &mut Canvas) {
        let m = &app.media;
        let ok = m.ok && (!m.title.is_empty() || !m.artist.is_empty());
        let accent = self.accent();
        let ax = PX;
        let ay = 12.0;
        if let (true, Some(img)) = (ok, self.art.clone()) {
            for (g, al) in [(10.0, 0.08), (6.0, 0.12), (3.0, 0.16)] {
                cv.rrect(ax - g / 2.0, ay + 4.0 - g / 2.0 + 3.0, 84.0 + g, 84.0 + g, 16.0 + g / 2.0, accent, al);
            }
            cv.image(ax, ay, 84.0, 16.0, &img);
        } else {
            cv.rrect(ax, ay, 84.0, 84.0, 16.0, 0x1f1f22, 1.0);
            cv.icon("note", icons::NOTE, ax + 25.0, ay + 25.0, 34.0, 0x5a5a5e, 1.0);
        }
        let ix = ax + 98.0;
        let iw = PW - 98.0;
        let title = if ok { if m.title.is_empty() { "Unknown title".to_string() } else { m.title.clone() } } else { "Nothing playing".into() };
        let t = cv.ellipsize(&title, 16.0, W::Bold, iw);
        cv.text(ix, ay + 52.0, &t, 16.0, W::Bold, WHITE, 1.0);
        let lyric = if ok && self.show_lyrics { current_lyric(app) } else { None };
        let sub = if let Some(l) = &lyric { l.clone() } else if ok {
            [m.artist.clone(), app_name(&m.app)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
        } else { "Play something in Spotify, YouTube… to control it here".into() };
        let sub = cv.ellipsize(&sub, 12.5, if lyric.is_some() { W::Semi } else { W::Regular }, iw);
        if lyric.is_some() {
            let t = clamp01((self.clock - self.lyric_swap) / 0.45);
            let e = ease_out(t);
            let sv = cv.alpha;
            cv.alpha *= e;
            cv.text(ix, ay + 70.0 + (1.0 - e) * 6.0, &sub, 12.5, W::Semi, lighten(accent), 1.0);
            cv.alpha = sv;
        } else {
            cv.text(ix, ay + 70.0, &sub, 12.5, W::Regular, MUTED, 1.0);
        }
        // progress
        let py = ay + 84.0 + 18.0;
        let dim = if ok { 1.0 } else { 0.45 };
        let (pos, end) = (m.position(), m.end);
        cv.text(PX, py + 4.5, &if end > 0 { time::mmss(pos) } else { "00:00".into() }, 13.0, W::Regular, WHITE, dim);
        cv.text_right(PX + PW, py + 4.5, &if end > 0 { format!("-{}", time::mmss(end - pos)) } else { "--:--".into() }, 13.0, W::Regular, WHITE, dim);
        let tx = PX + 54.0;
        let tw = PW - 54.0 - 58.0;
        let hov = self.hov(cv, tx, py - 8.0, tw, 16.0);
        let th = if hov { 6.0 } else { 4.0 };
        cv.rrect(tx, py - th / 2.0, tw, th, th / 2.0, 0x3a3a3e, dim);
        if end > 0 { cv.rrect(tx, py - th / 2.0, tw * (pos as f32 / end as f32), th, th / 2.0, 0xe5e5ea, dim); }
        self.hit(cv, tx, py - 8.0, tw, 16.0, Act::Seek(tx + cv.ox, tw));
        // controls
        let cy = py + 26.0;
        let mb = |ui: &mut Ui, cv: &mut Canvas, x: f32, name: &'static str, svg: &'static str, size: f32, color: u32, act: Act| {
            let hov = ui.hov(cv, x - 16.0, cy - 16.0, 32.0, 32.0);
            if hov { cv.circle(x, cy, 16.0, 0x222225, 1.0); }
            cv.icon(name, svg, x - size / 2.0, cy - size / 2.0, size, color, dim);
            ui.hit(cv, x - 16.0, cy - 16.0, 32.0, 32.0, act);
        };
        mb(self, cv, PX + 16.0, "quote", icons::QUOTE, 19.0, if self.show_lyrics { accent } else { 0xc7c7cc }, Act::Lyrics);
        let mid = CX;
        mb(self, cv, mid - 52.0, "back", icons::BACK, 22.0, WHITE, Act::Media("prev"));
        let playing = m.ok && m.playing;
        mb(self, cv, mid, if playing { "pause" } else { "play" }, if playing { icons::PAUSE } else { icons::PLAY }, 24.0, WHITE, Act::Media("toggle"));
        mb(self, cv, mid + 52.0, "fwd", icons::FWD, 22.0, WHITE, Act::Media("next"));
        mb(self, cv, PX + PW - 16.0, if self.muted { "mute" } else { "vol" }, if self.muted { icons::MUTE } else { icons::VOL }, 19.0, 0xc7c7cc, Act::Media("vol"));
    }

    /* ---------- artwork ---------- */

    pub fn set_art(&mut self, app: &App) {
        if app.media.art_key == self.art_key { return; }
        self.art_key = app.media.art_key.clone();
        self.art = None;
        self.art_accent = None;
        if let Some((size, data)) = &app.media.art {
            if let Some(mut pm) = Pixmap::new(*size, *size) {
                if pm.data().len() == data.len() {
                    pm.data_mut().copy_from_slice(data);
                    self.art_accent = vivid_color(data);
                    self.art = Some(pm);
                }
            }
        }
    }
}

pub fn current_lyric(app: &App) -> Option<String> {
    if !app.d.settings.lyrics || app.lyrics.key != app.media.key || app.lyrics.lines.is_empty() { return None; }
    let pos = app.media.position() + 250;
    let mut line = None;
    for (t, s) in &app.lyrics.lines { if *t <= pos { line = Some(s.clone()); } else { break; } }
    Some(line.filter(|s| !s.is_empty()).unwrap_or_else(|| "♪".into()))
}

fn app_name(id: &str) -> String {
    let s = id.to_lowercase();
    for (k, v) in [("spotify", "Spotify"), ("chrome", "Chrome"), ("msedge", "Edge"), ("firefox", "Firefox"), ("zune", "Media Player"), ("media", "Media Player"), ("apple", "Apple Music"), ("itunes", "Apple Music")] {
        if s.contains(k) { return v.into(); }
    }
    String::new()
}

fn vivid_color(rgba: &[u8]) -> Option<u32> {
    let mut best = None;
    let mut best_score = -1.0f32;
    for px in rgba.chunks(4).step_by(37) {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        if mx < 60.0 { continue; }
        let sat = (mx - mn) / mx;
        let score = sat * 0.7 + mx / 255.0 * 0.3;
        if score > best_score { best_score = score; best = Some(((r as u32) << 16) | ((g as u32) << 8) | b as u32); }
    }
    best
}

pub fn liters(ml: i32) -> String {
    let l = ml as f32 / 1000.0;
    if ml % 1000 == 0 { format!("{:.1}", l) } else {
        let s = format!("{:.2}", l);
        s.trim_end_matches('0').to_string()
    }
}

fn lighten(c: u32) -> u32 { blend(c, 0xffffff, 0.12) }

fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| -> f32 { ((a >> s) & 255) as f32 * (1.0 - t) + ((b >> s) & 255) as f32 * t };
    ((ch(16) as u32) << 16) | ((ch(8) as u32) << 8) | ch(0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Data;

    #[test]
    fn water_alert_stays_until_the_user_snoozes() {
        let mut app = App::new(Data::default());
        app.d.settings.mascot = false;
        let mut ui = Ui::new(&app);
        ui.alert(&app, "water", "water", "Time to drink water".into(), String::new());

        ui.clock = ui.alert.as_ref().unwrap().until + 1.0;
        let mut fonts = Fonts::load();
        ui.update(&mut app, &mut fonts, 0.1);
        assert!(ui.alert.is_some());

        ui.hits.push(Hit { r: (0.0, 0.0, 10.0, 10.0), act: Act::AlertSnooze });
        let out = ui.click(&mut app, (5.0, 5.0));

        assert_eq!(out.last(), Some(&Out::App(Action::WaterSnooze)));
        let alert = ui.alert.as_ref().unwrap();
        assert!(alert.snoozed);
        assert!(alert.done_at >= 0.0);
    }

    #[test]
    fn clicking_the_water_alert_body_does_not_dismiss_it() {
        let mut app = App::new(Data::default());
        let mut ui = Ui::new(&app);
        ui.alert(&app, "water", "water", "Time to drink water".into(), String::new());
        ui.hits.push(Hit { r: (0.0, 0.0, 10.0, 10.0), act: Act::AlertOpen });

        ui.click(&mut app, (5.0, 5.0));

        assert!(ui.alert.is_some());
        assert!(!ui.expanded);
    }
}
