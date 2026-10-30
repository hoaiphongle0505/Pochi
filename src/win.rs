//! Win32 shell: a per-pixel-alpha layered window that is redrawn only when something changes
//! (vsync-paced while animating, asleep otherwise), tray menu, hotkey, power/full-screen events.

use std::ffi::c_void;

use tiny_skia::Pixmap;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;

use crate::app::*;
use crate::data::Data;
use crate::gfx::*;
use crate::media_win::{self, MediaHandle, Msg, WM_MEDIA};
use crate::time::now_ms;
use crate::ui::*;
use crate::settings::{SettingsUi, SOut, SW, SH, Monitor as SMon};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DwmExtendFrameIntoClientArea, DWMWINDOWATTRIBUTE};
use windows::Win32::UI::Controls::MARGINS;

const WM_TRAY: u32 = WM_APP + 1;
const WM_SHOW_ME: u32 = WM_APP + 3;
const HOTKEY_ID: i32 = 1;
const CLASS: PCWSTR = w!("PochiNotchWindow");
const SCLASS: PCWSTR = w!("PochiSettingsWindow");

struct SetWin { hwnd: HWND, ui: SettingsUi, pm: Option<Pixmap>, dib: Option<(HDC, HBITMAP, *mut u8, i32, i32)>, scale: f32, dirty: bool, tracking: bool }

struct Monitor { rect: RECT, device: String, primary: bool, dpi: u32 }

struct Win {
    hwnd: HWND,
    app: App,
    ui: Ui,
    fonts: Fonts,
    icons: Icons,
    pm: Option<Pixmap>,
    dib: Option<(HDC, HBITMAP, *mut u8, i32, i32)>,
    scale: f32,
    origin: (i32, i32),
    size: (i32, i32),
    dirty: bool,
    last_frame: std::time::Instant,
    next_tick: i64,
    save_at: i64,
    fs_hidden: bool,
    visible: bool,
    focus_mode: bool,
    tracking: bool,
    media: Option<MediaHandle>,
    in_menu: bool,
    tray_icon: HICON,
    last_topmost: i64,
    mon_rect: RECT,
    loc_hook: Option<HWINEVENTHOOK>,
    settings: Option<SetWin>,
}

static mut WIN: *mut Win = std::ptr::null_mut();

fn win() -> Option<&'static mut Win> { unsafe { WIN.as_mut() } }

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);

        // single instance: ask the running copy to show itself
        let _m = CreateMutexW(None, true, w!("Local\\PochiNotchSingleton"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(h) = FindWindowW(CLASS, PCWSTR::null()) { let _ = PostMessageW(Some(h), WM_SHOW_ME, WPARAM(0), LPARAM(0)); }
            return;
        }

        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let swc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(settings_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(hinst.into()), PCWSTR(1 as *const u16)).unwrap_or_default(),
            lpszClassName: SCLASS,
            ..Default::default()
        };
        RegisterClassExW(&swc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            CLASS, w!("Pochi"), WS_POPUP, 0, 0, 10, 10, None, None, Some(hinst.into()), None,
        ).expect("window");

        let data = Data::load();
        let app = App::new(data);
        let ui = Ui::new(&app);
        let tray_icon = LoadImageW(Some(hinst.into()), PCWSTR(1 as *const u16), IMAGE_ICON, GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0)).unwrap_or_default();
        let w = Box::new(Win {
            hwnd, app, ui, fonts: Fonts::load(), icons: Icons::new(), pm: None, dib: None, scale: 1.0,
            origin: (0, 0), size: (0, 0), dirty: true, last_frame: std::time::Instant::now(), next_tick: 0,
            save_at: 0, fs_hidden: false, visible: false, focus_mode: false, tracking: false, media: None,
            in_menu: false, tray_icon, last_topmost: 0, mon_rect: RECT::default(), loc_hook: None, settings: None,
        });
        WIN = Box::into_raw(w);
        let w = win().unwrap();
        w.reposition();
        w.read_battery();
        w.add_tray();
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'P' as u32);
        w.apply_autostart();
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(win_event), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
        w.hook_foreground();
        w.media = Some(media_win::start(hwnd, w.app.d.settings.lyrics));
        w.check_fullscreen();
        w.apply_visibility();

        // ---- message loop: sleep until input/deadline; vsync-paced only while animating ----
        let mut msg = MSG::default();
        let mut redraw_at: i64 = 0;
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT { w.app.d.save(); return; }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let w = win().unwrap();
            let now = now_ms();
            if now >= w.next_tick {
                let next = w.app.tick();
                w.next_tick = now + next.min(5000);
                w.process_effects();
            }
            if w.app.changed { w.app.changed = false; w.dirty = true; }
            if w.app.dirty_save {
                if w.save_at == 0 { w.save_at = now + 600; }
                if now >= w.save_at { w.app.d.save(); w.app.dirty_save = false; w.save_at = 0; }
            }
            let dt = w.last_frame.elapsed().as_secs_f32().min(0.05);
            w.last_frame = std::time::Instant::now();
            let (anim, wait) = w.ui.update(&mut w.app, &mut w.fonts, dt);
            if now_ms() >= redraw_at { w.dirty = true; }
            if anim || w.dirty {
                w.present();
                if w.dirty { if let Some(sw) = w.settings.as_mut() { sw.dirty = true; } }
                w.dirty = false;
            }
            let mut sanim = false;
            if let Some(sw) = w.settings.as_mut() {
                sanim = sw.ui.animating(dt, w.app.d.settings.reduced_motion);
                if sanim || sw.ui.focus.is_some() { sw.dirty = true; }
                if sw.dirty { let _ = InvalidateRect(Some(sw.hwnd), None, false); }
            }
            let anim = anim || sanim;
            if anim {
                // wait for the next display refresh (smooth, no busy loop)
                if DwmFlush().is_err() { std::thread::sleep(std::time::Duration::from_millis(8)); }
            } else {
                redraw_at = now_ms() + wait;
                let mut wait = wait.min(w.next_tick - now_ms()).max(1);
                if w.app.dirty_save { wait = wait.min(700); }
                if w.settings.as_ref().map(|s| s.ui.focus.is_some()).unwrap_or(false) { wait = wait.min(530); }
                MsgWaitForMultipleObjects(None, false, wait as u32, QS_ALLINPUT);
            }
        }
    }
}

impl Win {
    fn monitors() -> Vec<Monitor> {
        unsafe extern "system" fn cb(hm: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
            let v = &mut *(data.0 as *mut Vec<Monitor>);
            let mut mi = MONITORINFOEXW::default();
            mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(hm, &mut mi.monitorInfo).as_bool() {
                let (mut dx, mut dy) = (96u32, 96u32);
                let _ = GetDpiForMonitor(hm, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
                let dev = String::from_utf16_lossy(&mi.szDevice).trim_end_matches('\0').to_string();
                v.push(Monitor { rect: mi.monitorInfo.rcMonitor, device: dev, primary: mi.monitorInfo.dwFlags & 1 == 1, dpi: dx });
            }
            TRUE
        }
        let mut v: Vec<Monitor> = vec![];
        unsafe { let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize)); }
        v.sort_by_key(|m| (!m.primary, m.rect.left));
        v
    }

    fn target_monitor(&self) -> Option<Monitor> {
        let mut ms = Self::monitors();
        let want = &self.app.d.settings.display;
        let i = ms.iter().position(|m| &m.device == want).unwrap_or(0);
        if ms.is_empty() { None } else { Some(ms.swap_remove(i)) }
    }

    fn reposition(&mut self) {
        let Some(m) = self.target_monitor() else { return };
        self.scale = m.dpi as f32 / 96.0 * self.app.d.settings.size_factor();
        let w = (WIN_W * self.scale).ceil() as i32;
        let h = (WIN_H * self.scale).ceil() as i32;
        let mw = m.rect.right - m.rect.left;
        self.origin = (m.rect.left + (mw - w) / 2, m.rect.top);
        self.mon_rect = m.rect;
        if self.size != (w, h) {
            self.size = (w, h);
            self.pm = Pixmap::new(w as u32, h as u32);
            unsafe {
                if let Some((dc, bmp, ..)) = self.dib.take() { let _ = DeleteObject(bmp.into()); let _ = DeleteDC(dc); }
                let screen = GetDC(None);
                let dc = CreateCompatibleDC(Some(screen));
                ReleaseDC(None, screen);
                let bi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32,
                        biCompression: BI_RGB.0, ..Default::default()
                    },
                    ..Default::default()
                };
                let mut bits: *mut c_void = std::ptr::null_mut();
                if let Ok(bmp) = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                    SelectObject(dc, bmp.into());
                    self.dib = Some((dc, bmp, bits as *mut u8, w, h));
                }
            }
        }
        self.dirty = true;
    }

    fn present(&mut self) {
        if !self.visible { return; }
        let Some(pm) = self.pm.as_mut() else { return };
        let Some((dc, _, bits, w, h)) = self.dib else { return };
        {
            let mut cv = Canvas { pm, s: self.scale, mask: None, alpha: 1.0, ox: 0.0, oy: 0.0, fonts: &mut self.fonts, icons: &mut self.icons };
            self.ui.draw(&self.app, &mut cv);
        }
        let src = pm.data();
        unsafe {
            let dst = std::slice::from_raw_parts_mut(bits, (w * h * 4) as usize);
            // premultiplied RGBA → premultiplied BGRA
            for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                d[0] = s[2]; d[1] = s[1]; d[2] = s[0]; d[3] = s[3];
            }
            let pos = POINT { x: self.origin.0, y: self.origin.1 };
            let size = SIZE { cx: w, cy: h };
            let zero = POINT { x: 0, y: 0 };
            let opacity = self.app.d.settings.opacity.clamp(30, 100) as u16;
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: (opacity * 255 / 100) as u8, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(self.hwnd, None, Some(&pos), Some(&size), Some(dc), Some(&zero), COLORREF(0), Some(&blend), ULW_ALPHA);
            let now = now_ms();
            if self.ui.expanded && now - self.last_topmost > 5000 {
                self.last_topmost = now;
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    fn apply_visibility(&mut self) {
        let show = !self.fs_hidden && !self.app.d.settings.hidden;
        if show == self.visible { return; }
        self.visible = show;
        unsafe {
            if show {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                self.dirty = true;
            } else {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
        }
    }

    fn check_fullscreen(&mut self) {
        let mut hide = false;
        if self.app.d.settings.hide_fullscreen {
            unsafe {
                let fg = GetForegroundWindow();
                if !fg.is_invalid() && fg != self.hwnd {
                    let mut cls = [0u16; 64];
                    let n = GetClassNameW(fg, &mut cls) as usize;
                    let cls = String::from_utf16_lossy(&cls[..n]);
                    if !["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd", "PochiNotchWindow"].contains(&cls.as_str()) {
                        let mut r = RECT::default();
                        if GetWindowRect(fg, &mut r).is_ok() {
                            let mr = self.mon_rect;
                            hide = mr.right > mr.left && r.left <= mr.left && r.top <= mr.top && r.right >= mr.right && r.bottom >= mr.bottom;
                        }
                    }
                }
            }
        }
        if hide != self.fs_hidden { self.fs_hidden = hide; self.apply_visibility(); }
    }

    /// listen for move/resize only on the foreground window's thread (no global event spam)
    fn hook_foreground(&mut self) {
        unsafe {
            if let Some(h) = self.loc_hook.take() { let _ = UnhookWinEvent(h); }
            let fg = GetForegroundWindow();
            if fg.is_invalid() || fg == self.hwnd { return; }
            let mut pid = 0u32;
            let tid = GetWindowThreadProcessId(fg, Some(&mut pid));
            if tid == 0 { return; }
            let h = SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(win_event), pid, tid, WINEVENT_OUTOFCONTEXT);
            if !h.is_invalid() { self.loc_hook = Some(h); }
        }
    }

    fn read_battery(&mut self) {
        let mut s = SYSTEM_POWER_STATUS::default();
        unsafe { if GetSystemPowerStatus(&mut s).is_err() { return; } }
        // desktops report "no system battery" (flag 128) → show nothing
        self.ui.battery = if s.BatteryFlag & 128 != 0 || s.BatteryLifePercent > 100 { None }
            else { Some((s.BatteryLifePercent as f32 / 100.0, s.ACLineStatus == 1)) };
        self.dirty = true;
    }

    fn apply_autostart(&self) {
        unsafe {
            let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            if self.app.d.settings.start_with_windows {
                let mut buf = [0u16; 1024];
                let n = windows::Win32::System::LibraryLoader::GetModuleFileNameW(None, &mut buf) as usize;
                let path = format!("\"{}\"\0", String::from_utf16_lossy(&buf[..n]));
                let wide: Vec<u16> = path.encode_utf16().collect();
                let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key, w!("Pochi"), REG_SZ.0, Some(wide.as_ptr() as *const c_void), (wide.len() * 2) as u32);
            } else {
                let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, key, w!("Pochi"));
            }
        }
    }

    fn set_focus_mode(&mut self, on: bool) {
        if on == self.focus_mode { return; }
        self.focus_mode = on;
        unsafe {
            let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE);
            let ex = if on { ex & !(WS_EX_NOACTIVATE.0 as i32) } else { ex | WS_EX_NOACTIVATE.0 as i32 };
            SetWindowLongW(self.hwnd, GWL_EXSTYLE, ex);
            if on { let _ = SetForegroundWindow(self.hwnd); let _ = SetFocus(Some(self.hwnd)); }
        }
    }

    fn handle_outs(&mut self, outs: Vec<Out>) {
        for o in outs {
            match o {
                Out::App(a) => { self.app.action(&a); if let Action::SetBool("lyrics", v) = a { self.media_send(Msg::Lyrics(v)); } }
                Out::Media(c) => match c.as_str() {
                    "mute" => key_press(VK_VOLUME_MUTE),
                    "volup" => key_press(VK_VOLUME_UP),
                    "voldown" => key_press(VK_VOLUME_DOWN),
                    _ => self.media_send(Msg::Cmd(c)),
                },
                Out::WantFocus(f) => self.set_focus_mode(f),
                Out::Changed => {}
            }
        }
        self.process_effects();
        self.dirty = true;
    }

    fn media_send(&self, m: Msg) { if let Some(h) = &self.media { let _ = h.tx.send(m); } }

    fn process_effects(&mut self) {
        let effects: Vec<Effect> = self.app.effects.drain(..).collect();
        for e in effects {
            match e {
                Effect::Notify { title, body } => self.balloon(&title, &body),
                Effect::Peek { page, force } => { if self.visible { self.ui.peek(&self.app, page, force, false); } }
                Effect::Alert { kind, page, title, body } => { if self.visible { self.ui.alert(&self.app, kind, page, title, body); } }
                Effect::Chime(kind) => chime(kind),
                Effect::Mood(m) => { if m == "happy" { self.ui.mood_happy(); } }
                Effect::Reposition => self.reposition(),
                Effect::Autostart => self.apply_autostart(),
                Effect::Visibility => { self.check_fullscreen(); self.apply_visibility(); }
            }
            self.dirty = true;
        }
    }

    /* ---------------- settings window ---------------- */

    fn smonitors(&self) -> Vec<SMon> {
        let target = self.target_monitor().map(|m| m.device).unwrap_or_default();
        Self::monitors().iter().enumerate().map(|(i, m)| {
            let r = m.rect;
            SMon { id: if m.primary { "main".into() } else { m.device.clone() }, label: format!("Display {} ({}×{}){}", i + 1, r.right - r.left, r.bottom - r.top, if m.primary { " · main" } else { "" }), selected: m.device == target }
        }).collect()
    }

    fn open_settings(&mut self, section: &str) {
        unsafe {
            if let Some(sw) = self.settings.as_mut() {
                sw.ui.goto(section);
                sw.dirty = true;
                let _ = ShowWindow(sw.hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(sw.hwnd);
                return;
            }
            let Some(m) = self.target_monitor() else { return };
            let scale = m.dpi as f32 / 96.0;
            let (w, h) = ((SW * scale) as i32, (SH * scale) as i32);
            let x = m.rect.left + ((m.rect.right - m.rect.left) - w) / 2;
            let y = m.rect.top + ((m.rect.bottom - m.rect.top) - h) / 2;
            let hinst = GetModuleHandleW(None).unwrap_or_default();
            let Ok(hwnd) = CreateWindowExW(WS_EX_APPWINDOW, SCLASS, w!("Pochi Settings"), WS_POPUP | WS_MINIMIZEBOX | WS_SYSMENU | WS_CLIPCHILDREN,
                x, y, w, h, None, None, Some(hinst.into()), None) else { return };
            let dark: i32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &dark as *const _ as *const c_void, 4);
            let round: i32 = 2;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(33), &round as *const _ as *const c_void, 4);
            let margins = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 1, cyBottomHeight: 0 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let mut ui = SettingsUi::new("general");
            ui.goto(section);
            ui.monitors = self.smonitors();
            ui.data_path = crate::data::data_file().to_string_lossy().to_string();
            let mut sw = SetWin { hwnd, ui, pm: Pixmap::new(w as u32, h as u32), dib: None, scale, dirty: true, tracking: false };
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
            let mut bits: *mut c_void = std::ptr::null_mut();
            if let Ok(bmp) = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
                SelectObject(dc, bmp.into());
                sw.dib = Some((dc, bmp, bits as *mut u8, w, h));
            }
            self.settings = Some(sw);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    fn paint_settings(&mut self, hdc: HDC) {
        let Some(sw) = self.settings.as_mut() else { return };
        let Some(pm) = sw.pm.as_mut() else { return };
        let Some((dc, _, bits, w, h)) = sw.dib else { return };
        if sw.dirty {
            sw.dirty = false;
            let mut cv = Canvas { pm, s: sw.scale, mask: None, alpha: 1.0, ox: 0.0, oy: 0.0, fonts: &mut self.fonts, icons: &mut self.icons };
            sw.ui.draw(&self.app, &mut cv);
            let src = pm.data();
            unsafe {
                let dst = std::slice::from_raw_parts_mut(bits, (w * h * 4) as usize);
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) { d[0] = s[2]; d[1] = s[1]; d[2] = s[0]; d[3] = 255; }
            }
        }
        unsafe { let _ = BitBlt(hdc, 0, 0, w, h, Some(dc), 0, 0, SRCCOPY); }
    }

    fn settings_outs(&mut self, outs: Vec<SOut>) {
        for o in outs {
            match o {
                SOut::App(a) => self.handle_outs(vec![Out::App(a)]),
                SOut::Display(id) => { self.handle_outs(vec![Out::App(Action::SetDisplay(id))]); if let Some(sw) = self.settings.as_mut() { let _ = sw; } let m = self.smonitors(); if let Some(sw) = self.settings.as_mut() { sw.ui.monitors = m; } }
                SOut::Platform("folder") => unsafe {
                    let p: Vec<u16> = crate::data::data_dir().to_string_lossy().encode_utf16().chain(Some(0)).collect();
                    let _ = ShellExecuteW(None, w!("open"), PCWSTR(p.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
                },
                SOut::Platform("updates") => unsafe {
                    let result = ShellExecuteW(None, w!("open"), w!("https://github.com/hoaiphongle0505/Pochi/releases/latest"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
                    if result.0 as usize <= 32 {
                        let _ = MessageBoxW(Some(self.hwnd), w!("Could not open GitHub releases. Visit github.com/hoaiphongle0505/Pochi/releases/latest in your browser."), w!("Pochi"), MB_OK | MB_ICONERROR);
                    }
                },
                SOut::Platform(_) => unsafe { self.app.d.save(); let nid = self.nid(); let _ = Shell_NotifyIconW(NIM_DELETE, &nid); PostQuitMessage(0); },
                SOut::Close => unsafe { if let Some(sw) = &self.settings { let _ = DestroyWindow(sw.hwnd); } },
                SOut::Focus(_) | SOut::Redraw => {}
            }
        }
        if let Some(sw) = self.settings.as_mut() { sw.dirty = true; let _ = unsafe { InvalidateRect(Some(sw.hwnd), None, false) }; }
    }

    /* ---------------- tray ---------------- */

    fn nid(&self) -> NOTIFYICONDATAW {
        let mut nid = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..Default::default() };
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = self.tray_icon;
        copy_w(&mut nid.szTip, "Pochi");
        nid
    }

    fn add_tray(&self) { unsafe { let _ = Shell_NotifyIconW(NIM_ADD, &self.nid()); } }

    fn balloon(&self, title: &str, body: &str) {
        let mut nid = self.nid();
        nid.uFlags = NIF_INFO;
        copy_w(&mut nid.szInfoTitle, title);
        copy_w(&mut nid.szInfo, body);
        nid.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
        unsafe { let _ = Shell_NotifyIconW(NIM_MODIFY, &nid); }
    }

    fn tray_menu(&mut self) {
        let s = self.app.d.settings.clone();
        let pc = self.app.d.pomodoro.clone();
        let mut acts: Vec<Option<Action>> = vec![None];
        let mut special: Vec<(u32, &'static str)> = vec![];
        unsafe {
            let menu = CreatePopupMenu().unwrap();
            let mut add = |m: HMENU, label: &str, checked: bool, act: Option<Action>, acts: &mut Vec<Option<Action>>| {
                let id = acts.len() as u32;
                acts.push(act);
                let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(m, MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED }, id as usize, PCWSTR(wide.as_ptr()));
                id
            };
            let sep = |m: HMENU| { let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null()); };
            let sub = |parent: HMENU, label: &str| -> HMENU {
                let m = CreatePopupMenu().unwrap();
                let wide: Vec<u16> = label.encode_utf16().chain(Some(0)).collect();
                let _ = AppendMenuW(parent, MF_POPUP, m.0 as usize, PCWSTR(wide.as_ptr()));
                m
            };
            let id_settings = add(menu, "Settings…", false, None, &mut acts);
            special.push((id_settings, "settings"));
            sep(menu);
            let id = add(menu, if s.hidden { "Show notch\tCtrl+Alt+P" } else { "Hide notch\tCtrl+Alt+P" }, false, Some(Action::SetBool("hidden", !s.hidden)), &mut acts);
            let _ = id;
            sep(menu);
            add(menu, &format!("Drink water (+{} ml)", self.app.water_sip()), false, Some(Action::Drink), &mut acts);
            add(menu, if self.app.pomo.running { "Pause Pomodoro" } else { "Start Pomodoro" }, false, Some(Action::PomoToggle), &mut acts);
            sep(menu);
            add(menu, "Start with Windows", s.start_with_windows, Some(Action::SetBool("startWithWindows", !s.start_with_windows)), &mut acts);
            add(menu, "Pochi mascot", s.mascot, Some(Action::SetBool("mascot", !s.mascot)), &mut acts);
            add(menu, "Reduce motion", s.reduced_motion, Some(Action::SetBool("reducedMotion", !s.reduced_motion)), &mut acts);
            let m = sub(menu, "Preset");
            for (id, label) in [("focus", "Focus"), ("work", "Work"), ("minimal", "Minimal")] {
                add(m, label, false, Some(Action::SetPreset(id)), &mut acts);
            }
            let m = sub(menu, "Pochi says hi");
            for (v, l) in [(0, "Off"), (60, "Every 1 min"), (300, "Every 5 min"), (900, "Every 15 min")] {
                add(m, l, s.idle_after == v, Some(Action::SetIdle(v)), &mut acts);
            }
            add(menu, "Minimal notch", s.minimal, Some(Action::SetBool("minimal", !s.minimal)), &mut acts);
            add(menu, "Auto-hide notch", s.auto_hide, Some(Action::SetBool("autoHide", !s.auto_hide)), &mut acts);
            add(menu, "Hide in full screen", s.hide_fullscreen, Some(Action::SetBool("hideFullscreen", !s.hide_fullscreen)), &mut acts);
            let m = sub(menu, "Notch size");
            let cur = s.size.as_u64().map(|v| v as u32);
            for (v, l) in [(None, "Compact (default)"), (Some(100), "Medium (100%)"), (Some(115), "Large (115%)"), (Some(130), "Extra large (130%)"), (Some(150), "Huge (150%)")] {
                add(m, l, cur == v, Some(Action::SetSize(v)), &mut acts);
            }
            let m = sub(menu, "Display");
            let mons = Self::monitors();
            let target = self.target_monitor().map(|m| m.device).unwrap_or_default();
            for (i, mo) in mons.iter().enumerate() {
                let r = mo.rect;
                let l = format!("Display {} ({}×{}){}", i + 1, r.right - r.left, r.bottom - r.top, if mo.primary { " · main" } else { "" });
                add(m, &l, mo.device == target, Some(Action::SetDisplay(if mo.primary { "main".into() } else { mo.device.clone() })), &mut acts);
            }
            sep(menu);
            let m = sub(menu, "Notch pages");
            for (id, l) in [("home", "Home & quick controls"), ("notifications", "Notifications"), ("media", "Music"), ("calendar", "Calendar"), ("water", "Water"), ("todo", "To-do"), ("pomodoro", "Pomodoro")] {
                add(m, l, s.pages.get(id), Some(Action::SetPage(id, !s.pages.get(id))), &mut acts);
            }
            let m = sub(menu, "Clock && header");
            add(m, "24-hour clock", s.clock24, Some(Action::SetBool("clock24", !s.clock24)), &mut acts);
            add(m, "Show date", s.show_date, Some(Action::SetBool("showDate", !s.show_date)), &mut acts);
            add(m, "Show battery", s.show_battery, Some(Action::SetBool("showBattery", !s.show_battery)), &mut acts);
            add(m, "Expand on hover", s.hover_expand, Some(Action::SetBool("hoverExpand", !s.hover_expand)), &mut acts);
            let m = sub(menu, "Reminders");
            add(m, "Sounds", s.sounds, Some(Action::SetBool("sounds", !s.sounds)), &mut acts);
            add(m, "Pop open the notch", s.peek, Some(Action::SetBool("peek", !s.peek)), &mut acts);
            add(m, "Windows notifications", s.sys_notif, Some(Action::SetBool("sysNotif", !s.sys_notif)), &mut acts);
            add(m, "Water reminders", self.app.d.water.enabled, Some(Action::WaterToggle), &mut acts);
            let ev = sub(m, "Remind before events");
            for (v, l) in [(0, "At start"), (5, "5 min"), (10, "10 min"), (15, "15 min"), (30, "30 min")] {
                add(ev, l, s.event_lead == v, Some(Action::SetEventLead(v)), &mut acts);
            }
            let m = sub(menu, "Music");
            add(m, "Show on collapsed notch", s.media_pill, Some(Action::SetBool("mediaPill", !s.media_pill)), &mut acts);
            add(m, "Pop open on new song", s.media_peek, Some(Action::SetBool("mediaPeek", !s.media_peek)), &mut acts);
            add(m, "Synced lyrics", s.lyrics, Some(Action::SetBool("lyrics", !s.lyrics)), &mut acts);
            let m = sub(menu, "Pomodoro");
            let f = sub(m, "Focus");
            for v in [15, 20, 25, 30, 45, 50, 60, 90] { add(f, &format!("{} min", v), pc.focus == v, Some(Action::PomoSet(0, v)), &mut acts); }
            let f = sub(m, "Short break");
            for v in [3, 5, 10, 15] { add(f, &format!("{} min", v), pc.short == v, Some(Action::PomoSet(1, v)), &mut acts); }
            let f = sub(m, "Long break");
            for v in [10, 15, 20, 30] { add(f, &format!("{} min", v), pc.long == v, Some(Action::PomoSet(2, v)), &mut acts); }
            let f = sub(m, "Sessions before long break");
            for v in [2, 3, 4, 5, 6] { add(f, &format!("{}", v), pc.rounds == v, Some(Action::PomoSet(3, v)), &mut acts); }
            add(m, "Auto-start next session", pc.auto_start, Some(Action::PomoSet(4, if pc.auto_start { 0 } else { 1 })), &mut acts);
            add(m, "Reset all rounds", false, Some(Action::PomoResetAll), &mut acts);
            let m = sub(menu, "Clean up");
            add(m, "Clear completed to-dos", false, Some(Action::TodoClearDone), &mut acts);
            add(m, "Remove past events", false, Some(Action::EventClearPast), &mut acts);
            add(m, "Undo last drink", false, Some(Action::WaterUndo), &mut acts);
            sep(menu);
            let id_folder = add(menu, "Open data folder", false, None, &mut acts);
            special.push((id_folder, "folder"));
            let id_about = add(menu, &format!("About Pochi {}", env!("CARGO_PKG_VERSION")), false, None, &mut acts);
            special.push((id_about, "about"));
            let id_exit = add(menu, "Exit", false, None, &mut acts);
            special.push((id_exit, "exit"));

            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(self.hwnd);
            self.in_menu = true;
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, Some(0), self.hwnd, None).0 as u32;
            self.in_menu = false;
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            if cmd == 0 { return; }
            if let Some((_, what)) = special.iter().find(|(i, _)| *i == cmd) {
                match *what {
                    "exit" => { self.app.d.save(); let nid = self.nid(); let _ = Shell_NotifyIconW(NIM_DELETE, &nid); PostQuitMessage(0); }
                    "folder" => {
                        let p: Vec<u16> = crate::data::data_dir().to_string_lossy().encode_utf16().chain(Some(0)).collect();
                        let _ = ShellExecuteW(None, w!("open"), PCWSTR(p.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
                    }
                    "settings" => self.open_settings("general"),
                    _ => self.open_settings("about"),
                }
                return;
            }
            if let Some(Some(a)) = acts.get(cmd as usize).cloned() {
                self.handle_outs(vec![Out::App(a)]);
            }
        }
    }
}

fn copy_w(dst: &mut [u16], s: &str) {
    for (i, c) in s.encode_utf16().take(dst.len() - 1).enumerate() { dst[i] = c; }
}

fn key_press(vk: VIRTUAL_KEY) {
    unsafe {
        let mut inputs = [INPUT::default(), INPUT::default()];
        inputs[0].r#type = INPUT_KEYBOARD;
        inputs[0].Anonymous.ki = KEYBDINPUT { wVk: vk, ..Default::default() };
        inputs[1].r#type = INPUT_KEYBOARD;
        inputs[1].Anonymous.ki = KEYBDINPUT { wVk: vk, dwFlags: KEYEVENTF_KEYUP, ..Default::default() };
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/* ---------------- sounds: tiny synthesized chimes ---------------- */

fn wav(notes: &[f32]) -> Vec<u8> {
    let rate = 22050u32;
    let step = 0.15f32;
    let len = ((notes.len() as f32 * step + 0.9) * rate as f32) as usize;
    let mut s = vec![0f32; len];
    for (i, f) in notes.iter().enumerate() {
        let start = (i as f32 * step * rate as f32) as usize;
        for j in 0..(0.9 * rate as f32) as usize {
            let t = j as f32 / rate as f32;
            let env = if t < 0.02 { t / 0.02 } else { (-(t - 0.02) * 5.5).exp() };
            if start + j < len { s[start + j] += (t * f * std::f32::consts::TAU).sin() * env * 0.22; }
        }
    }
    let data: Vec<u8> = s.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32000.0) as i16).to_le_bytes()).collect();
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend(b"RIFF"); out.extend((36 + data.len() as u32).to_le_bytes()); out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes()); out.extend(1u16.to_le_bytes()); out.extend(1u16.to_le_bytes());
    out.extend(rate.to_le_bytes()); out.extend((rate * 2).to_le_bytes()); out.extend(2u16.to_le_bytes()); out.extend(16u16.to_le_bytes());
    out.extend(b"data"); out.extend((data.len() as u32).to_le_bytes()); out.extend(data);
    out
}

fn chime(kind: &str) {
    use std::sync::OnceLock;
    static POMO: OnceLock<Vec<u8>> = OnceLock::new();
    static TODO: OnceLock<Vec<u8>> = OnceLock::new();
    static EVENT: OnceLock<Vec<u8>> = OnceLock::new();
    static WATER: OnceLock<Vec<u8>> = OnceLock::new();
    let buf = match kind {
        "pomo" => POMO.get_or_init(|| wav(&[784.0, 988.0, 1175.0, 1568.0])),
        "todo" => TODO.get_or_init(|| wav(&[660.0, 880.0])),
        "event" => EVENT.get_or_init(|| wav(&[523.0, 784.0, 1047.0])),
        _ => WATER.get_or_init(|| wav(&[880.0, 1320.0])),
    };
    unsafe { let _ = PlaySoundW(PCWSTR(buf.as_ptr() as *const u16), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT); }
}

/* ---------------- callbacks ---------------- */

unsafe extern "system" fn win_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, id_object: i32, _: i32, _: u32, _: u32) {
    let Some(w) = win() else { return };
    if w.in_menu { return; }
    if event == EVENT_OBJECT_LOCATIONCHANGE && (id_object != OBJID_WINDOW.0 || hwnd != GetForegroundWindow()) { return; }
    if event == EVENT_SYSTEM_FOREGROUND { w.hook_foreground(); }
    w.check_fullscreen();
}

fn lp_point(l: LPARAM, scale: f32) -> (f32, f32) {
    let x = (l.0 & 0xffff) as i16 as f32;
    let y = ((l.0 >> 16) & 0xffff) as i16 as f32;
    (x / scale, y / scale)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(w) = win() else { return DefWindowProcW(hwnd, msg, wp, lp) };
    if w.in_menu && msg != WM_TRAY { return DefWindowProcW(hwnd, msg, wp, lp); }
    match msg {
        WM_MOUSEMOVE => {
            if !w.tracking {
                let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut t);
                w.tracking = true;
            }
            if w.ui.mouse_move(lp_point(lp, w.scale), &w.app) { w.dirty = true; }
            LRESULT(0)
        }
        WM_MOUSELEAVE => { w.tracking = false; w.ui.mouse_leave(); w.dirty = true; LRESULT(0) }
        WM_LBUTTONDOWN => {
            let p = lp_point(lp, w.scale);
            let outs = w.ui.click(&mut w.app, p);
            w.handle_outs(outs);
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            if !w.ui.dblclick(&w.app) {
                let p = lp_point(lp, w.scale);
                let outs = w.ui.click(&mut w.app, p);
                w.handle_outs(outs);
            }
            w.dirty = true;
            LRESULT(0)
        }
        WM_RBUTTONUP => { w.tray_menu(); LRESULT(0) }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
            let outs = w.ui.wheel(&mut w.app, delta);
            w.handle_outs(outs);
            LRESULT(0)
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wp.0 as u32) { let outs = w.ui.char_input(c); w.handle_outs(outs); }
            LRESULT(0)
        }
        WM_ACTIVATE => {
            if (wp.0 & 0xffff) as u32 == WA_INACTIVE && w.focus_mode {
                // clicked elsewhere while typing: keep the draft, release focus
                w.ui.focus = None;
                w.set_focus_mode(false);
                w.dirty = true;
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => {
            if w.focus_mode { LRESULT(MA_ACTIVATE as isize) } else { LRESULT(MA_NOACTIVATE as isize) }
        }
        WM_HOTKEY => {
            if wp.0 as i32 == HOTKEY_ID {
                let hidden = !w.app.d.settings.hidden;
                w.handle_outs(vec![Out::App(Action::SetBool("hidden", hidden))]);
            }
            LRESULT(0)
        }
        WM_TRAY => {
            match (lp.0 & 0xffff) as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => w.tray_menu(),
                WM_LBUTTONUP => {
                    if w.app.d.settings.hidden { w.handle_outs(vec![Out::App(Action::SetBool("hidden", false))]); }
                    let page = w.ui.page;
                    w.ui.peek(&w.app, page, true, false);
                    w.dirty = true;
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_SHOW_ME => {
            if w.app.d.settings.hidden { w.handle_outs(vec![Out::App(Action::SetBool("hidden", false))]); }
            w.open_settings("general");
            LRESULT(0)
        }
        WM_MEDIA => {
            if let Some(h) = &w.media {
                let (m, l, changed) = {
                    let mut g = h.shared.lock().unwrap();
                    let c = g.track_changed;
                    g.track_changed = false;
                    (g.media.clone(), g.lyrics.clone(), c)
                };
                w.app.media = m;
                w.app.lyrics = l;
                w.ui.set_art(&w.app);
                let s = &w.app.d.settings;
                if changed && s.media_peek && s.pages.media && w.app.media.playing && w.visible && w.ui.focus.is_none() {
                    w.ui.peek(&w.app, "media", false, true);
                }
                w.dirty = true;
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => { w.read_battery(); LRESULT(1) }
        WM_DISPLAYCHANGE | WM_DPICHANGED | WM_SETTINGCHANGE => { w.reposition(); w.check_fullscreen(); LRESULT(0) }
        WM_ENDSESSION | WM_QUERYENDSESSION => { w.app.d.save(); LRESULT(1) }
        WM_DESTROY => { PostQuitMessage(0); LRESULT(0) }
        _ => {
            // Explorer restarted: re-add the tray icon
            static mut TASKBAR_CREATED: u32 = 0;
            if TASKBAR_CREATED == 0 { TASKBAR_CREATED = RegisterWindowMessageW(w!("TaskbarCreated")); }
            if msg == TASKBAR_CREATED { w.add_tray(); }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

unsafe extern "system" fn settings_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let Some(w) = win() else { return DefWindowProcW(hwnd, msg, wp, lp) };
    let scale = w.settings.as_ref().map(|s| s.scale).unwrap_or(1.0);
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            w.paint_settings(hdc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_NCHITTEST => {
            let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
            let _ = ScreenToClient(hwnd, &mut pt);
            let p = (pt.x as f32 / scale, pt.y as f32 / scale);
            if let Some(sw) = &w.settings { if sw.ui.is_drag(p) { return LRESULT(HTCAPTION as isize); } }
            LRESULT(HTCLIENT as isize)
        }
        WM_MOUSEMOVE => {
            if let Some(sw) = w.settings.as_mut() {
                if !sw.tracking {
                    let mut t = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut t);
                    sw.tracking = true;
                }
                sw.ui.mouse = lp_point(lp, scale);
                sw.dirty = true;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => { if let Some(sw) = w.settings.as_mut() { sw.tracking = false; sw.ui.mouse = (-1.0, -1.0); sw.dirty = true; let _ = InvalidateRect(Some(hwnd), None, false); } LRESULT(0) }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let outs = match w.settings.as_mut() { Some(sw) => sw.ui.click(lp_point(lp, scale)), None => vec![] };
            w.settings_outs(outs);
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let d = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
            if let Some(sw) = w.settings.as_mut() { if sw.ui.wheel(d) { sw.dirty = true; let _ = InvalidateRect(Some(hwnd), None, false); } }
            LRESULT(0)
        }
        WM_CHAR => {
            let outs = match (w.settings.as_mut(), char::from_u32(wp.0 as u32)) { (Some(sw), Some(c)) => sw.ui.char_input(c), _ => vec![] };
            w.settings_outs(outs);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wp.0 as u16 == VK_ESCAPE.0 && w.settings.as_ref().map(|s| s.ui.focus.is_none()).unwrap_or(false) { let _ = DestroyWindow(hwnd); }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(sw) = w.settings.take() {
                if let Some((dc, bmp, ..)) = sw.dib { let _ = DeleteObject(bmp.into()); let _ = DeleteDC(dc); }
            }
            w.icons = Icons::new(); // drop large cached icons
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
