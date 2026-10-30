//! Platform-independent app state and logic (ported from the Electron main process).

use crate::data::*;
use crate::time::{self, now_ms};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase { Focus, Short, Long }
impl Phase {
    pub fn label(self) -> &'static str {
        match self { Phase::Focus => "Focus time", Phase::Short => "Short break", Phase::Long => "Long break" }
    }
    pub fn color(self) -> u32 {
        match self { Phase::Focus => 0xff453a, Phase::Short => 0x34c759, Phase::Long => 0x4cc2ff }
    }
}

pub struct Pomo { pub phase: Phase, pub running: bool, pub ends_at: i64, pub remaining: i64, pub round: i32 }

#[derive(Clone, Default, Debug)]
pub struct Media {
    pub ok: bool,
    pub title: String,
    pub artist: String,
    pub app: String,
    pub playing: bool,
    pub pos: i64,
    pub end: i64,
    pub upd: i64, // local receive time of `pos`
    pub key: String,
    /// premultiplied RGBA, square
    pub art: Option<(u32, Vec<u8>)>,
    pub art_key: String,
}
impl Media {
    pub fn position(&self) -> i64 {
        if !self.ok || self.end <= 0 { return 0; }
        let mut p = self.pos;
        if self.playing { p += now_ms() - self.upd; }
        p.clamp(0, self.end)
    }
}

#[derive(Clone, Default)]
pub struct Lyrics { pub key: String, pub lines: Vec<(i64, String)> }

#[derive(Debug, Clone)]
pub enum Effect {
    Notify { title: String, body: String },
    Peek { page: &'static str, force: bool },
    Alert { kind: &'static str, page: &'static str, title: String, body: String },
    Chime(&'static str),
    Mood(&'static str),
    Reposition,
    Autostart,
    Visibility,
}

pub struct App {
    pub d: Data,
    pub pomo: Pomo,
    pub water_next_at: i64,
    pub media: Media,
    pub lyrics: Lyrics,
    pub effects: Vec<Effect>,
    pub dirty_save: bool,
    pub changed: bool,
    last_day: String,
}

fn uid() -> String {
    let n = now_ms();
    let r = (n.wrapping_mul(6364136223846793005) >> 20) & 0xfffff;
    format!("{:x}{:05x}", n, r)
}

pub const PAGE_IDS: [&str; 5] = ["media", "calendar", "water", "todo", "pomodoro"];
pub const WATER_SNOOZE_MS: i64 = 10 * 60_000;
pub const WATER_SNOOZE_MINUTES: i64 = 10;

impl App {
    pub fn new(d: Data) -> App {
        let focus = d.pomodoro.focus as i64 * 60_000;
        let mut a = App {
            d,
            pomo: Pomo { phase: Phase::Focus, running: false, ends_at: 0, remaining: focus, round: 1 },
            water_next_at: 0,
            media: Media::default(),
            lyrics: Lyrics::default(),
            effects: vec![],
            dirty_save: false,
            changed: true,
            last_day: time::day_key(now_ms()),
        };
        let cutoff = time::day_key(now_ms() - 60 * 86_400_000);
        a.d.water.log.retain(|k, _| k.as_str() >= cutoff.as_str());
        a.d.pomodoro.stats.retain(|k, _| k.as_str() >= cutoff.as_str());
        a.rollover_todos();
        let last = a.today_entries().last().map(|e| e.t).unwrap_or(now_ms());
        a.schedule_water(last);
        a
    }

    fn save(&mut self) { self.dirty_save = true; self.changed = true; }

    /* ---------------- water ---------------- */

    pub fn today_entries(&self) -> &[Sip] {
        self.d.water.log.get(&time::day_key(now_ms())).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn water_today(&self) -> i32 { self.today_entries().iter().map(|e| e.ml).sum() }
    pub fn water_reminders(&self) -> i32 {
        let w = &self.d.water;
        (((w.end_hour - w.start_hour) * 60) / w.interval.max(1)).max(1)
    }
    pub fn water_sip(&self) -> i32 {
        let w = &self.d.water;
        if w.cup > 0 { w.cup } else { ((w.goal as f32 / self.water_reminders() as f32).round() as i32).max(10) }
    }
    fn clamp_active(&self, t: i64) -> i64 {
        let w = &self.d.water;
        let l = time::local(t);
        let h = l.hour as f32 + l.minute as f32 / 60.0;
        if h < w.start_hour as f32 {
            time::from_local(l.year, l.month, l.day, w.start_hour as u32, 0)
        } else if h >= w.end_hour as f32 {
            let next = time::start_of_local_day(t) + 86_400_000 + 3_600_000; // safe across DST
            let n = time::local(next);
            time::from_local(n.year, n.month, n.day, w.start_hour as u32, 0)
        } else { t }
    }
    fn schedule_water(&mut self, from: i64) {
        self.water_next_at = self.clamp_active(from + self.d.water.interval as i64 * 60_000);
    }

    /* ---------------- todos ---------------- */

    fn rollover_todos(&mut self) {
        let today = time::day_key(now_ms());
        if self.d.todo_day == today { return; }
        self.d.todo_day = today.clone();
        self.d.todos.retain(|t| t.daily || !t.done || t.done_at.map(|x| time::day_key(x) == today).unwrap_or(false));
        for t in self.d.todos.iter_mut().filter(|t| t.daily) {
            t.done = false;
            t.notified = false;
            if let Some(r) = t.remind_at.as_deref().and_then(time::parse_iso) {
                let rl = time::local(r);
                let n = time::local(now_ms());
                t.remind_at = Some(time::to_iso(time::from_local(n.year, n.month, n.day, rl.hour, rl.minute)));
            }
        }
        self.save();
    }

    /* ---------------- pomodoro ---------------- */

    pub fn phase_ms(&self, p: Phase) -> i64 {
        let c = &self.d.pomodoro;
        (match p { Phase::Focus => c.focus, Phase::Short => c.short, Phase::Long => c.long }) as i64 * 60_000
    }
    pub fn pomo_remaining(&self) -> i64 {
        if self.pomo.running { (self.pomo.ends_at - now_ms()).max(0) } else { self.pomo.remaining }
    }
    fn pomo_advance(&mut self, completed: bool) {
        if self.pomo.phase == Phase::Focus {
            if completed {
                *self.d.pomodoro.stats.entry(time::day_key(now_ms())).or_insert(0) += 1;
                self.save();
            }
            self.pomo.phase = if self.pomo.round >= self.d.pomodoro.rounds { Phase::Long } else { Phase::Short };
        } else {
            self.pomo.round = if self.pomo.phase == Phase::Long { 1 } else { self.pomo.round + 1 };
            self.pomo.phase = Phase::Focus;
        }
        self.pomo.remaining = self.phase_ms(self.pomo.phase);
        self.pomo.running = completed && self.d.pomodoro.auto_start;
        self.pomo.ends_at = if self.pomo.running { now_ms() + self.pomo.remaining } else { 0 };
    }
    pub fn pomo_today(&self) -> i32 { *self.d.pomodoro.stats.get(&time::day_key(now_ms())).unwrap_or(&0) }

    /* ---------------- notifications ---------------- */

    fn notify(&mut self, title: &str, body: String, page: &'static str, kind: &'static str) {
        let body_copy = body.clone();
        let s = &self.d.settings;
        if s.sys_notif { self.effects.push(Effect::Notify { title: title.into(), body }); }
        if s.sounds { self.effects.push(Effect::Chime(kind)); }
        self.effects.push(Effect::Mood("alert"));
        if s.peek { self.effects.push(Effect::Alert { kind, page, title: title.to_string(), body: body_copy }); }
    }

    /// Called about once a second. Returns the ms until the next deadline we care about,
    /// so the platform layer can sleep instead of polling.
    pub fn tick(&mut self) -> i64 {
        let now = now_ms();
        let day = time::day_key(now);
        if day != self.last_day { self.last_day = day; self.changed = true; }
        self.rollover_todos();

        if self.pomo.running && now >= self.pomo.ends_at {
            let was_focus = self.pomo.phase == Phase::Focus;
            self.pomo_advance(true);
            if was_focus {
                let m = self.phase_ms(self.pomo.phase) / 60_000;
                self.notify("Focus session done!", format!("Take a {}-minute break.", m), "pomodoro", "pomo");
            } else {
                self.notify("Break is over", "Time to focus again!".into(), "pomodoro", "pomo");
            }
            self.changed = true;
        }

        if self.d.water.enabled && self.water_next_at > 0 && now >= self.water_next_at {
            let today = self.water_today();
            if today < self.d.water.goal {
                let body = format!("Today: {} / {} ml. Have {} ml!", today, self.d.water.goal, self.water_sip());
                self.notify("Time to drink water", body, "water", "water");
            }
            self.schedule_water(now);
            self.changed = true;
        }

        let mut fire: Vec<String> = vec![];
        for t in self.d.todos.iter_mut() {
            if t.done || t.notified { continue; }
            if let Some(r) = t.remind_at.as_deref().and_then(time::parse_iso) {
                if now >= r { t.notified = true; fire.push(t.text.clone()); }
            }
        }
        for f in fire { self.notify("To-do reminder", f, "todo", "todo"); self.save(); }

        let lead = self.d.settings.event_lead as i64 * 60_000;
        let mut fire: Vec<(String, String)> = vec![];
        for e in self.d.events.iter_mut() {
            if e.notified { continue; }
            let Some((y, m, d)) = time::parse_day_key(&e.date) else { e.notified = true; continue };
            let (hh, mm) = parse_hm(&e.time).unwrap_or((9, 0));
            let start = time::from_local(y, m, d, hh, mm);
            if now >= start - lead && now < start + 30 * 60_000 {
                e.notified = true;
                let mins = ((start - now) / 60_000).max(0);
                let body = if mins > 0 { format!("Starts in {} min ({})", mins, e.time) } else if e.time.is_empty() { "Starting now (all day)".into() } else { format!("Starting now ({})", e.time) };
                fire.push((e.title.clone(), body));
            } else if now >= start + 30 * 60_000 { e.notified = true; }
        }
        if !fire.is_empty() { self.save(); }
        for (t, b) in fire { self.notify(&t, b, "calendar", "event"); }

        // next deadline
        let mut next = 60_000 - (now % 60_000); // minute boundary (clock)
        if self.pomo.running { next = next.min(1000 - now % 1000); }
        if self.water_next_at > now { next = next.min(self.water_next_at - now); }
        next.max(50)
    }

    /* ---------------- actions ---------------- */

    pub fn action(&mut self, a: &Action) {
        use Action::*;
        let now = now_ms();
        match a {
            Drink => {
                let ml = self.water_sip();
                self.d.water.log.entry(time::day_key(now)).or_default().push(Sip { t: now, ml });
                self.schedule_water(now);
                self.effects.push(Effect::Mood("happy"));
            }
            WaterSnooze => { self.water_next_at = self.clamp_active(now + WATER_SNOOZE_MS); }
            WaterUndo => { if let Some(v) = self.d.water.log.get_mut(&time::day_key(now)) { v.pop(); } }
            WaterReset => { self.d.water.log.remove(&time::day_key(now)); self.schedule_water(now); }
            WaterToggle => { self.d.water.enabled = !self.d.water.enabled; self.schedule_water(now); }
            WaterGoal(g) => { self.d.water.goal = (*g).clamp(250, 6000); }
            WaterBottle(d) => { self.d.water.bottle = (self.d.water.bottle + d).clamp(100, 3000); }
            WaterHours(start, d) => {
                let w = &mut self.d.water;
                if *start { w.start_hour = (w.start_hour + d).clamp(0, 23); } else { w.end_hour = (w.end_hour + d).clamp(1, 24); }
                if w.end_hour <= w.start_hour { w.end_hour = (w.start_hour + 1).min(24); }
                self.schedule_water(now);
            }
            WaterInterval(m) => { self.d.water.interval = (*m).clamp(10, 240); self.schedule_water(now); }
            WaterCup(c) => { self.d.water.cup = *c; }
            WaterView(chart) => { self.d.settings.water_view = if *chart { "chart".into() } else { "gauge".into() }; }

            TodoAdd { text, time: tm, daily } => {
                let text: String = text.trim().chars().take(200).collect();
                if text.is_empty() { return; }
                let remind_at = tm.map(|(h, m)| {
                    let l = time::local(now);
                    let mut t = time::from_local(l.year, l.month, l.day, h, m);
                    if t < now && *daily { t += 86_400_000; }
                    time::to_iso(t)
                });
                self.d.todos.push(Todo { id: uid(), text, remind_at, daily: *daily, created_at: now, ..Default::default() });
            }
            TodoToggle(id) => {
                if let Some(t) = self.d.todos.iter_mut().find(|t| &t.id == id) {
                    t.done = !t.done;
                    t.done_at = if t.done { Some(now) } else { None };
                    if t.done { self.effects.push(Effect::Mood("happy")); }
                }
            }
            TodoDelete(id) => self.d.todos.retain(|t| &t.id != id),
            TodoClearDone => self.d.todos.retain(|t| !t.done || t.daily),

            PomoToggle => {
                if self.pomo.running {
                    self.pomo.remaining = (self.pomo.ends_at - now).max(0);
                    self.pomo.running = false;
                } else {
                    if self.pomo.remaining <= 0 { self.pomo.remaining = self.phase_ms(self.pomo.phase); }
                    self.pomo.ends_at = now + self.pomo.remaining;
                    self.pomo.running = true;
                }
            }
            PomoReset => { self.pomo.running = false; self.pomo.remaining = self.phase_ms(self.pomo.phase); }
            PomoResetAll => { self.pomo = Pomo { phase: Phase::Focus, running: false, ends_at: 0, remaining: self.phase_ms(Phase::Focus), round: 1 }; }
            PomoPhase(p) => { self.pomo.phase = *p; self.pomo.running = false; self.pomo.remaining = self.phase_ms(*p); }
            PomoSkip => self.pomo_advance(false),
            PomoSet(which, v) => {
                let c = &mut self.d.pomodoro;
                match *which { 0 => c.focus = *v, 1 => c.short = *v, 2 => c.long = *v, 3 => c.rounds = *v, _ => c.auto_start = *v != 0 }
                if !self.pomo.running { self.pomo.remaining = self.phase_ms(self.pomo.phase); }
            }

            EventAdd { title, date, time: tm } => {
                let title: String = title.trim().chars().take(120).collect();
                if title.is_empty() { return; }
                let time = tm.map(|(h, m)| format!("{:02}:{:02}", h, m)).unwrap_or_default();
                self.d.events.push(Event { id: uid(), title, date: date.clone(), time, notified: false });
                self.d.events.sort_by(|a, b| (a.date.clone() + &a.time).cmp(&(b.date.clone() + &b.time)));
            }
            EventDelete(id) => self.d.events.retain(|e| &e.id != id),
            EventClearPast => { let today = time::day_key(now); self.d.events.retain(|e| e.date >= today); }

            SetBool(key, v) => {
                let s = &mut self.d.settings;
                match *key {
                    "startWithWindows" => { s.start_with_windows = *v; self.effects.push(Effect::Autostart); }
                    "minimal" => s.minimal = *v,
                    "mascot" => s.mascot = *v,
                    "sounds" => s.sounds = *v,
                    "hideFullscreen" => { s.hide_fullscreen = *v; self.effects.push(Effect::Visibility); }
                    "autoHide" => s.auto_hide = *v,
                    "hidden" => { s.hidden = *v; self.effects.push(Effect::Visibility); }
                    "hoverExpand" => s.hover_expand = *v,
                    "clock24" => s.clock24 = *v,
                    "showBattery" => s.show_battery = *v,
                    "showDate" => s.show_date = *v,
                    "peek" => s.peek = *v,
                    "sysNotif" => s.sys_notif = *v,
                    "mediaPeek" => s.media_peek = *v,
                    "mediaPill" => s.media_pill = *v,
                    "lyrics" => s.lyrics = *v,
                    _ => {}
                }
            }
            SetPage(id, v) => self.d.settings.pages.set(id, *v),
            SetIdle(v) => self.d.settings.idle_after = *v,
            SetEventLead(v) => self.d.settings.event_lead = *v,
            SetPeekSeconds(v) => self.d.settings.peek_seconds = *v,
            SetCollapse(v) => self.d.settings.collapse_delay = *v,
            WaterGoalAbs(v) => self.d.water.goal = (*v).clamp(250, 6000),
            WaterBottleAbs(v) => self.d.water.bottle = (*v).clamp(100, 3000),
            SetSize(v) => { self.d.settings.size = match v { Some(n) => serde_json::json!(n), None => serde_json::json!("auto") }; self.effects.push(Effect::Reposition); }
            SetDisplay(v) => { self.d.settings.display = v.clone(); self.effects.push(Effect::Reposition); }
            LastPage(p) => { self.d.settings.page = serde_json::json!(p); self.dirty_save = true; return; }
        }
        self.save();
    }
}

pub fn parse_hm(s: &str) -> Option<(u32, u32)> {
    let (a, b) = s.split_once(':')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_snooze_defers_the_next_reminder_without_logging_a_drink() {
        let mut app = App::new(Data::default());
        let expected = app.clamp_active(now_ms() + WATER_SNOOZE_MS);

        app.action(&Action::WaterSnooze);

        assert!((app.water_next_at - expected).abs() <= 1000);
        assert_eq!(app.water_today(), 0);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Drink, WaterSnooze, WaterUndo, WaterReset, WaterToggle, WaterGoal(i32), WaterBottle(i32), WaterHours(bool, i32),
    WaterInterval(i32), WaterCup(i32), WaterView(bool),
    TodoAdd { text: String, time: Option<(u32, u32)>, daily: bool }, TodoToggle(String), TodoDelete(String), TodoClearDone,
    PomoToggle, PomoReset, PomoResetAll, PomoPhase(Phase), PomoSkip, PomoSet(u8, i32),
    EventAdd { title: String, date: String, time: Option<(u32, u32)> }, EventDelete(String), EventClearPast,
    SetBool(&'static str, bool), SetPage(&'static str, bool), SetIdle(u32), SetEventLead(u32), SetPeekSeconds(u32), SetCollapse(u32), WaterGoalAbs(i32), WaterBottleAbs(i32),
    SetSize(Option<u32>), SetDisplay(String), LastPage(&'static str),
}
