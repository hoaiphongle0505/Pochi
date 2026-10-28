//! Linux-only preview: renders notch states to PNG and benchmarks frame cost.
use crate::app::*;
use crate::data::*;
use crate::gfx::*;
use crate::time::{self, now_ms};
use crate::ui::*;
#[allow(unused_imports)] use crate::ui::Out;
use tiny_skia::Pixmap;

fn sample() -> App {
    let mut d = Data::default();
    let now = now_ms();
    let l = time::local(now);
    let at = |h, m| time::from_local(l.year, l.month, l.day, h, m);
    d.water.start_hour = 9; d.water.end_hour = 18; d.water.interval = 15;
    d.water.log.insert(time::day_key(now), vec![Sip { t: at(9, 10), ml: 42 }, Sip { t: at(10, 5), ml: 42 }, Sip { t: at(11, 20), ml: 84 }, Sip { t: at(12, 40), ml: 188 }, Sip { t: at(13, 40), ml: 168 }]);
    for (i, (t, done)) in [("Send the weekly report", false), ("Stretch for 5 minutes", false), ("Team stand-up", true), ("Water the plants", false)].iter().enumerate() {
        d.todos.push(Todo { id: format!("t{}", i), text: t.to_string(), done: *done, daily: i == 1, remind_at: if i == 0 { Some(time::to_iso(now + 3_600_000)) } else { None }, ..Default::default() });
    }
    d.events.push(Event { id: "e1".into(), title: "Team weekly".into(), date: time::day_key(now), time: "17:30".into(), notified: false });
    d.events.push(Event { id: "e2".into(), title: "Design review".into(), date: time::day_key(now + 86_400_000), time: "09:00".into(), notified: false });
    let mut a = App::new(d);
    a.media = Media { ok: true, title: "Midnight Drive".into(), artist: "Demo Artist".into(), app: "Spotify.exe".into(), playing: true, pos: 64_000, end: 213_000, upd: now, key: "k".into(), art: Some(art()), art_key: "k".into() };
    a.lyrics = Lyrics { key: "k".into(), lines: vec![(0, "Intro".into()), (60_000, "City lights are fading into blue".into()), (90_000, "next".into())] };
    a
}

fn art() -> (u32, Vec<u8>) {
    let n = 96u32;
    let mut v = vec![0u8; (n * n * 4) as usize];
    for y in 0..n { for x in 0..n {
        let i = ((y * n + x) * 4) as usize;
        let d = (((x as f32 - 48.0).powi(2) + (y as f32 - 48.0).powi(2)).sqrt() < 24.0) as u8;
        let (r, g, b) = if d == 1 { (255, 210, 120) } else { (255 - y as u8 / 2, 90 + y as u8 / 3, 140 + y as u8 / 2) };
        v[i] = r; v[i + 1] = g; v[i + 2] = b; v[i + 3] = 255;
    } }
    (n, v)
}

pub fn settings_preview(out: &str) {
    use crate::settings::*;
    let scale = 1.5f32;
    let mut fonts = Fonts::load();
    let mut icons = Icons::new();
    let mut app = sample();
    let mut su = SettingsUi::new("general");
    su.monitors = vec![Monitor { id: "main".into(), label: "Display 1 (1920×1080) · main".into(), selected: true }, Monitor { id: "2".into(), label: "Display 2 (1920×1080)".into(), selected: false }];
    su.data_path = "C:\\Users\\you\\AppData\\Roaming\\Pochi\\pochi-data.json".into();
    su.mouse = (330.0, 150.0);
    let mut pm = Pixmap::new((SW * scale) as u32, (SH * scale) as u32).unwrap();
    for sec in ["general", "display", "appearance", "media", "todo", "calendar", "pomodoro", "water", "hud", "about"] {
        su.goto(sec);
        for _ in 0..90 {
            su.animating(1.0 / 60.0);
            let mut cv = Canvas { pm: &mut pm, s: scale, mask: None, alpha: 1.0, ox: 0.0, oy: 0.0, fonts: &mut fonts, icons: &mut icons };
            let t0 = std::time::Instant::now();
            su.draw(&mut app, &mut cv);
            let _ = t0;
        }
        let t0 = std::time::Instant::now();
        let mut cv = Canvas { pm: &mut pm, s: scale, mask: None, alpha: 1.0, ox: 0.0, oy: 0.0, fonts: &mut fonts, icons: &mut icons };
        su.draw(&mut app, &mut cv);
        println!("settings {:<10} {:.2} ms", sec, t0.elapsed().as_secs_f64() * 1000.0);
        pm.save_png(format!("{}/s-{}.png", out, sec)).unwrap();
    }
}

pub fn alert_frames(out: &str) {
    let scale = 2.0f32;
    let mut fonts = Fonts::load();
    let mut icons = Icons::new();
    let mut app = sample();
    app.d.settings.idle_after = 0;
    app.media.playing = false;
    let mut ui = Ui::new(&app);
    ui.battery = Some((0.82, true));
    let mut pm = Pixmap::new((WIN_W * scale) as u32, (WIN_H * scale) as u32).unwrap();
    let mut n = 0;
    let mut step = |ui: &mut Ui, app: &mut App, pm: &mut Pixmap, secs: f32, save: bool| {
        for _ in 0..(secs * 30.0) as i32 {
            ui.update(app, &mut fonts, 1.0 / 30.0);
            let mut cv = Canvas { pm, s: scale, mask: None, alpha: 1.0, ox: 0.0, oy: 0.0, fonts: &mut fonts, icons: &mut icons };
            ui.draw(app, &mut cv);
            if save { pm.save_png(format!("{}/f{:04}.png", out, n)).unwrap(); n += 1; }
        }
    };
    step(&mut ui, &mut app, &mut pm, 0.6, true);
    ui.alert(&app, "water", "water", "Time to drink water".into(), "".into());
    step(&mut ui, &mut app, &mut pm, 3.2, true);
    // press the +ml button
    let p = (WIN_W / 2.0 + 180.0 - 50.0, 30.0);
    let outs = ui.click(&mut app, p);
    for o in outs { if let Out::App(a) = o { app.action(&a); } }
    step(&mut ui, &mut app, &mut pm, 2.6, true);
    ui.alert(&app, "todo", "todo", "To-do reminder".into(), "Send the weekly report".into());
    step(&mut ui, &mut app, &mut pm, 2.4, true);
    ui.alert = None;
    step(&mut ui, &mut app, &mut pm, 0.8, true);
    ui.expand();
    step(&mut ui, &mut app, &mut pm, 1.2, true);
}

pub fn run() {
    if let Some(o) = std::env::var("POCHI_ALERT").ok() { std::fs::create_dir_all(&o).unwrap(); alert_frames(&o); return; }
    if let Some(o) = std::env::var("POCHI_SETTINGS").ok() { std::fs::create_dir_all(&o).unwrap(); settings_preview(&o); return; }
    let out = std::env::args().nth(1).unwrap_or_else(|| "/tmp/pochi-prev".into());
    std::fs::create_dir_all(&out).unwrap();
    let scale = 2.0f32;
    let mut fonts = Fonts::load();
    let mut icons = Icons::new();
    let mut app = sample();
    app.d.settings.idle_after = 0;
    let mut ui = Ui::new(&app);
    ui.battery = Some((0.82, true));
    ui.set_art(&app);
    let mut pm = Pixmap::new((WIN_W * scale) as u32, (WIN_H * scale) as u32).unwrap();
    let mut frame = |ui: &mut Ui, app: &mut App, pm: &mut Pixmap, secs: f32, name: Option<&str>| {
        let steps = (secs * 60.0) as i32;
        let mut worst = 0f64;
        let mut total = 0f64;
        for _ in 0..steps.max(1) {
            ui.update(app, &mut fonts, 1.0 / 60.0);
            let t0 = std::time::Instant::now();
            let mut cv = Canvas { pm, s: scale, mask: None, alpha: 1.0, ox: 0.0, oy: 0.0, fonts: &mut fonts, icons: &mut icons };
            ui.draw(app, &mut cv);
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            worst = worst.max(dt); total += dt;
        }
        if let Some(n) = name { pm.save_png(format!("{}/{}.png", out, n)).unwrap(); println!("{:<14} avg {:.2} ms  worst {:.2} ms", n, total / steps.max(1) as f64, worst); }
    };
    frame(&mut ui, &mut app, &mut pm, 1.0, Some("pill"));
    ui.expand();
    ui.page = "water";
    frame(&mut ui, &mut app, &mut pm, 0.08, Some("expand-mid"));
    frame(&mut ui, &mut app, &mut pm, 1.4, Some("water"));
    let _ = ui.click(&mut app, (WIN_W / 2.0 + 150.0, 100.0));
    frame(&mut ui, &mut app, &mut pm, 1.4, Some("water-chart"));
    ui.water_edit = true;
    frame(&mut ui, &mut app, &mut pm, 0.6, Some("water-edit"));
    ui.water_edit = false;
    for p in ["todo", "pomodoro", "calendar", "media"] {
        ui.peek(&app, p, true, false);
        frame(&mut ui, &mut app, &mut pm, 1.2, Some(p));
    }
    ui.peek(&app, "todo", true, false);
    ui.todo_form.open = true; ui.todo_form.text = "Call mom".into(); ui.focus = Some(FieldId::Todo);
    frame(&mut ui, &mut app, &mut pm, 0.5, Some("todo-form"));
    ui.todo_form.open = false; ui.focus = None;
    ui.peek(&app, "calendar", true, false);
    ui.event_form.open = true; ui.event_form.text = "Dentist".into(); ui.event_form.day = 1; ui.event_form.time = Some((14, 30));
    frame(&mut ui, &mut app, &mut pm, 0.5, Some("event-form"));
    ui.event_form.open = false;
    // page switch mid-transition
    ui.peek(&app, "water", true, false);
    frame(&mut ui, &mut app, &mut pm, 0.5, None);
    let _ = ui.click(&mut app, (WIN_W / 2.0 + 280.0 - 23.0, 91.0));
    frame(&mut ui, &mut app, &mut pm, 0.12, Some("switch-mid"));
    frame(&mut ui, &mut app, &mut pm, 0.6, None);
    ui.collapse(true);
    app.media.playing = false;
    frame(&mut ui, &mut app, &mut pm, 1.2, Some("collapsed"));
    app.d.settings.idle_after = 1;
    frame(&mut ui, &mut app, &mut pm, 1.3, None);
    frame(&mut ui, &mut app, &mut pm, 0.9, Some("cameo-open"));
    frame(&mut ui, &mut app, &mut pm, 1.0, Some("cameo-wave"));
    // benchmark: full expand + 4 page switches
    app.d.settings.idle_after = 0;
    frame(&mut ui, &mut app, &mut pm, 6.0, None);
    ui.expand();
    frame(&mut ui, &mut app, &mut pm, 1.0, Some("bench-expand"));
    if std::env::var("POCHI_HOLD").is_ok() { println!("holding"); std::thread::sleep(std::time::Duration::from_secs(8)); }
}
