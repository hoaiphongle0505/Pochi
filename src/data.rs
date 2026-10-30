//! Persistent data. The JSON layout matches the Electron version (pochi-data.json) so existing
//! users keep their history, to-dos, events and settings.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn t() -> bool { true }

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Pages {
    pub home: bool,
    pub notifications: bool,
    pub media: bool,
    pub calendar: bool,
    pub water: bool,
    pub todo: bool,
    pub pomodoro: bool,
}
impl Default for Pages {
    fn default() -> Self { Pages { home: true, notifications: true, media: true, calendar: true, water: true, todo: true, pomodoro: true } }
}
impl Pages {
    pub fn get(&self, id: &str) -> bool {
        match id { "home" => self.home, "notifications" => self.notifications, "media" => self.media, "calendar" => self.calendar, "water" => self.water, "todo" => self.todo, "pomodoro" => self.pomodoro, _ => false }
    }
    pub fn set(&mut self, id: &str, v: bool) {
        match id { "home" => self.home = v, "notifications" => self.notifications = v, "media" => self.media = v, "calendar" => self.calendar = v, "water" => self.water = v, "todo" => self.todo = v, "pomodoro" => self.pomodoro = v, _ => {} }
        if !(self.home || self.notifications || self.media || self.calendar || self.water || self.todo || self.pomodoro) { self.set(id, true); }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    #[serde(default = "t")] pub start_with_windows: bool,
    pub size: serde_json::Value, // "auto" | 100 | 115 | 130 | 150
    pub minimal: bool,
    pub capsule_color: String,
    pub opacity: u8,
    pub reduced_motion: bool,
    #[serde(default = "t")] pub mascot: bool,
    pub idle_after: u32,
    #[serde(default = "t")] pub sounds: bool,
    #[serde(default = "t")] pub hide_fullscreen: bool,
    pub auto_hide: bool,
    pub hidden: bool,
    pub display: String,
    #[serde(default = "t")] pub hover_expand: bool,
    pub collapse_delay: u32,
    pub clock24: bool,
    #[serde(default = "t")] pub show_battery: bool,
    #[serde(default = "t")] pub show_date: bool,
    #[serde(default = "t")] pub peek: bool,
    pub peek_seconds: u32,
    #[serde(default = "t")] pub sys_notif: bool,
    pub event_lead: u32,
    pub page: serde_json::Value,
    pub pages: Pages,
    #[serde(default = "t")] pub media_peek: bool,
    #[serde(default = "t")] pub media_pill: bool,
    #[serde(default = "t")] pub lyrics: bool,
    pub water_view: String,
}
impl Default for Settings {
    fn default() -> Self {
        Settings {
            start_with_windows: true, size: serde_json::json!("auto"), minimal: false, capsule_color: "midnight".into(), opacity: 100,
            reduced_motion: false, mascot: true, idle_after: 60,
            sounds: true, hide_fullscreen: true, auto_hide: false, hidden: false, display: "main".into(),
            hover_expand: true, collapse_delay: 400, clock24: false, show_battery: true, show_date: true,
            peek: true, peek_seconds: 6, sys_notif: true, event_lead: 10, page: serde_json::json!("home"),
            pages: Pages::default(), media_peek: true, media_pill: true, lyrics: true, water_view: "gauge".into(),
        }
    }
}
impl Settings {
    pub fn size_factor(&self) -> f32 {
        match &self.size { serde_json::Value::Number(n) => (n.as_f64().unwrap_or(100.0) / 100.0) as f32, _ => 0.86 }
    }
    pub fn page_id(&self) -> String {
        match &self.page { serde_json::Value::String(s) => s.clone(), _ => "home".into() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Sip { pub t: i64, pub ml: i32 }

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Water {
    #[serde(default = "t")] pub enabled: bool,
    pub goal: i32,
    pub bottle: i32,
    pub cup: i32, // 0 = auto
    pub interval: i32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub log: BTreeMap<String, Vec<Sip>>,
}
impl Default for Water {
    fn default() -> Self { Water { enabled: true, goal: 1500, bottle: 350, cup: 0, interval: 60, start_hour: 8, end_hour: 22, log: BTreeMap::new() } }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Todo {
    pub id: String,
    pub text: String,
    pub done: bool,
    pub remind_at: Option<String>,
    pub daily: bool,
    pub notified: bool,
    pub created_at: i64,
    pub done_at: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Pomodoro {
    pub focus: i32,
    pub short: i32,
    pub long: i32,
    pub rounds: i32,
    pub auto_start: bool,
    pub stats: BTreeMap<String, i32>,
}
impl Default for Pomodoro {
    fn default() -> Self { Pomodoro { focus: 25, short: 5, long: 15, rounds: 4, auto_start: false, stats: BTreeMap::new() } }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub date: String,
    pub time: String,
    pub notified: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Data {
    pub settings: Settings,
    pub water: Water,
    pub todos: Vec<Todo>,
    pub todo_day: String,
    pub pomodoro: Pomodoro,
    pub events: Vec<Event>,
    pub v2: bool,
    pub v3: bool,
}

pub fn data_dir() -> std::path::PathBuf {
    let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir());
    base.join("Pochi")
}

pub fn data_file() -> std::path::PathBuf { data_dir().join("pochi-data.json") }

impl Data {
    pub fn load() -> Data {
        let mut d: Data = std::fs::read_to_string(data_file()).ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .or_else(|| {
                // migrate from the very first version (Notchy)
                let base = std::env::var_os("APPDATA").map(std::path::PathBuf::from)?;
                let s = std::fs::read_to_string(base.join("Notchy").join("notchy-data.json")).ok()?;
                serde_json::from_str(&s).ok()
            })
            .unwrap_or_default();
        if !d.v2 { d.water.cup = 0; d.v2 = true; }
        d.v3 = true;
        d
    }

    pub fn save(&self) {
        let _ = std::fs::create_dir_all(data_dir());
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = data_file().with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() { let _ = std::fs::rename(&tmp, data_file()); }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_settings_json_uses_defaults_for_new_options_and_pages() {
        let settings: Settings = serde_json::from_str(r#"{"page":"water","pages":{"water":true}}"#).unwrap();

        assert_eq!(settings.page_id(), "water");
        assert_eq!(settings.capsule_color, "midnight");
        assert_eq!(settings.opacity, 100);
        assert!(!settings.reduced_motion);
        assert!(settings.pages.get("home"));
        assert!(settings.pages.get("notifications"));
    }

    #[test]
    fn invalid_saved_page_falls_back_to_home() {
        let settings: Settings = serde_json::from_str(r#"{"page":null}"#).unwrap();

        assert_eq!(settings.page_id(), "home");
    }

    #[test]
    fn disabling_every_page_keeps_the_selected_page_enabled() {
        let mut pages = Pages::default();
        for id in ["home", "notifications", "media", "calendar", "water", "todo", "pomodoro"] {
            pages.set(id, false);
        }

        assert!(["home", "notifications", "media", "calendar", "water", "todo", "pomodoro"].iter().any(|id| pages.get(id)));
    }
}
