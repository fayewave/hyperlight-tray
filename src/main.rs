//! Hyperlight Tray: shows a Hitscan Hyperlight mouse's battery level in the notification
//! area and raises a Windows notification when it runs low.
#![windows_subsystem = "windows"]

mod autostart;
mod config;
mod icon;
mod menu;
mod saved;
mod text;
mod web;

use std::cell::RefCell;
use std::mem;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use hyperlight::estimate::{Estimator, Saved};
use hyperlight::remaining::{self, Discharge};
use hyperlight::hid::{self, Battery, Status};
use hyperlight::update;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use config::Config;
use icon::Tone;

const WM_TRAY: u32 = WM_APP + 1;
const WM_STATUS: u32 = WM_APP + 2;
/// Sent by `hyperlight-tray.exe --configurator` to an already-running copy.
const WM_OPEN_CONFIGURATOR: u32 = WM_APP + 3;
/// A finished update check (lparam: Box<Option<String>>, the latest release tag).
const WM_UPDATE: u32 = WM_APP + 6;
const WINDOW_CLASS: &str = "HyperlightTrayWindow";
const TIMER_POLL: usize = 1;
const TIMER_SOON: usize = 2;
const TIMER_UPDATE: usize = 3;
const TRAY_UID: u32 = 1;

const ID_REFRESH: usize = 1;
const ID_CONFIGURATOR: usize = 2;
const ID_EXIT: usize = 3;
const ID_AUTOSTART: usize = 4;
const ID_UPDATE: usize = 5;
/// Always warn again here, even if the user's threshold is higher.
const CRITICAL: u8 = 5;
/// The battery has to climb this far above a threshold before that alert re-arms.
const REARM_MARGIN: u8 = 5;
/// How often the battery is checked. Each check is two 17-byte HID messages.
const POLL_MS: u32 = 30_000;
/// Opening the menu refreshes the reading if it is older than this.
const MENU_REFRESH_AGE: Duration = Duration::from_secs(60);
/// First update check, a little after start so it doesn't compete with sign-in.
const UPDATE_FIRST_MS: u32 = 15_000;
/// Then once a day.
const UPDATE_EVERY_MS: u32 = 24 * 60 * 60 * 1000;

const DBT_DEVNODES_CHANGED: usize = 0x0007;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;

static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let max = dst.len() - 1;
    let mut n = 0;
    for (d, c) in dst.iter_mut().zip(s.encode_utf16()).take(max) {
        *d = c;
        n += 1;
    }
    dst[n] = 0;
}

fn clock() -> String {
    let mut t: SYSTEMTIME = unsafe { mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    format!("{:02}:{:02}", t.wHour, t.wMinute)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn volts(mv: u16) -> String {
    format!("{}.{:02} V", mv / 1000, (mv % 1000) / 10)
}

struct App {
    hwnd: HWND,
    cfg: Config,
    /// Result of the most recent poll; None until the first one finishes.
    status: Option<Status>,
    /// Most recent good reading, kept while the mouse sleeps. Its `percent` is the
    /// smoothed value we display, not the firmware's raw figure.
    last: Option<(Battery, String, Instant)>,
    /// Smooths readings the way the official utility does.
    est: Estimator,
    est_device: [u8; 3],
    polling: bool,
    icon: HICON,
    in_tray: bool,
    warned_low: bool,
    warned_critical: bool,
    warned_full: bool,
    /// Recent discharge steps, for the time-remaining estimate.
    discharge: Discharge,
    /// Tag of a newer release on GitHub, if the last check found one.
    update: Option<String>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

impl App {
    fn nid(&self) -> NOTIFYICONDATAW {
        let mut n: NOTIFYICONDATAW = unsafe { mem::zeroed() };
        n.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        n.hWnd = self.hwnd;
        n.uID = TRAY_UID;
        n
    }

    /// "85% (~12h)", or just "85%" when there's no estimate (yet), it's switched
    /// off, or the mouse is charging.
    fn level_text(&self, b: &Battery) -> String {
        let left = (self.cfg.show_estimate && !b.charging)
            .then(|| self.discharge.hours_left(unix_now(), b.percent))
            .flatten();
        match left {
            Some(h) => format!("{}% ({})", b.percent, remaining::format_hours(h)),
            None => format!("{}%", b.percent),
        }
    }

    fn summary(&self) -> (String, String) {
        match (self.status, &self.last) {
            (None, _) => ("Checking…".into(), String::new()),
            (Some(Status::Battery(b)), _) => (
                format!("Battery {}", self.level_text(&b)),
                format!("{} · {}", if b.charging { "Charging" } else { "Not charging" }, volts(b.millivolts)),
            ),
            (Some(Status::MouseOffline), Some((b, at, _))) => {
                ("Mouse asleep or off".into(), format!("Last reading {}% at {}", b.percent, at))
            }
            (Some(Status::MouseOffline), None) => ("Mouse asleep or off".into(), String::new()),
            (Some(Status::NoReceiver), _) => ("No Hitscan receiver found".into(), String::new()),
        }
    }

    fn redraw(&mut self) {
        let light = icon::taskbar_is_light();
        // Colour follows the level (charging shows in the tooltip and menu).
        let (text, tone) = match (self.status, &self.last) {
            (Some(Status::Battery(b)), _) => (b.percent.to_string(), Tone::for_percent(b.percent)),
            (Some(Status::MouseOffline), Some((b, _, _))) => (b.percent.to_string(), Tone::Stale),
            _ => ("–".to_string(), Tone::Stale),
        };
        let badge = self.update.is_some().then(|| icon::badge_color(light));
        let new_icon = icon::make_icon(&text, icon::color(tone, light), icon::tray_icon_size(), badge);
        let (line1, line2) = self.summary();
        let tip = match self.status {
            Some(Status::Battery(b)) => format!("Mouse: {}\n{line2}", self.level_text(&b)),
            _ if line2.is_empty() => line1,
            _ => format!("{line1}\n{line2}"),
        };
        let tip = match &self.update {
            Some(tag) => format!("{tip}\nUpdate available: {tag}"),
            None => tip,
        };

        let mut n = self.nid();
        n.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
        n.uCallbackMessage = WM_TRAY;
        n.hIcon = new_icon;
        copy_wide(&mut n.szTip, &tip);
        unsafe {
            if !self.in_tray || Shell_NotifyIconW(NIM_MODIFY, &n) == 0 {
                self.in_tray = Shell_NotifyIconW(NIM_ADD, &n) != 0;
            }
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
        self.icon = new_icon;
    }

    fn remove_from_tray(&mut self) {
        let n = self.nid();
        unsafe { Shell_NotifyIconW(NIM_DELETE, &n) };
        self.in_tray = false;
    }

    fn notify(&self, title: &str, body: &str, warning: bool) {
        if !self.in_tray {
            return;
        }
        let mut n = self.nid();
        n.uFlags = NIF_INFO;
        n.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        copy_wide(&mut n.szInfoTitle, title);
        copy_wide(&mut n.szInfo, body);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &n) };
    }

    fn check_alerts(&mut self, b: Battery) {
        let t = self.cfg.threshold;
        if b.charging || b.percent >= t.saturating_add(REARM_MARGIN) {
            self.warned_low = false;
        }
        if b.charging || b.percent >= CRITICAL + REARM_MARGIN {
            self.warned_critical = false;
        }
        if !b.charging {
            self.warned_full = false;
        }

        if !b.charging {
            if t > CRITICAL && b.percent <= CRITICAL && !self.warned_critical {
                self.warned_critical = true;
                self.warned_low = true;
                self.notify(
                    "Mouse battery critical",
                    &format!("{}% left. Plug your mouse in soon.", b.percent),
                    true,
                );
            } else if t > 0 && b.percent <= t && !self.warned_low {
                self.warned_low = true;
                if t <= CRITICAL {
                    self.warned_critical = true;
                }
                self.notify("Mouse battery low", &format!("{}% left.", b.percent), true);
            }
        } else if self.cfg.notify_full && b.percent >= 100 && !self.warned_full {
            self.warned_full = true;
            self.notify("Mouse fully charged", "Your Hyperlight is at 100%.", false);
        }
    }

    fn on_status(&mut self, mut s: Status) {
        self.polling = false;
        if let Status::Battery(raw) = s {
            if raw.device_id != self.est_device {
                self.est = Estimator::default(); // a different mouse
                self.est_device = raw.device_id;
            }
            let now = unix_now();
            let before = self.est.shown();
            let saved = if before.is_none() { saved::load(raw.device_id) } else { None };
            let shown = self.est.update(&raw, now, saved);
            if before != Some(shown) {
                saved::store(raw.device_id, Saved { percent: shown, unix: now });
            }
            if self.discharge.observe(now, shown, raw.charging) {
                saved::store_discharge(&self.discharge);
            }
            let b = Battery { percent: shown, ..raw };
            s = Status::Battery(b);
            self.last = Some((b, clock(), Instant::now()));
            self.check_alerts(b);
        }
        self.status = Some(s);
        self.redraw();
        let (icon, text, detail) = self.status_row();
        menu::update_status(icon, text, detail);
    }

    fn schedule(&self) {
        unsafe { SetTimer(self.hwnd, TIMER_POLL, POLL_MS, None) };
    }

    fn reading_is_fresh(&self) -> bool {
        matches!(self.status, Some(Status::Battery(_)))
            && self.last.as_ref().is_some_and(|(_, _, at)| at.elapsed() < MENU_REFRESH_AGE)
    }

    /// Contents of the tray menu (drawn by menu.rs). Alert settings are on the
    /// configurator's Performance page; Start with Windows is in both places.
    /// Icon, main line and detail line for the menu's battery row.
    fn status_row(&self) -> (u16, String, String) {
        let (text, detail) = self.summary();
        let icon = match (self.status, &self.last) {
            (Some(Status::Battery(b)), _) if b.charging => menu::ICON_CHARGING,
            (Some(Status::Battery(b)), _) => menu::battery_icon(b.percent),
            (Some(Status::MouseOffline), Some((b, _, _))) => menu::battery_icon(b.percent),
            _ => menu::ICON_MOUSE,
        };
        (icon, text, detail)
    }

    fn menu_items(&self) -> Vec<menu::Item> {
        use menu::Item;
        let (icon, text, detail) = self.status_row();
        let mut items = vec![Item::Title {
            icon: menu::ICON_MOUSE,
            text: "Hyperlight Tray".into(),
            note: format!("v{}", env!("CARGO_PKG_VERSION")),
        }];
        if let Some(tag) = &self.update {
            // Opens the latest release on GitHub.
            items.push(Item::Link { id: ID_UPDATE, icon: menu::ICON_DOWNLOAD, text: format!("Update available: {tag}") });
        }
        items.extend([
            Item::Separator,
            // The small refresh button re-reads the battery and updates this row in place.
            Item::Status { icon, text, detail, button: Some((ID_REFRESH, menu::ICON_REFRESH)) },
            Item::Separator,
            // Time remaining is switched on and off in the configurator only.
            Item::Action { id: ID_CONFIGURATOR, icon: menu::ICON_SLIDERS, text: "Open Configurator...".into() },
            Item::Toggle {
                id: ID_AUTOSTART,
                icon: menu::ICON_STARTUP,
                text: "Start with Windows".into(),
                on: autostart::is_enabled(),
            },
            Item::Separator,
            Item::Action { id: ID_EXIT, icon: menu::ICON_POWER, text: "Exit".into() },
        ]);
        items
    }

    /// Result of an update check: `tag` is the latest release's tag, or None if
    /// the check failed (offline, no releases yet), which leaves things as they were.
    fn on_update(&mut self, tag: Option<String>) {
        let Some(tag) = tag.filter(|t| update::valid_tag(t)) else { return };
        let newer = (self.cfg.check_updates && update::is_newer(env!("CARGO_PKG_VERSION"), &tag)).then_some(tag);
        if newer != self.update {
            self.update = newer;
            self.redraw();
        }
    }

    fn set_check_updates(&mut self, on: bool) {
        self.cfg.check_updates = on;
        self.cfg.save();
        if on {
            unsafe { SetTimer(self.hwnd, TIMER_UPDATE, 1000, None) };
        } else if self.update.take().is_some() {
            self.redraw();
        }
    }

    fn set_show_estimate(&mut self, on: bool) {
        self.cfg.show_estimate = on;
        self.cfg.save();
        self.redraw();
    }

    /// The battery as the tray icon shows it, for the Stream Deck plugin.
    fn battery_info(&self) -> web::BatteryInfo {
        let (state, b) = match (self.status, &self.last) {
            (None, _) => ("checking", None),
            (Some(Status::Battery(b)), _) => ("battery", Some(b)),
            (Some(Status::MouseOffline), last) => ("asleep", last.as_ref().map(|(b, _, _)| *b)),
            (Some(Status::NoReceiver), _) => ("noReceiver", None),
        };
        let live = b.filter(|_| state == "battery");
        let remaining = live
            .filter(|b| self.cfg.show_estimate && !b.charging)
            .and_then(|b| self.discharge.hours_left(unix_now(), b.percent))
            .map(remaining::format_hours);
        web::BatteryInfo {
            state,
            percent: b.map(|b| b.percent),
            charging: live.is_some_and(|b| b.charging),
            remaining,
            update: self.update.clone(),
        }
    }

    /// Answers the configurator's Hyperlight Tray card (see web.rs), on the UI thread.
    fn api(&mut self, req: &mut web::ApiRequest) {
        match req.call {
            web::ApiCall::Get => {}
            web::ApiCall::Battery => {
                req.battery = Some(self.battery_info());
                return;
            }
            web::ApiCall::Refresh => {
                // Polls on the next timer tick (start_poll can't run while APP is borrowed).
                unsafe { SetTimer(self.hwnd, TIMER_SOON, 10, None) };
                req.battery = Some(self.battery_info());
                return;
            }
            web::ApiCall::TestNotification => {
                self.notify("Test notification", "Hyperlight Tray notifications are working.", false)
            }
            web::ApiCall::Set { threshold, notify_full, autostart: start, estimate, check_updates } => {
                if let Some(on) = check_updates {
                    self.set_check_updates(on);
                }
                if let Some(on) = estimate {
                    self.set_show_estimate(on);
                }
                if let Some(t) = threshold {
                    self.cfg.threshold = t;
                    self.warned_low = false;
                    self.warned_critical = false;
                    if let Some(Status::Battery(b)) = self.status {
                        self.check_alerts(b);
                    }
                    self.redraw();
                }
                if let Some(on) = notify_full {
                    self.cfg.notify_full = on;
                }
                if threshold.is_some() || notify_full.is_some() {
                    self.cfg.save();
                }
                if let Some(on) = start {
                    autostart::set(on);
                }
            }
        }
        req.result = Some(web::TrayInfo {
            threshold: self.cfg.threshold,
            notify_full: self.cfg.notify_full,
            autostart: autostart::is_enabled(),
            estimate: self.cfg.show_estimate,
            check_updates: self.cfg.check_updates,
            update: self.update.clone(),
        });
    }
}

/// Reads the battery on a worker thread so the UI never blocks on USB.
fn start_poll(hwnd: HWND) {
    let go = with_app(|a| !std::mem::replace(&mut a.polling, true)).unwrap_or(false);
    if !go {
        return;
    }
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let status = Box::into_raw(Box::new(hid::query()));
        unsafe {
            if PostMessageW(target as HWND, WM_STATUS, 0, status as LPARAM) == 0 {
                drop(Box::from_raw(status));
            }
        }
    });
}

/// Asks GitHub for the latest release on a worker thread. Set
/// HYPERLIGHT_FAKE_UPDATE=v9.9.9 to pretend a release exists (for testing).
fn start_update_check(hwnd: HWND) {
    if with_app(|a| a.cfg.check_updates) != Some(true) {
        return;
    }
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let tag = std::env::var("HYPERLIGHT_FAKE_UPDATE").ok().or_else(|| update::latest_tag(update::REPO));
        let tag = Box::into_raw(Box::new(tag));
        unsafe {
            if PostMessageW(target as HWND, WM_UPDATE, 0, tag as LPARAM) == 0 {
                drop(Box::from_raw(tag));
            }
        }
    });
}

fn open_url(url: &str) {
    let (verb, url) = (wide("open"), wide(url));
    unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), url.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

fn open_configurator() {
    if !web::open() {
        with_app(|a| {
            a.notify(
                "Couldn't open the configurator",
                "Ports 47820 to 47824 on this PC are all in use.",
                true,
            )
        });
    }
}

unsafe fn show_menu(hwnd: HWND) {
    if with_app(|a| a.reading_is_fresh()) == Some(false) {
        start_poll(hwnd);
    }
    let Some(items) = with_app(|a| a.menu_items()) else { return };
    menu::show(hwnd, items);
}

/// A menu item was chosen (posted by menu.rs).
unsafe fn run_command(hwnd: HWND, id: usize) {
    match id {
        ID_REFRESH => start_poll(hwnd),
        ID_CONFIGURATOR => open_configurator(),
        ID_AUTOSTART => autostart::set(!autostart::is_enabled()),
        ID_UPDATE => open_url(&update::release_page()),
        ID_EXIT => {
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let event = (lp as u32) & 0xFFFF;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                show_menu(hwnd);
            }
            0
        }
        web::WM_TRAY_API => {
            let req = &mut *(lp as *mut web::ApiRequest);
            with_app(|a| a.api(req));
            0
        }
        menu::WM_MENU_COMMAND => {
            run_command(hwnd, wp);
            0
        }
        WM_OPEN_CONFIGURATOR => {
            open_configurator();
            0
        }
        WM_STATUS => {
            let status = *Box::from_raw(lp as *mut Status);
            with_app(|a| a.on_status(status));
            0
        }
        WM_UPDATE => {
            let tag = *Box::from_raw(lp as *mut Option<String>);
            with_app(|a| a.on_update(tag));
            0
        }
        WM_TIMER if wp == TIMER_UPDATE => {
            SetTimer(hwnd, TIMER_UPDATE, UPDATE_EVERY_MS, None);
            start_update_check(hwnd);
            0
        }
        WM_TIMER => {
            if wp == TIMER_SOON {
                KillTimer(hwnd, TIMER_SOON);
            }
            start_poll(hwnd);
            0
        }
        WM_DEVICECHANGE => {
            // Receiver plugged or unplugged: re-check once things settle.
            if wp == DBT_DEVNODES_CHANGED {
                SetTimer(hwnd, TIMER_SOON, 2000, None);
            }
            TRUE as LRESULT
        }
        WM_POWERBROADCAST => {
            if wp == PBT_APMRESUMEAUTOMATIC {
                SetTimer(hwnd, TIMER_SOON, 5000, None);
            }
            TRUE as LRESULT
        }
        WM_SETTINGCHANGE | WM_DISPLAYCHANGE | WM_DPICHANGED => {
            // Light/dark taskbar switch or DPI change: repaint the number.
            with_app(|a| a.redraw());
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_DESTROY => {
            with_app(|a| a.remove_from_tray());
            PostQuitMessage(0);
            0
        }
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            // Explorer restarted: our icon is gone, add it back.
            with_app(|a| {
                a.in_tray = false;
                a.redraw();
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    unsafe {
        let mutex_name = wide("Local\\HyperlightTray.SingleInstance");
        let open_cfg = std::env::args().any(|a| a == "--configurator");
        let class = wide(WINDOW_CLASS);
        let _mutex = CreateMutexW(null(), TRUE, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Already running: ask that copy to open the page, then leave.
            if open_cfg {
                let other = FindWindowW(class.as_ptr(), null());
                if !other.is_null() {
                    AllowSetForegroundWindow(u32::MAX); // ASFW_ANY: let it bring the browser forward
                    PostMessageW(other, WM_OPEN_CONFIGURATOR, 0, 0);
                }
            }
            return;
        }

        let hinst = GetModuleHandleW(null());
        let mut wc: WNDCLASSW = mem::zeroed();
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = hinst;
        wc.lpszClassName = class.as_ptr();
        wc.hIcon = LoadIconW(hinst, 1 as _);
        RegisterClassW(&wc);

        TASKBAR_CREATED.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Ordering::Relaxed);

        // A hidden top-level window (not message-only) so we receive broadcasts like
        // WM_SETTINGCHANGE, WM_DEVICECHANGE and TaskbarCreated.
        let title = wide("Hyperlight Tray");
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            hinst,
            null(),
        );
        if hwnd.is_null() {
            return;
        }
        web::set_window(hwnd);
        // Always on, so the Stream Deck plugin can read the battery.
        web::serve();

        let (cfg, existed) = Config::load();
        if !existed {
            cfg.save();
        }
        autostart::refresh();

        APP.with(|cell| {
            *cell.borrow_mut() = Some(App {
                hwnd,
                cfg,
                status: None,
                last: None,
                est: Estimator::default(),
                est_device: [0; 3],
                polling: false,
                icon: null_mut(),
                in_tray: false,
                warned_low: false,
                warned_critical: false,
                warned_full: false,
                discharge: saved::load_discharge(),
                update: None,
            })
        });
        with_app(|a| {
            a.redraw();
            a.schedule();
            if !existed {
                a.notify(
                    "Hyperlight Tray is running",
                    "Your mouse battery shows in the tray. Click it for Start with \
                     Windows, or Open Configurator for alert settings.",
                    false,
                );
            }
        });
        start_poll(hwnd);
        SetTimer(hwnd, TIMER_UPDATE, UPDATE_FIRST_MS, None);
        if open_cfg {
            PostMessageW(hwnd, WM_OPEN_CONFIGURATOR, 0, 0);
        }

        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        with_app(|a| {
            if !a.icon.is_null() {
                DestroyIcon(a.icon);
            }
        });
    }
}
