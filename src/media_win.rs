//! Windows media session (WinRT, event driven), album art via WIC, lyrics via WinHTTP.
//! Runs on its own MTA thread and posts a window message when something changes.

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows::core::*;
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::*;
use windows::Storage::Streams::{DataReader, IInputStream};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub const WM_MEDIA: u32 = 0x8000 + 2;

#[derive(Clone, Default)]
pub struct Shared {
    pub media: crate::app::Media,
    pub lyrics: crate::app::Lyrics,
    pub track_changed: bool,
}

pub enum Msg { Refresh, Cmd(String), Lyrics(bool) }

pub struct MediaHandle { pub tx: Sender<Msg>, pub shared: Arc<Mutex<Shared>> }

pub fn start(hwnd: HWND, lyrics: bool) -> MediaHandle {
    let (tx, rx) = channel::<Msg>();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let h = hwnd.0 as isize;
    let sh = shared.clone();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        unsafe { let _ = CoInitializeEx(None, COINIT_MULTITHREADED); }
        let _ = run(HWND(h as *mut _), rx, tx2, sh, lyrics);
    });
    MediaHandle { tx, shared }
}

fn filetime_to_unix_ms(t: i64) -> i64 { (t - 116_444_736_000_000_000) / 10_000 }

fn run(hwnd: HWND, rx: Receiver<Msg>, tx: Sender<Msg>, shared: Arc<Mutex<Shared>>, mut lyrics_on: bool) -> Result<()> {
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;
    let txc = tx.clone();
    mgr.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| { let _ = txc.send(Msg::Refresh); Ok(()) }))?;
    let txs = tx.clone();
    mgr.SessionsChanged(&TypedEventHandler::new(move |_, _| { let _ = txs.send(Msg::Refresh); Ok(()) }))?;

    let mut hooked: Option<(GlobalSystemMediaTransportControlsSession, i64, i64, i64)> = None;
    let mut last_key = String::new();
    let mut last_sent = (false, String::new(), false, 0i64, 0i64);
    let mut lyric_key = String::new();
    let _ = tx.send(Msg::Refresh);

    loop {
        let msg = rx.recv_timeout(Duration::from_secs(5));
        let session = mgr.GetCurrentSession().ok();
        match msg {
            Ok(Msg::Cmd(c)) => {
                if let Some(s) = &session {
                    let r = match c.split_whitespace().next().unwrap_or("") {
                        "toggle" => s.TryTogglePlayPauseAsync().and_then(|o| o.get()),
                        "next" => s.TrySkipNextAsync().and_then(|o| o.get()),
                        "prev" => s.TrySkipPreviousAsync().and_then(|o| o.get()),
                        "seek" => {
                            let ms: i64 = c.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                            s.TryChangePlaybackPositionAsync(ms * 10_000).and_then(|o| o.get())
                        }
                        _ => Ok(false),
                    };
                    let _ = r;
                    std::thread::sleep(Duration::from_millis(120));
                }
            }
            Ok(Msg::Lyrics(on)) => { lyrics_on = on; lyric_key.clear(); }
            Ok(Msg::Refresh) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }

        // (re)subscribe to the current session's change events
        let same = match (&hooked, &session) {
            (Some((a, ..)), Some(b)) => a == b,
            (None, None) => true,
            _ => false,
        };
        if !same {
            if let Some((old, t1, t2, t3)) = hooked.take() {
                let _ = old.RemoveMediaPropertiesChanged(t1);
                let _ = old.RemovePlaybackInfoChanged(t2);
                let _ = old.RemoveTimelinePropertiesChanged(t3);
            }
            if let Some(s) = &session {
                let (a, b, c) = (tx.clone(), tx.clone(), tx.clone());
                let t1 = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| { let _ = a.send(Msg::Refresh); Ok(()) })).unwrap_or_default();
                let t2 = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| { let _ = b.send(Msg::Refresh); Ok(()) })).unwrap_or_default();
                let t3 = s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| { let _ = c.send(Msg::Refresh); Ok(()) })).unwrap_or_default();
                hooked = Some((s.clone(), t1, t2, t3));
            }
        }

        let mut m = crate::app::Media::default();
        if let Some(s) = &session {
            if let Ok(p) = s.TryGetMediaPropertiesAsync().and_then(|o| o.get()) {
                m.title = p.Title().map(|v| v.to_string()).unwrap_or_default();
                m.artist = p.Artist().map(|v| v.to_string()).unwrap_or_default();
                m.app = s.SourceAppUserModelId().map(|v| v.to_string()).unwrap_or_default();
                m.ok = !(m.title.is_empty() && m.artist.is_empty());
                m.key = format!("{}|{}|{}", m.app, m.title, m.artist);
                if m.ok && m.key != last_key {
                    if let Ok(r) = p.Thumbnail() { m.art = read_thumbnail(&r).ok(); }
                }
            }
            if let Ok(pb) = s.GetPlaybackInfo() {
                m.playing = pb.PlaybackStatus().map(|v| v == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing).unwrap_or(false);
            }
            if let Ok(tl) = s.GetTimelineProperties() {
                m.pos = tl.Position().map(|v| v.Duration / 10_000).unwrap_or(0);
                m.end = tl.EndTime().map(|v| v.Duration / 10_000).unwrap_or(0);
                let upd = tl.LastUpdatedTime().map(|v| filetime_to_unix_ms(v.UniversalTime)).unwrap_or(0);
                let now = crate::time::now_ms();
                m.upd = if upd > 0 && upd <= now { upd } else { now };
            }
        }
        let track_changed = m.ok && m.key != last_key;
        {
            let mut g = shared.lock().unwrap();
            if m.ok && m.key == last_key { m.art = g.media.art.clone(); m.art_key = g.media.art_key.clone(); }
            else if m.art.is_some() { m.art_key = m.key.clone(); }
            let sig = (m.ok, m.key.clone(), m.playing, m.pos / 1000, m.end);
            if sig == last_sent && !track_changed { continue; }
            last_sent = sig;
            g.media = m.clone();
            g.track_changed |= track_changed;
        }
        if m.ok { last_key = m.key.clone(); } else { last_key.clear(); }
        unsafe { let _ = PostMessageW(Some(hwnd), WM_MEDIA, WPARAM(0), LPARAM(0)); }

        if lyrics_on && m.ok && m.key != lyric_key && !m.title.is_empty() {
            lyric_key = m.key.clone();
            let lines = fetch_lyrics(&m.title, &m.artist, m.end).unwrap_or_default();
            shared.lock().unwrap().lyrics = crate::app::Lyrics { key: m.key.clone(), lines };
            unsafe { let _ = PostMessageW(Some(hwnd), WM_MEDIA, WPARAM(0), LPARAM(0)); }
        }
    }
}

/// thumbnail → 128×128 premultiplied RGBA (centre-cropped)
fn read_thumbnail(r: &windows::Storage::Streams::IRandomAccessStreamReference) -> Result<(u32, Vec<u8>)> {
    let stream = r.OpenReadAsync()?.get()?;
    let size = stream.Size()? as u32;
    if size == 0 || size > 20_000_000 { return Err(Error::empty()); }
    let input: IInputStream = stream.cast()?;
    let reader = DataReader::CreateDataReader(&input)?;
    reader.LoadAsync(size)?.get()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes)?;
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let ws = factory.CreateStream()?;
        ws.InitializeFromMemory(&bytes)?;
        let dec = factory.CreateDecoderFromStream(&ws, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = dec.GetFrame(0)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h)?;
        let side = w.min(h).max(1);
        let clip = factory.CreateBitmapClipper()?;
        let rect = WICRect { X: ((w - side) / 2) as i32, Y: ((h - side) / 2) as i32, Width: side as i32, Height: side as i32 };
        clip.Initialize(&frame, &rect)?;
        let n = 128u32;
        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&clip, n, n, WICBitmapInterpolationModeFant)?;
        let conv = factory.CreateFormatConverter()?;
        conv.Initialize(&scaler, &GUID_WICPixelFormat32bppPRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let mut out = vec![0u8; (n * n * 4) as usize];
        conv.CopyPixels(std::ptr::null(), n * 4, &mut out)?;
        Ok((n, out))
    }
}

/* ------------------------------------------------------------ lyrics */

fn parse_lrc(text: &str) -> Vec<(i64, String)> {
    let mut out = vec![];
    for line in text.lines() {
        let mut rest = line;
        let mut stamps = vec![];
        while let Some(r) = rest.strip_prefix('[') {
            let Some(end) = r.find(']') else { break };
            let tag = &r[..end];
            if let Some((m, s)) = tag.split_once(':') {
                if let (Ok(m), Ok(s)) = (m.parse::<f64>(), s.parse::<f64>()) { stamps.push(((m * 60.0 + s) * 1000.0) as i64); }
            }
            rest = &r[end + 1..];
        }
        for t in stamps { out.push((t, rest.trim().to_string())); }
    }
    out.sort_by_key(|x| x.0);
    out
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect()
}

fn fetch_lyrics(title: &str, artist: &str, end: i64) -> Option<Vec<(i64, String)>> {
    let clean: String = {
        let lower = title.to_lowercase();
        let mut t = title.to_string();
        for (o, c) in [('(', ')'), ('[', ']')] {
            if let (Some(a), Some(b)) = (t.find(o), t.rfind(c)) {
                let inner = lower.get(a..=b).unwrap_or("");
                if ["official", "lyric", "video", "audio", "mv", "remaster", "live"].iter().any(|k| inner.contains(k)) && b > a {
                    t.replace_range(a..=b, "");
                }
            }
        }
        t.trim().to_string()
    };
    let mut path = format!("/api/get?track_name={}&artist_name={}", enc(&clean), enc(artist));
    if end > 0 { path += &format!("&duration={}", end / 1000); }
    let body = http_get("lrclib.net", &path)?;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
    let synced = v.get("syncedLyrics").and_then(|s| s.as_str()).map(parse_lrc).unwrap_or_default();
    if !synced.is_empty() { return Some(synced); }
    let body = http_get("lrclib.net", &format!("/api/search?q={}", enc(&format!("{} {}", artist, clean).trim().to_string())))?;
    let arr: serde_json::Value = serde_json::from_str(&body).ok()?;
    let first = arr.as_array()?.iter().find(|x| x.get("syncedLyrics").and_then(|s| s.as_str()).is_some())?;
    Some(parse_lrc(first.get("syncedLyrics")?.as_str()?))
}

fn http_get(host: &str, path: &str) -> Option<String> {
    use windows::Win32::Networking::WinHttp::*;
    unsafe {
        let ua = HSTRING::from("Pochi/3.0 (notch app)");
        let session = WinHttpOpen(&ua, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0);
        if session.is_null() { return None; }
        let _ = WinHttpSetTimeouts(session, 5000, 5000, 8000, 8000);
        let h = HSTRING::from(host);
        let conn = WinHttpConnect(session, &h, 443, 0);
        let mut result = None;
        if !conn.is_null() {
            let verb = HSTRING::from("GET");
            let p = HSTRING::from(path);
            let req = WinHttpOpenRequest(conn, &verb, &p, PCWSTR::null(), PCWSTR::null(), std::ptr::null(), WINHTTP_FLAG_SECURE);
            if !req.is_null() {
                if WinHttpSendRequest(req, None, None, 0, 0, 0).is_ok() && WinHttpReceiveResponse(req, std::ptr::null_mut()).is_ok() {
                    let mut buf = Vec::new();
                    loop {
                        let mut avail = 0u32;
                        if WinHttpQueryDataAvailable(req, &mut avail).is_err() || avail == 0 { break; }
                        let mut chunk = vec![0u8; avail as usize];
                        let mut read = 0u32;
                        if WinHttpReadData(req, chunk.as_mut_ptr() as *mut _, avail, &mut read).is_err() || read == 0 { break; }
                        buf.extend_from_slice(&chunk[..read as usize]);
                        if buf.len() > 2_000_000 { break; }
                    }
                    result = String::from_utf8(buf).ok();
                }
                let _ = WinHttpCloseHandle(req);
            }
            let _ = WinHttpCloseHandle(conn);
        }
        let _ = WinHttpCloseHandle(session);
        result
    }
}
