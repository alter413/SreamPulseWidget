use crate::{
    glass,
    native::{self, Draw, MenuItem, Row, Shell, Widget},
};
use chrono::{DateTime, Local, Utc};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use steam_pulse::{
    layout,
    model::{self, LocalGame, Preferences, tr},
    online::{self, Friends, Offer, Update},
    steam::LibraryGame,
    windows_data::{self as os, Reader, Recent, Status, wide},
};
use tao::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{Event as Tao, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    platform::windows::{WindowBuilderExtWindows, WindowExtWindows},
    window::{Window, WindowBuilder, WindowId},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, RECT, WAIT_ABANDONED,
        WAIT_OBJECT_0,
    },
    System::Threading::{
        CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, OpenEventW, ReleaseMutex, SetEvent,
        WaitForSingleObject,
    },
    UI::WindowsAndMessaging::{IDYES, MB_ICONQUESTION, MB_YESNO, MessageBoxW, SetForegroundWindow},
};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const SETTINGS: &str = include_str!("../assets/settings.html");
const MENU: &str = include_str!("../assets/menu.html");
const SECOND: Duration = Duration::from_secs(1);
const BRIDGE: &str = r#"
    (() => {
        const events = new EventTarget();
        Object.defineProperty(window, "steamPulseBridge", {value: Object.freeze({
            postMessage: data => window.ipc.postMessage(JSON.stringify(data)),
            addEventListener: events.addEventListener.bind(events),
            removeEventListener: events.removeEventListener.bind(events),
            receive: data => events.dispatchEvent(new MessageEvent("message", {data}))
        })});
    })();
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    Settings,
    WidgetMenu,
}

pub enum Event {
    OpenSettings,
    OpenLibrary,
    Tray,
    Notification,
    Taskbar,
    Drag { center: f64, commit: bool },
    WidgetMenu,
    Ipc(PanelKind, u64, String),
    Status(Box<Status>),
    Library(Option<PathBuf>, Vec<LibraryGame>),
    Friends(u64, Friends),
    Sales(String, String, bool, Option<Vec<Offer>>),
    Update(Update, bool),
    Scan(isize, Vec<std::ops::Range<i32>>),
    Glass(PanelKind, u64, Option<String>),
    Artwork(String, Option<image::RgbaImage>),
    Stop,
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Instance(Handle);
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0.0);
        }
    }
}

fn instance(
    verify: bool,
    restart: bool,
    proxy: EventLoopProxy<Event>,
) -> std::io::Result<Option<Instance>> {
    let root = if verify {
        r"Local\SteamPulse.Verify."
    } else {
        r"Local\SteamPulse."
    };
    let mutex = wide(format!("{root}App"));
    let signal = wide(format!("{root}Settings"));
    unsafe {
        let handle = CreateMutexW(null(), 1, mutex.as_ptr());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let existing = GetLastError() == ERROR_ALREADY_EXISTS;
        let handle = Handle(handle);
        if existing && restart {
            if !matches!(
                WaitForSingleObject(handle.0, 30_000),
                WAIT_OBJECT_0 | WAIT_ABANDONED
            ) {
                return Ok(None);
            }
        } else if existing {
            let e = OpenEventW(EVENT_MODIFY_STATE, 0, signal.as_ptr());
            if !e.is_null() {
                SetEvent(e);
                CloseHandle(e);
            }
            return Ok(None);
        }
        let event = CreateEventW(null(), 0, 0, signal.as_ptr());
        if !event.is_null() {
            let raw = event as usize;
            thread::spawn(move || {
                let event = Handle(raw as HANDLE);
                while WaitForSingleObject(event.0, u32::MAX) == WAIT_OBJECT_0 {
                    if proxy.send_event(Event::OpenSettings).is_err() {
                        break;
                    }
                }
            });
        }
        Ok(Some(Instance(handle)))
    }
}

struct Panel {
    web: WebView,
    window: Window,
    id: u64,
    ready: bool,
    frame: Option<String>,
    frame_in_flight: bool,
    shown: bool,
    next_frame: Instant,
    glass_cache: Arc<Mutex<glass::Cache>>,
}

impl Panel {
    fn hwnd(&self) -> HWND {
        self.window.hwnd() as HWND
    }
    fn send(&self, value: Value) {
        let _ = self
            .web
            .evaluate_script(&format!("window.steamPulseBridge?.receive({value})"));
    }
}

struct App {
    p: Preferences,
    folder: PathBuf,
    locale: String,
    proxy: EventLoopProxy<Event>,
    verify: bool,
    shell: Shell,
    widget: Option<Widget>,
    settings: Option<Panel>,
    popup: Option<Panel>,
    web_context: WebContext,
    panel_seq: u64,
    startup: bool,
    save_failed: bool,
    save_at: Option<Instant>,
    sample_tx: mpsc::Sender<(Preferences, bool, bool)>,
    status: Status,
    busy_sample: bool,
    sample_at: Instant,
    library: Option<(Option<PathBuf>, Vec<LibraryGame>)>,
    library_busy: bool,
    library_requested: bool,
    open_library: bool,
    offers: Vec<Offer>,
    offer_message: &'static str,
    offer_card: Option<(Offer, Instant)>,
    notification: Option<u32>,
    sale_busy: bool,
    sale_at: Instant,
    friends: Friends,
    friend_account: u64,
    friends_busy: Option<u64>,
    friend_at: Instant,
    seen_apps: HashSet<String>,
    update: Option<Update>,
    update_busy: bool,
    update_at: Instant,
    widget_at: Instant,
    drag_position: Option<f64>,
    scan_at: Instant,
    scan_busy: bool,
    scan: Option<(isize, Vec<std::ops::Range<i32>>)>,
    memory_at: Instant,
    review_at: Instant,
    art_cache: HashMap<PathBuf, Option<Arc<image::RgbaImage>>>,
    remote_art: HashMap<String, Option<Arc<image::RgbaImage>>>,
}

struct Args {
    verify: bool,
    restart: bool,
    settings: bool,
    smoke: bool,
    fixture: Option<PathBuf>,
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut options = Args {
        verify: false,
        restart: false,
        settings: false,
        smoke: false,
        fixture: None,
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--verify") => options.verify = true,
            Some("--settings") => options.settings = true,
            Some("--memory-restart") => options.restart = true,
            Some("--smoke") => {
                options.smoke = true;
                options.verify = true;
            }
            Some("--verify-folder") => {
                options.verify = true;
                options.fixture = args.next().map(PathBuf::from);
            }
            _ => {}
        }
    }
    native::app_identity();
    let loop_: EventLoop<Event> = EventLoopBuilder::with_user_event().build();
    let proxy = loop_.create_proxy();
    let Some(instance) = instance(options.verify, options.restart, proxy.clone())? else {
        return Ok(());
    };
    let folder = if options.verify {
        options.fixture.clone().unwrap_or_else(|| {
            std::env::temp_dir().join(format!("SteamPulse-verify-{}", std::process::id()))
        })
    } else {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
            .join("SteamPulse")
    };
    std::fs::create_dir_all(&folder)?;
    let mut app = App::new(folder, &options, proxy.clone())?;
    if options.steam_settings() {
        let _ = proxy.send_event(Event::OpenSettings);
    }
    if options.smoke {
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(9));
            let _ = proxy.send_event(Event::Stop);
        });
    }
    let mut instance = Some(instance);
    loop_.run(move |event, target, control| {
        let mut stop = false;
        match event {
            Tao::UserEvent(event) => {
                stop = app.event(event, target);
            }
            Tao::WindowEvent {
                window_id, event, ..
            } => app.window(window_id, event),
            Tao::MainEventsCleared => stop = app.tick(),
            Tao::LoopDestroyed => {
                app.popup.take();
                app.settings.take();
                app.widget.take();
                instance.take();
            }
            _ => {}
        }
        *control = if stop {
            ControlFlow::Exit
        } else {
            let delay = if app.popup.is_some() || app.widget.as_ref().is_some_and(|w| w.animating) {
                16
            } else {
                80
            };
            ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(delay))
        };
    })
}

impl Args {
    fn steam_settings(&self) -> bool {
        self.verify || self.settings
    }
}

impl App {
    fn new(folder: PathBuf, options: &Args, proxy: EventLoopProxy<Event>) -> std::io::Result<Self> {
        let p = Preferences::load(&folder);
        let now = Instant::now();
        let (tx, rx) = mpsc::channel::<(Preferences, bool, bool)>();
        let send = proxy.clone();
        thread::spawn(move || {
            let mut reader = Reader::default();
            while let Ok((p, visuals, recent)) = rx.recv() {
                if send
                    .send_event(Event::Status(Box::new(reader.sample(&p, visuals, recent))))
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok(Self {
            p,
            locale: os::system_locale(),
            shell: Shell::new(proxy.clone(), !options.verify)?,
            web_context: WebContext::new(Some(folder.join("WebViewRust"))),
            folder,
            proxy,
            verify: options.verify,
            widget: None,
            settings: None,
            popup: None,
            panel_seq: 0,
            startup: !options.verify && os::startup(),
            save_failed: false,
            save_at: None,
            sample_tx: tx,
            status: Status::default(),
            busy_sample: false,
            sample_at: now,
            library: None,
            library_busy: false,
            library_requested: false,
            open_library: false,
            offers: Vec::new(),
            offer_message: "loadingSales",
            offer_card: None,
            notification: None,
            sale_busy: false,
            sale_at: now,
            friends: Friends::default(),
            friend_account: 0,
            friends_busy: None,
            friend_at: now,
            seen_apps: HashSet::new(),
            update: None,
            update_busy: false,
            update_at: now,
            widget_at: now,
            drag_position: None,
            scan_at: now,
            scan_busy: false,
            scan: None,
            memory_at: now
                + if options.restart {
                    Duration::from_secs(60)
                } else {
                    SECOND
                },
            review_at: now + Duration::from_secs(3 * 3600),
            art_cache: HashMap::new(),
            remote_art: HashMap::new(),
        })
    }

    fn lang(&self) -> &str {
        model::language(self.p.text("Language"), &self.locale)
    }
    fn t<'a>(&self, key: &'a str) -> &'a str {
        tr(self.lang(), key)
    }

    fn tick(&mut self) -> bool {
        let now = Instant::now();
        self.shell.maintain_tray(now);
        if self.popup.as_ref().is_some_and(|panel| {
            let anchor = self.widget.as_ref().map(|widget| widget.bounds);
            native::outside_popup_press(panel.hwnd(), anchor)
        }) {
            self.popup.take();
        }
        if self.save_at.is_some_and(|at| now >= at) {
            self.save();
        }
        if now >= self.sample_at && !self.busy_sample {
            let visuals = self.p.flag("Widget") || self.settings.is_some();
            let recent = self.p.flag("MenuRecent")
                && self
                    .p
                    .local_games()
                    .iter()
                    .any(|g| !g.ignored && model::standalone(Path::new(&g.path)));
            if visuals || recent {
                self.busy_sample = self
                    .sample_tx
                    .send((self.p.clone(), visuals, recent))
                    .is_ok();
            }
            self.sample_at = now
                + Duration::from_secs(if visuals {
                    self.p.number("MetricsInterval").clamp(1., 5.) as u64
                } else {
                    2
                });
        }
        let needs_friends = self.status.active
            && (self.p.flag("Friends") && (self.p.flag("Widget") || self.settings.is_some())
                || self.popup.is_some() && self.p.flag("MenuFriends"));
        if needs_friends
            && self.status.account != 0
            && self.friends_busy.is_none()
            && now >= self.friend_at
            && !self.verify
        {
            let account = self.status.account;
            self.friends_busy = Some(account);
            self.friend_at = now + Duration::from_secs(120);
            let tx = self.proxy.clone();
            thread::spawn(move || {
                let _ = tx.send_event(Event::Friends(account, online::friends(account)));
            });
        }
        if self.p.flag("Discounts") && !self.verify && now >= self.sale_at {
            self.sales(true);
        }
        if self.p.flag("AutoUpdate") && !self.verify && now >= self.update_at {
            self.check_update();
        }
        if (self.widget.is_some() || self.p.flag("Widget")) && now >= self.widget_at {
            self.refresh_widget();
        }
        self.frames(now);
        if !self.verify && now >= self.memory_at {
            self.memory_at = now + SECOND;
            let review = now >= self.review_at;
            if review {
                self.review_at = now + Duration::from_secs(3 * 3600);
            }
            if native::working_set().is_some_and(|bytes| layout::should_restart(bytes, review))
                && self.save()
                && std::env::current_exe()
                    .and_then(|exe| {
                        std::process::Command::new(exe)
                            .arg("--memory-restart")
                            .spawn()
                    })
                    .is_ok()
            {
                return true;
            }
        }
        false
    }

    fn event(&mut self, event: Event, target: &EventLoopWindowTarget<Event>) -> bool {
        match event {
            Event::OpenSettings => self.open_settings(target),
            Event::OpenLibrary => {
                self.open_settings(target);
                self.open_library = true;
                self.show_library();
            }
            Event::Tray => {
                if self.tray(target) {
                    return true;
                }
            }
            Event::Notification => {
                if let Some(id) = self.notification {
                    self.open_offer(id);
                }
            }
            Event::Taskbar => {
                self.shell.readd();
                self.update_tray_tip();
                self.scan = None;
                self.scan_at = Instant::now();
                self.widget_at = Instant::now();
            }
            Event::Drag { center, commit } => {
                if !self.p.flag("WidgetFrozen")
                    && !self.p.flag("ClickThrough")
                    && let Some(display) = self.display()
                {
                    let (_, bar) = native::taskbar(&display);
                    let width = bar.right - bar.left;
                    if width > 0 {
                        let position = (100. * (center - f64::from(bar.left)) / f64::from(width))
                            .clamp(0., 100.);
                        if commit {
                            self.drag_position = None;
                            self.p.set(
                                "HorizontalPosition",
                                json!((position * 100.).round() / 100.),
                            );
                            self.changed();
                            self.sync();
                        } else {
                            self.drag_position = Some(position);
                        }
                        self.refresh_widget();
                    }
                } else if commit {
                    self.drag_position = None;
                }
            }
            Event::WidgetMenu => self.widget_menu(target),
            Event::Ipc(kind, id, body) => {
                if body.len() <= 2_048
                    && self.panel(kind).is_some_and(|panel| panel.id == id)
                    && let Ok(v) = serde_json::from_str::<Value>(&body)
                {
                    match kind {
                        PanelKind::Settings => self.settings_message(&v),
                        PanelKind::WidgetMenu => self.menu_message(&v),
                    }
                }
            }
            Event::Status(status) => {
                self.busy_sample = false;
                if status.account != self.friend_account {
                    self.friend_account = status.account;
                    self.friends = Friends::default();
                    self.remote_art.clear();
                    self.friend_at = Instant::now();
                }
                let arrivals: Vec<_> = status
                    .launches
                    .iter()
                    .filter(|g| !self.status.windows.contains_key(&g.id))
                    .cloned()
                    .map(|mut g| {
                        if !self.seen_apps.insert(g.id.clone()) {
                            g.started = Utc::now();
                        }
                        g
                    })
                    .collect();
                if !arrivals.is_empty() {
                    self.remember_recent(arrivals);
                }
                self.status = *status;
                self.update_tray_tip();
                if self.library_requested
                    && self
                        .library
                        .as_ref()
                        .is_some_and(|(root, _)| *root != self.status.root)
                {
                    self.load_library(false);
                }
                self.send_status();
                self.sync_menu();
                self.widget_at = Instant::now();
            }
            Event::Library(root, list) => {
                self.library_busy = false;
                self.library = Some((root, list));
                if self.settings.is_some() {
                    self.load_library(false);
                }
            }
            Event::Friends(account, snapshot) => {
                self.friends_busy = None;
                if account == self.friend_account {
                    if self.p.flag("WidgetMenu") && self.p.flag("MenuFriends") {
                        self.remember_friends(account, &snapshot);
                    }
                    self.friends = snapshot;
                    self.sync_menu();
                    self.send_status();
                    self.widget_at = Instant::now();
                }
            }
            Event::Sales(country, locale, notify, offers) => {
                self.sale_busy = false;
                if country == self.country() && locale == self.lang() {
                    self.receive_sales(notify, offers);
                } else {
                    self.sale_at = Instant::now();
                }
            }
            Event::Update(update, done) => {
                if done {
                    self.update_busy = false;
                }
                self.update = Some(update);
                self.send_update();
            }
            Event::Scan(hwnd, hits) => {
                self.scan_busy = false;
                if !hits.is_empty() {
                    self.scan = Some((hwnd, hits));
                }
                self.widget_at = Instant::now();
            }
            Event::Glass(kind, id, frame) => {
                let key = if kind == PanelKind::Settings {
                    "SettingsGlass"
                } else {
                    "MenuGlass"
                };
                let enabled = self.p.flag(key);
                if let Some(panel) = self.panel_mut(kind).filter(|panel| panel.id == id) {
                    panel.frame_in_flight = false;
                    panel.next_frame = Instant::now() + Duration::from_millis(140);
                    if let Some(frame) = frame.filter(|_| enabled) {
                        panel.frame = Some(frame);
                        panel.send(json!({"type":"glass","frame":panel.frame}));
                    }
                }
            }
            Event::Artwork(url, artwork) => {
                if let Some(entry) = self.remote_art.get_mut(&url) {
                    *entry = artwork.map(Arc::new);
                    self.widget_at = Instant::now();
                }
            }
            Event::Stop => {
                if self.verify {
                    let ready = self.settings.as_ref().is_some_and(|p| p.ready);
                    let check = json!({"settingsReady":ready,"widget":self.widget.is_some(),"libraryReady":self.library.is_some(),"steamFound":self.status.root.is_some(),"steamActive":self.status.active});
                    let _ = std::fs::write(self.folder.join("smoke.json"), check.to_string());
                }
                if self.save_at.is_some() {
                    self.save();
                }
                return true;
            }
        }
        false
    }

    fn panel(&self, kind: PanelKind) -> Option<&Panel> {
        match kind {
            PanelKind::Settings => self.settings.as_ref(),
            PanelKind::WidgetMenu => self.popup.as_ref(),
        }
    }

    fn panel_mut(&mut self, kind: PanelKind) -> Option<&mut Panel> {
        match kind {
            PanelKind::Settings => self.settings.as_mut(),
            PanelKind::WidgetMenu => self.popup.as_mut(),
        }
    }

    fn new_panel(&mut self, window: Window, html: &str, kind: PanelKind) -> wry::Result<Panel> {
        self.panel_seq += 1;
        let id = self.panel_seq;
        let tx = self.proxy.clone();
        let html = html.replace("window.chrome?.webview", "window.steamPulseBridge");
        let backdrop = match (kind, self.p.flag("LightTheme")) {
            (PanelKind::WidgetMenu, true) => (245, 248, 252, 255),
            (PanelKind::WidgetMenu, false) => (20, 30, 43, 255),
            (PanelKind::Settings, true) => (243, 245, 248, 255),
            (PanelKind::Settings, false) => (17, 18, 22, 255),
        };
        let web = WebViewBuilder::new_with_web_context(&mut self.web_context)
            .with_html(html)
            .with_background_color(backdrop)
            .with_initialization_script(BRIDGE)
            .with_navigation_handler(|url| {
                url == "about:blank" || url.starts_with("data:text/html")
            })
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_browser_accelerator_keys(false)
            .with_default_context_menus(false)
            .with_clipboard(false)
            .with_devtools(false)
            .with_ipc_handler(move |req| {
                if req.uri() == "about:blank" && req.body().len() <= 2048 {
                    let _ = tx.send_event(Event::Ipc(kind, id, req.into_body()));
                }
            })
            .build(&window)?;
        Ok(Panel {
            web,
            window,
            id,
            ready: false,
            frame: None,
            frame_in_flight: false,
            shown: false,
            next_frame: Instant::now(),
            glass_cache: Arc::new(Mutex::new(glass::Cache::default())),
        })
    }

    fn open_settings(&mut self, target: &EventLoopWindowTarget<Event>) {
        if let Some(panel) = &self.settings {
            panel.window.set_minimized(false);
            panel.window.set_visible(true);
            unsafe {
                SetForegroundWindow(panel.hwnd());
            }
            return;
        }
        let height: f64 = self.display().map_or(760., |d| {
            (f64::from(d.work.bottom - d.work.top) / d.dpi).clamp(485., 760.)
        });
        let window = WindowBuilder::new()
            .with_title(format!("Steam Pulse · {}", self.t("settings")))
            .with_visible(false)
            .with_inner_size(LogicalSize::new(890., height - 40.))
            .build(target);
        let Ok(window) = window else {
            return;
        };
        if let Some(display) = self.display() {
            let width = (910. * display.dpi).round() as i32;
            let height = (height * display.dpi).round() as i32;
            window.set_outer_position(PhysicalPosition::new(
                display.work.left + (display.work.right - display.work.left - width) / 2,
                display.work.top + (display.work.bottom - display.work.top - height) / 2,
            ));
        }
        let outer = window.outer_size();
        let inner = window.inner_size();
        let frame_width = outer.width.saturating_sub(inner.width);
        let frame_height = outer.height.saturating_sub(inner.height);
        let dpi = window.scale_factor();
        let client = |width: f64, height: f64| {
            PhysicalSize::new(
                ((width * dpi).round() as u32).saturating_sub(frame_width),
                ((height * dpi).round() as u32).saturating_sub(frame_height),
            )
        };
        window.set_inner_size(client(910., height));
        window.set_min_inner_size(Some(client(650., 485.)));
        native::window_icons(window.hwnd() as HWND);
        match self.new_panel(window, SETTINGS, PanelKind::Settings) {
            Ok(panel) => {
                native::caption(panel.hwnd(), self.p.flag("LightTheme"));
                panel.window.set_visible(true);
                self.settings = Some(panel);
                self.library_requested = false;
                self.sample_at = Instant::now();
            }
            Err(error) => {
                eprintln!("{error}");
                if self.verify {
                    let _ =
                        std::fs::write(self.folder.join("webview-error.txt"), error.to_string());
                }
                native::error(self.t("runtimeError"));
            }
        }
    }

    fn close_settings(&mut self) {
        self.settings.take();
        self.library = None;
        self.library_requested = false;
        self.sample_at = Instant::now();
    }

    fn widget_menu(&mut self, target: &EventLoopWindowTarget<Event>) {
        if self.popup.take().is_some() || !self.p.flag("WidgetMenu") || self.p.flag("ClickThrough")
        {
            return;
        }
        let (Some(widget), Some(display)) = (self.widget.as_ref(), self.display()) else {
            return;
        };
        let b = widget.bounds;
        let scale = display.dpi;
        let width = (374. * scale).round() as i32;
        let height = ((self.menu_height() as f64 * scale).round() as i32)
            .min(display.work.bottom - display.work.top);
        let x = ((b.left + b.right - width) / 2).clamp(
            display.work.left,
            (display.work.right - width).max(display.work.left),
        );
        let gap = (8. * scale).round() as i32;
        let y = if b.top > display.work.top + height + gap {
            b.top - height - gap
        } else {
            b.bottom + gap
        };
        let y = y.clamp(
            display.work.top,
            (display.work.bottom - height).max(display.work.top),
        );
        let window = WindowBuilder::new()
            .with_title("Steam Pulse")
            .with_decorations(false)
            .with_undecorated_shadow(false)
            .with_resizable(false)
            .with_visible(false)
            .with_always_on_top(true)
            .with_skip_taskbar(true)
            .with_position(PhysicalPosition::new(x, y))
            .with_inner_size(PhysicalSize::new(width, height))
            .build(target);
        if let Ok(window) = window {
            let bounds = RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
            };
            native::place_popup(window.hwnd() as HWND, bounds, self.p.number("MenuRadius"));
            let Ok(panel) = self.new_panel(window, MENU, PanelKind::WidgetMenu) else {
                return;
            };
            self.popup = Some(panel);
            self.friend_at = Instant::now();
        }
    }

    fn menu_height(&self) -> i32 {
        let rows =
            44 + if self.p.flag("MenuRecent") {
                70 + self.recent_rows().len().max(1) as i32 * 38
            } else {
                0
            } + if self.p.flag("MenuNotices") {
                64 + self.menu_offers().len().max(1) as i32 * 34
            } else {
                0
            } + if self.p.flag("MenuFriends") {
                66 + self.friends.people.len().clamp(1, 5) as i32 * 38
            } else {
                0
            };
        rows.clamp(112, 570)
    }

    fn window(&mut self, id: WindowId, event: WindowEvent<'_>) {
        if self.settings.as_ref().is_some_and(|p| p.window.id() == id) {
            match event {
                WindowEvent::CloseRequested => self.close_settings(),
                WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                    if let Some(p) = &mut self.settings {
                        p.next_frame = Instant::now();
                    }
                }
                _ => {}
            }
        } else if self.popup.as_ref().is_some_and(|p| p.window.id() == id)
            && matches!(event, WindowEvent::CloseRequested)
        {
            self.popup.take();
        }
    }

    fn settings_message(&mut self, v: &Value) {
        let action = v["action"].as_str().unwrap_or_default();
        match action {
            "ready" => {
                if let Some(p) = &mut self.settings {
                    p.ready = true;
                }
                self.sync();
                self.show_library();
                if self.verify {
                    let _ = std::fs::write(self.folder.join("settings-ready"), VERSION);
                    self.load_library(false);
                }
            }
            "status" => self.send_status(),
            "glassReady" => {
                if let Some(p) = &mut self.settings {
                    p.next_frame = Instant::now();
                    p.send(json!({"type":"glass","frame":p.frame}));
                }
            }
            "change" => {
                let key = v["key"].as_str().unwrap_or_default();
                if key == "AutoStart" {
                    if let Some(on) = v["value"].as_bool() {
                        if self.verify || os::set_startup(on).is_ok() {
                            self.startup = on;
                        } else {
                            self.settings_error("startupError");
                        }
                        self.sync();
                    }
                    return;
                }
                let monitors = if key == "MonitorName" {
                    native::displays()
                } else {
                    Vec::new()
                };
                let names: Vec<_> = monitors.iter().map(|d| d.name.as_str()).collect();
                if !self.p.edit(key, &v["value"], &names) {
                    return;
                }
                if key == "LightTheme" {
                    let now = Instant::now();
                    self.save_at = Some(now + Duration::from_millis(350));
                    self.widget_at = now;
                    if let Some(panel) = &self.settings {
                        native::caption(panel.hwnd(), self.p.flag("LightTheme"));
                    }
                    self.sync_menu();
                    return;
                }
                if key == "Language"
                    && let Some(panel) = &self.settings
                {
                    panel
                        .window
                        .set_title(&format!("Steam Pulse · {}", self.t("settings")));
                }
                if ["Discounts", "Country", "MinDiscount", "Language"].contains(&key) {
                    self.sale_at = Instant::now() + Duration::from_secs(4);
                }
                if key == "AutoUpdate" {
                    self.update_at = Instant::now();
                }
                self.changed();
                if key == "Language" {
                    self.sync();
                }
            }
            "addLocalGame" => {
                let Some(owner) = self.settings.as_ref().map(Panel::hwnd) else {
                    return;
                };
                let mut games = self.p.local_games();
                if games.len() >= 64 {
                    return;
                }
                let Some(path) = native::choose_exe(owner, self.t("addLocalGame")) else {
                    return;
                };
                if !path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
                    || games
                        .iter()
                        .any(|g| model::matches_executable(Path::new(&g.path), &path))
                {
                    return;
                }
                games.push(LocalGame {
                    id: uuid::Uuid::new_v4().simple().to_string(),
                    name: path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .chars()
                        .take(90)
                        .collect(),
                    path: path.to_string_lossy().into_owned(),
                    ignored: false,
                });
                self.set_local(games);
            }
            "removeLocalGame" | "ignoreLocalGame" | "renameLocalGame" => {
                let Some(id) = v["id"].as_str() else {
                    return;
                };
                let mut games = self.p.local_games();
                if action == "removeLocalGame" {
                    games.retain(|g| g.id != id);
                } else if let Some(game) = games.iter_mut().find(|g| g.id == id) {
                    if action == "ignoreLocalGame"
                        && let Some(ignored) = v["ignored"].as_bool()
                    {
                        game.ignored = ignored;
                    }
                    if action == "renameLocalGame"
                        && let Some(name) = v["name"]
                            .as_str()
                            .map(str::trim)
                            .filter(|s| (1..=90).contains(&s.encode_utf16().count()))
                    {
                        game.name = name.to_owned();
                    }
                }
                self.set_local(games);
            }
            "library" => self.load_library(v["refresh"] == true),
            "ignore" | "openSteamGame" => {
                if let Some(id) = v["id"]
                    .as_u64()
                    .and_then(|id| u32::try_from(id).ok())
                    .filter(|id| {
                        self.library
                            .as_ref()
                            .is_some_and(|(_, games)| games.iter().any(|g| g.id == *id))
                    })
                {
                    if action == "openSteamGame" {
                        if !os::steam_app(id) {
                            self.settings_error("storeError");
                        }
                    } else if let Some(ignore) = v["ignored"].as_bool() {
                        let mut ids = self
                            .p
                            .get("IgnoredGames")
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        ids.retain(|n| n != &json!(id));
                        if ignore {
                            ids.push(json!(id));
                        }
                        self.p.set("IgnoredGames", json!(ids));
                        self.changed();
                    }
                }
            }
            "reset" => self.reset(),
            "close" => self.close_settings(),
            "checkUpdates" => self.check_update(),
            "showUpdate" => {
                if let Some(folder) = self
                    .update
                    .as_ref()
                    .and_then(|u| u.path.as_ref())
                    .filter(|p| p.is_file())
                    .and_then(|p| p.parent())
                    && !os::shell_open(folder)
                {
                    self.update = Some(Update::status("updateFailed"));
                    self.send_update();
                }
            }
            "icons" => {
                os::shell_open("https://www.flaticon.com/uicons");
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        let owner = self.settings.as_ref().map_or(null_mut(), Panel::hwnd);
        let answer = unsafe {
            MessageBoxW(
                owner,
                wide(self.t("resetPrompt")).as_ptr(),
                wide("Steam Pulse").as_ptr(),
                MB_YESNO | MB_ICONQUESTION,
            )
        };
        if answer != IDYES {
            return;
        }
        if !self.verify && self.startup {
            if os::set_startup(false).is_err() {
                self.settings_error("startupError");
            } else {
                self.startup = false;
            }
        } else {
            self.startup = false;
        }
        let mut fresh = Preferences::default();
        for key in [
            "SeenOffers",
            "Widget",
            "LocalGames",
            "RecentLaunches",
            "FriendHistory",
            "MenuOffers",
        ] {
            fresh.set(key, self.p.get(key).clone());
        }
        self.p = fresh;
        self.changed();
        self.sync();
    }

    fn settings_error(&self, key: &str) {
        if let Some(p) = &self.settings {
            p.send(json!({"type":"error","key":key}));
        }
    }

    fn changed(&mut self) {
        self.save_at = Some(Instant::now() + Duration::from_millis(350));
        self.sample_at = Instant::now();
        self.widget_at = Instant::now();
        if !self.p.flag("Widget") {
            self.widget.take();
            self.art_cache.clear();
            self.remote_art.clear();
            self.scan = None;
        }
        if !self.p.flag("Widget") || !self.p.flag("WidgetMenu") || self.p.flag("ClickThrough") {
            self.popup.take();
        }
        self.sync_menu();
        self.update_tray_tip();
    }

    fn update_tray_tip(&self) {
        let label = if self.status.active {
            "connected"
        } else if self.status.root.is_some() {
            "closed"
        } else {
            "notFound"
        };
        let line = if self.p.flag("Name")
            && self.p.flag("Downloads")
            && let Some(d) = self
                .status
                .download
                .as_ref()
                .filter(|d| !self.p.ignored(d.game.id))
        {
            format!("{} · {}", self.t("download"), d.game.name)
        } else if self.p.flag("Name")
            && self.p.flag("Games")
            && let Some(g) = self.status.game.as_ref().filter(|g| !self.p.ignored(g.id))
        {
            format!("{} · {}", self.t("inGame"), g.name)
        } else {
            self.t(label).to_owned()
        };
        self.shell.tooltip(&format!("Steam Pulse\n{line}"));
    }

    fn save(&mut self) -> bool {
        self.save_at = None;
        self.save_failed = self.p.save(&self.folder).is_err();
        if let Some(p) = &self.settings {
            p.send(json!({"type":"save","failed":self.save_failed}));
        }
        !self.save_failed
    }

    fn set_local(&mut self, games: Vec<LocalGame>) {
        self.p.set("LocalGames", json!(games));
        self.remember_recent(Vec::new());
        self.changed();
        self.send_local();
    }

    fn send_local(&self) {
        if let Some(p) = &self.settings {
            let games: Vec<_> = self
                .p
                .local_games()
                .iter()
                .map(|g| {
                    json!({"id":g.id,"name":g.name,"ignored":g.ignored,
                "file":Path::new(&g.path).file_name().unwrap_or_default().to_string_lossy()})
                })
                .collect();
            p.send(json!({"type":"localGames","games":games}));
        }
    }

    fn sync(&self) {
        if let Some(p) = &self.settings {
            p.send(json!({"type":"settings","entries":self.p.entries(self.startup),"language":self.lang(),"saveFailed":self.save_failed,"version":VERSION}));
            p.send(json!({"type":"monitors","displays":native::displays()}));
            self.send_local();
            self.send_update();
            self.send_status();
        }
    }

    fn send_status(&self) {
        if let Some(p) = &self.settings {
            let status = &self.status;
            let friends = self.p.flag("Friends").then_some(&self.friends);
            p.send(
                json!({"type":"status","load":status.load,"friends":friends,"active":status.active,
                "available":status.root.is_some(),"game":status.game,"download":status.download}),
            );
        }
    }

    fn show_library(&mut self) {
        if self.open_library
            && let Some(panel) = self.settings.as_ref().filter(|panel| panel.ready)
        {
            self.open_library = false;
            panel.send(json!({"type":"section","value":"games"}));
        }
    }

    fn load_library(&mut self, force: bool) {
        self.library_requested = true;
        if self.library_busy {
            return;
        }
        if !force
            && let Some((root, games)) = self
                .library
                .as_ref()
                .filter(|(root, _)| *root == self.status.root)
        {
            let _ = root;
            if let Some(p) = &self.settings {
                p.send(json!({"type":"library","games":games}));
            }
            return;
        }
        let root = self.status.root.clone();
        let tx = self.proxy.clone();
        self.library_busy = true;
        thread::spawn(move || {
            let games = os::read_library(root.as_deref());
            let _ = tx.send_event(Event::Library(root, games));
        });
    }

    fn send_update(&self) {
        if let (Some(p), Some(u)) = (&self.settings, &self.update) {
            p.send(
                json!({"type":"update","key":u.key,"version":u.version,"ready":u.path.is_some()}),
            );
        }
    }

    fn check_update(&mut self) {
        if self.update_busy {
            return;
        }
        self.update_busy = true;
        self.update_at = Instant::now() + Duration::from_secs(6 * 3600);
        self.update = Some(Update::status("updateChecking"));
        self.send_update();
        let folder = self.folder.join("Updates");
        let tx = self.proxy.clone();
        thread::spawn(move || {
            let status = online::update(&folder, |status| {
                let _ = tx.send_event(Event::Update(status, false));
            });
            let _ = tx.send_event(Event::Update(status, true));
        });
    }

    fn country(&self) -> String {
        let chosen = self.p.text("Country");
        if !chosen.is_empty() {
            return chosen.to_owned();
        }
        self.locale
            .split(['-', '_'])
            .skip(1)
            .find(|s| s.len() == 2 && s.bytes().all(|c| c.is_ascii_alphabetic()))
            .unwrap_or("US")
            .to_owned()
    }

    fn sales(&mut self, notify: bool) {
        if self.sale_busy {
            return;
        }
        self.sale_busy = true;
        self.sale_at = Instant::now() + Duration::from_secs(35 * 60);
        let country = self.country();
        let locale = self.lang().to_owned();
        let tx = self.proxy.clone();
        thread::spawn(move || {
            let offers = online::offers(&country, &locale).ok();
            let _ = tx.send_event(Event::Sales(country, locale, notify, offers));
        });
    }

    fn receive_sales(&mut self, notify: bool, offers: Option<Vec<Offer>>) {
        let Some(offers) = offers else {
            self.offer_message = "storeError";
            return;
        };
        let min = self.p.number("MinDiscount") as i32;
        let mut seen_ids = HashSet::new();
        self.offers = offers
            .into_iter()
            .filter(|o| o.discount >= min && seen_ids.insert(o.id))
            .collect();
        self.offers.sort_by_key(|o| std::cmp::Reverse(o.discount));
        self.offer_message = if self.offers.is_empty() {
            "noOffers"
        } else {
            ""
        };
        if !notify || !self.p.flag("Discounts") {
            return;
        }
        let previous = self.p.get("SeenOffers").clone();
        let mapping: HashMap<_, _> = self.offers.iter().map(|o| (o.id, o.discount)).collect();
        let fresh: Vec<_> = self
            .offers
            .iter()
            .filter(|o| previous[o.id.to_string()].as_i64().unwrap_or(0) < i64::from(o.discount))
            .cloned()
            .collect();
        let value = json!(mapping);
        if previous != value {
            self.p.set("SeenOffers", value);
            self.save_at = Some(Instant::now());
        }
        if let Some(first) = fresh.first() {
            self.shell.notify(
                self.t("sales"),
                &format!(
                    "{} · −{}%{}",
                    first.name,
                    first.discount,
                    if fresh.len() > 1 {
                        format!("  •  +{}", fresh.len() - 1)
                    } else {
                        String::new()
                    }
                ),
            );
            self.offer_card = Some((first.clone(), Instant::now() + Duration::from_secs(25)));
            self.notification = Some(first.id);
            let mut ids = HashSet::new();
            self.p.set(
                "MenuOffers",
                json!(
                    fresh
                        .into_iter()
                        .chain(self.menu_offers())
                        .filter(|o| ids.insert(o.id))
                        .take(3)
                        .collect::<Vec<_>>()
                ),
            );
            self.save_at = Some(Instant::now());
            self.sync_menu();
        }
    }

    fn menu_offers(&self) -> Vec<Offer> {
        self.p
            .get("MenuOffers")
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect()
    }

    fn remember_recent(&mut self, arrivals: Vec<Recent>) {
        let known: HashSet<_> = self
            .p
            .local_games()
            .into_iter()
            .filter(|g| !g.ignored && model::standalone(Path::new(&g.path)))
            .map(|g| g.id)
            .collect();
        let mut values: Vec<Recent> = self
            .p
            .get("RecentLaunches")
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .chain(arrivals)
            .filter(|r| known.contains(&r.id))
            .collect();
        values.sort_by_key(|r| std::cmp::Reverse(r.started));
        let mut ids = HashSet::new();
        let values = json!(
            values
                .into_iter()
                .filter(|r| ids.insert(r.id.clone()))
                .take(3)
                .collect::<Vec<_>>()
        );
        if &values != self.p.get("RecentLaunches") {
            self.p.set("RecentLaunches", values);
            self.save_at = Some(Instant::now() + Duration::from_millis(350));
        }
    }

    fn remember_friends(&mut self, account: u64, snapshot: &Friends) {
        if !snapshot.available || account == 0 {
            return;
        }
        let account = account.to_string();
        let all = self
            .p
            .get("FriendHistory")
            .as_object()
            .cloned()
            .unwrap_or_default();
        let previous = all
            .get(&account)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut next = serde_json::Map::new();
        for friend in &snapshot.people {
            let id = friend.id.to_string();
            let saved = previous.get(&id);
            let changed = saved.and_then(|v| v["Game"].as_str()) != Some(friend.game.as_str())
                || self
                    .friends
                    .people
                    .iter()
                    .find(|f| f.id == friend.id)
                    .is_some_and(|before| before.game != friend.game);
            if !friend.game.is_empty() && changed {
                next.insert(id, json!({"Game":friend.game,"Observed":Utc::now()}));
            } else if let Some(saved) = saved {
                next.insert(id, saved.clone());
            }
        }
        if next != previous {
            let mut all = all;
            all.insert(account, json!(next));
            self.p.set("FriendHistory", json!(all));
            self.save_at = Some(Instant::now() + Duration::from_millis(350));
        }
    }

    fn recent_rows(&self) -> Vec<(LocalGame, Option<String>)> {
        let eligible: Vec<_> = self
            .p
            .local_games()
            .into_iter()
            .filter(|g| !g.ignored && model::standalone(Path::new(&g.path)))
            .collect();
        let rows: Vec<_> = self
            .p
            .get("RecentLaunches")
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| {
                let id = v["Id"].as_str()?;
                let g = eligible.iter().find(|g| g.id == id)?;
                let when = v["Started"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|date| date.with_timezone(&Local).format("%d.%m %H:%M").to_string());
                Some((g.clone(), when))
            })
            .take(3)
            .collect();
        if rows.is_empty() {
            eligible.into_iter().take(3).map(|g| (g, None)).collect()
        } else {
            rows
        }
    }

    fn sync_menu(&self) {
        let Some(panel) = self.popup.as_ref().filter(|p| p.ready) else {
            return;
        };
        let history = &self.p.get("FriendHistory")[self.friend_account.to_string()];
        let data = json!({
            "type":"menu","language":self.lang(),"light":self.p.flag("LightTheme"),"radius":self.p.number("MenuRadius"),
            "tint":self.p.number("MenuGlassTint"),"glass":self.p.flag("MenuGlass"),
            "settings":self.t("settings"),"manage":self.t("manageExes"),
            "online":self.t("menuOnline"),"offline":self.t("menuOffline"),"avatars":self.p.flag("FriendAvatars"),
            "recentTitle":self.t("MenuRecent"),"noticeTitle":self.t("notifications"),"friendsTitle":self.t("friendsTab"),
            "recentEmpty":self.t("menuAddExeHelp"),"noticeEmpty":self.t("menuNoNotices"),
            "friendsEmpty":self.t(if self.friends.available {"menuNoFriends"} else if self.friends_busy.is_some() {"friendsLoading"} else {"friendsUnavailable"}),
            "hide":self.t("menuHide"),
            "recent":self.p.flag("MenuRecent").then(|| self.recent_rows().into_iter().map(|(g,time)| {
                let icon = native::load_art(Path::new(&g.path)).and_then(|i| glass::png(&i));
                let open=self.status.windows.contains_key(&g.id);
                json!({"id":g.id,"name":g.name,"detail":if open {Some(self.t("recentOpen").to_owned())} else {time},"icon":icon,
                    "action":self.t(if open {"recentSwitchTo"} else {"recentLaunch"})})
            }).collect::<Vec<_>>()),
            "offers":(self.p.flag("MenuNotices") && self.p.flag("Discounts")).then(|| self.menu_offers()),
            "people":self.p.flag("MenuFriends").then(|| self.friends.people.iter().map(|f| {
                let last = &history[f.id.to_string()];
                let game = last["Game"].as_str().or_else(|| (!f.game.is_empty()).then_some(f.game.as_str())).unwrap_or_else(||self.t("menuUnknownGame"));
                let when = last["Observed"].as_str().and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|date| date.with_timezone(&Local).format("%d.%m %H:%M").to_string());
                json!({"name":f.name,"online":f.online,"playing":f.game,"game":game,"when":when,"avatar":f.avatar.as_ref().filter(|_| self.p.flag("FriendAvatars"))})
            }).collect::<Vec<_>>())
        });
        panel.send(data);
    }

    fn menu_message(&mut self, v: &Value) {
        match v["action"].as_str().unwrap_or_default() {
            "ready" => {
                if let Some(panel) = &mut self.popup {
                    panel.ready = true;
                }
                self.sync_menu();
            }
            "measure" => {
                let Some(amount) = v["id"].as_u64().filter(|n| (32..=850).contains(n)) else {
                    return;
                };
                let display = self.display();
                let (Some(panel), Some(widget), Some(display)) =
                    (self.popup.as_mut(), &self.widget, display)
                else {
                    return;
                };
                let width = (374. * display.dpi).round() as i32;
                let h = ((amount as f64 * display.dpi).round() as i32)
                    .min(display.work.bottom - display.work.top);
                let b = widget.bounds;
                let gap = (8. * display.dpi).round() as i32;
                let y = if b.top >= display.work.top + h + gap {
                    b.top - h - gap
                } else {
                    b.bottom + gap
                };
                let y = y.clamp(display.work.top, display.work.bottom - h);
                let x = ((b.left + b.right - width) / 2).clamp(
                    display.work.left,
                    (display.work.right - width).max(display.work.left),
                );
                native::place_popup(
                    panel.hwnd(),
                    RECT {
                        left: x,
                        top: y,
                        right: x + width,
                        bottom: y + h,
                    },
                    self.p.number("MenuRadius"),
                );
                if !panel.shown {
                    panel.shown = true;
                    panel.next_frame = Instant::now();
                    panel.window.set_visible(true);
                    unsafe {
                        SetForegroundWindow(panel.hwnd());
                    }
                    panel.send(json!({"type":"reveal","animate":native::client_area_animation()}));
                }
            }
            "close" => {
                self.popup.take();
            }
            "settings" | "manage" => {
                self.popup.take();
                native::send(if v["action"] == "manage" {
                    Event::OpenLibrary
                } else {
                    Event::OpenSettings
                });
            }
            "launch" => {
                let Some(id) = v["id"].as_str() else {
                    return;
                };
                let Some((game, _)) = self.recent_rows().into_iter().find(|(g, _)| g.id == id)
                else {
                    return;
                };
                self.popup.take();
                if self.status.windows.get(id).copied().is_some_and(os::focus) {
                    return;
                }
                if let Some(path) = model::resolve_launch_path(Path::new(&game.path)) {
                    let mut command = std::process::Command::new(&path);
                    if let Some(dir) = path.parent() {
                        command.current_dir(dir);
                    }
                    if command.spawn().is_ok() {
                        self.remember_recent(vec![Recent {
                            id: game.id,
                            started: Utc::now(),
                        }]);
                        return;
                    }
                }
                native::error(self.t("menuLaunchError"));
            }
            "offer" | "hideOffer" => {
                let Some(id) = v["id"].as_u64().and_then(|id| u32::try_from(id).ok()) else {
                    return;
                };
                let mut offers = self.menu_offers();
                if !offers.iter().any(|o| o.id == id) {
                    return;
                }
                if v["action"] == "offer" {
                    self.popup.take();
                    self.open_offer(id);
                } else {
                    offers.retain(|o| o.id != id);
                    self.p.set("MenuOffers", json!(offers));
                    if self.offer_card.as_ref().is_some_and(|(o, _)| o.id == id) {
                        self.offer_card = None;
                    }
                    self.changed();
                }
            }
            _ => {}
        }
    }

    fn open_offer(&self, id: u32) {
        if !os::steam_app(id) {
            native::error(self.t("storeError"));
        }
    }

    fn tray(&mut self, target: &EventLoopWindowTarget<Event>) -> bool {
        let item = |id, key| MenuItem::Command(id, self.t(key).to_owned(), true, false);
        let mut sales: Vec<_> = if self.offer_message.is_empty() {
            self.offers
                .iter()
                .take(12)
                .enumerate()
                .map(|(i, o)| {
                    MenuItem::Command(
                        100 + i as u32,
                        format!("{} · −{}%", o.name, o.discount),
                        true,
                        false,
                    )
                })
                .collect()
        } else {
            vec![MenuItem::Command(
                0,
                self.t(self.offer_message).to_owned(),
                false,
                false,
            )]
        };
        sales.extend([
            MenuItem::Separator,
            item(5, "checkNow"),
            item(6, "allSales"),
        ]);
        let label = if self.status.active {
            "connected"
        } else if self.status.root.is_some() {
            "closed"
        } else {
            "notFound"
        };
        let items = [
            MenuItem::Command(0, self.t(label).to_owned(), false, false),
            MenuItem::Separator,
            item(1, "settings"),
            item(
                2,
                if self.p.flag("Widget") {
                    "hideWidget"
                } else {
                    "Widget"
                },
            ),
            item(
                3,
                if self.p.flag("WidgetFrozen") {
                    "unfreezeWidget"
                } else {
                    "freezeWidget"
                },
            ),
            item(4, "openDownloads"),
            MenuItem::Submenu(self.t("sales").to_owned(), sales),
        ];
        match self.shell.menu(&items, self.p.flag("LightTheme")) {
            1 => self.open_settings(target),
            id @ (2 | 3) => {
                let key = if id == 2 { "Widget" } else { "WidgetFrozen" };
                self.p.set(key, json!(!self.p.flag(key)));
                self.changed();
                self.sync();
            }
            id @ (4 | 6) => {
                if !os::shell_open(if id == 4 {
                    "steam://open/downloads"
                } else {
                    "steam://url/StoreSpecials"
                }) {
                    native::error(self.t("storeError"));
                }
            }
            5 => self.sales(false),
            id @ 100..=111 => {
                if let Some(o) = self.offers.get((id - 100) as usize) {
                    self.open_offer(o.id);
                }
            }
            _ => {}
        }
        false
    }

    fn display(&self) -> Option<native::Display> {
        let ds = native::displays();
        ds.iter()
            .find(|d| d.name == self.p.text("MonitorName"))
            .or_else(|| ds.iter().find(|d| d.primary))
            .or_else(|| ds.first())
            .cloned()
    }

    fn refresh_widget(&mut self) {
        let now = Instant::now();
        self.widget_at = now + Duration::from_millis(240);
        if !self.p.flag("Widget") {
            self.widget.take();
            return;
        }
        if self
            .offer_card
            .as_ref()
            .is_some_and(|(_, until)| now >= *until)
        {
            self.offer_card = None;
        }
        let Some(display) = self.display() else {
            self.widget.take();
            return;
        };
        let (hwnd, bar) = native::taskbar(&display);
        if hwnd != 0 && !self.scan_busy && now >= self.scan_at {
            self.scan_at = now + Duration::from_millis(750);
            self.scan_busy = true;
            let tx = self.proxy.clone();
            thread::spawn(move || {
                let _ = tx.send_event(Event::Scan(hwnd, native::scan_taskbar(hwnd, bar)));
            });
        }
        let draw = self.draw(display.dpi);
        let occupied = self
            .scan
            .as_ref()
            .filter(|(id, _)| *id == hwnd)
            .map_or(&[][..], |(_, ranges)| ranges.as_slice());
        if self.widget.is_none() {
            self.widget = Widget::new();
        }
        if let Some(w) = &mut self.widget {
            w.place(&self.p, bar, occupied, self.drag_position, draw);
            if w.animating {
                self.widget_at = now + Duration::from_millis(16);
            }
        }
    }

    fn draw(&mut self, dpi: f64) -> Draw {
        let p = &self.p;
        let text = |key| self.t(key).to_owned();
        let icon = p.flag("Icon");
        let name = p.flag("Name");
        let rows = &mut Vec::new();
        let mut art_paths = Vec::new();
        if p.flag("Downloads")
            && let Some(d) = self
                .status
                .download
                .as_ref()
                .filter(|d| !p.ignored(d.game.id))
        {
            let mut stats = Vec::new();
            if p.flag("Percent")
                && let Some(percent) = d.percent
            {
                stats.push(format!(
                    "{}{percent:.1}%",
                    if d.estimated { "≈" } else { "" }
                ));
            }
            if p.flag("Speed")
                && let Some(rate) = d.speed
            {
                stats.push(speed(rate, p.flag("Bits")));
            }
            if p.flag("RemainingTime")
                && let Some(remaining) = d.remaining
            {
                stats.push(format!("≈{}", duration(remaining, self.lang())));
            }
            rows.push(Row {
                title: if name {
                    d.game.name.clone()
                } else {
                    String::new()
                },
                state: text(if d.installing {
                    "installing"
                } else {
                    "download"
                }),
                detail: stats.join("  "),
                progress: d.percent.filter(|_| p.flag("Bar")),
                glyph: if icon { "↓" } else { "" },
                art: None,
                badge: "↓",
                ..Row::default()
            });
            art_paths.push(d.game.art.clone());
        }
        if p.flag("Games")
            && let Some(g) = self.status.game.as_ref().filter(|g| !p.ignored(g.id))
        {
            let mut metrics = Vec::new();
            if p.flag("MonitorSystem")
                && let Some(load) = self.status.load.as_ref()
            {
                if p.flag("MonitorCpu")
                    && let Some(cpu) = load.cpu
                {
                    metrics.push(format!("CPU {cpu}%"));
                }
                if p.flag("MonitorRam")
                    && let Some(ram) = load.memory_percent
                {
                    metrics.push(if p.flag("RamGiB") {
                        format!("RAM {:.1} GiB", load.memory_gib.unwrap_or_default())
                    } else {
                        format!("RAM {ram}%")
                    });
                }
            }
            rows.push(Row {
                title: if name { g.name.clone() } else { String::new() },
                state: text("inGame"),
                detail: metrics.join("  "),
                glyph: if icon { "▶" } else { "" },
                badge: "▶",
                ..Row::default()
            });
            art_paths.push(g.art.clone());
        }
        let mut sale_art = None;
        if rows.is_empty()
            && p.flag("Discounts")
            && p.flag("DiscountCard")
            && let Some((o, _)) = self.offer_card.as_ref().filter(|(o, _)| !p.ignored(o.id))
        {
            sale_art = o
                .image
                .clone()
                .filter(|_| icon)
                .map(|url| (rows.len(), url));
            rows.push(Row {
                title: if name { o.name.clone() } else { String::new() },
                state: text("sale"),
                detail: format!("−{}%", o.discount),
                glyph: if icon { "%" } else { "" },
                badge: "%",
                ..Row::default()
            });
            art_paths.push(None);
        }
        let mut avatar = None;
        if rows.len() < 2 && p.flag("Friends") && !self.friends.playing.is_empty() {
            let friends = &self.friends.playing;
            let seconds = Utc::now().timestamp().max(0) as usize;
            let index = (seconds / (p.number("FriendCycle").max(3.) as usize)) % friends.len();
            let row = &friends[index];
            let detail = if friends.len() > 1 {
                format!("+{}", friends.len() - 1)
            } else {
                String::new()
            };
            avatar = row
                .avatar
                .clone()
                .filter(|_| p.flag("FriendAvatars"))
                .map(|url| (rows.len(), url));
            rows.push(Row {
                title: format!("{} · {}", row.name, row.game),
                detail,
                glyph: "●",
                friend: true,
                ..Row::default()
            });
            art_paths.push(None);
        }
        if rows.is_empty() {
            rows.push(Row {
                state: text(if self.status.root.is_none() {
                    "notFound"
                } else if self.status.active {
                    "online"
                } else {
                    "closed"
                }),
                badge: "•",
                simple: true,
                ..Row::default()
            });
        }
        if rows.len() < 2
            && self.status.download.is_none()
            && p.flag("Network")
            && let Some(rate) = self.status.network
        {
            rows.push(Row {
                state: text("network"),
                detail: speed(rate, p.flag("Bits")),
                simple: true,
                ..Row::default()
            });
        }
        let mut d = colors(p, dpi);
        if icon {
            for (row, path) in rows.iter_mut().zip(art_paths) {
                if let Some(path) = path {
                    if self.art_cache.len() >= 100 && !self.art_cache.contains_key(&path) {
                        self.art_cache.clear();
                    }
                    row.art = self
                        .art_cache
                        .entry(path.clone())
                        .or_insert_with(|| native::load_art(&path).map(Arc::new))
                        .clone();
                }
            }
        }
        if let Some((row, url)) = avatar {
            rows[row].art = self.remote_image(url, true);
        }
        if let Some((row, url)) = sale_art {
            rows[row].art = self.remote_image(url, false);
        }
        d.rows = std::mem::take(rows);
        d
    }

    fn remote_image(&mut self, url: String, avatar: bool) -> Option<Arc<image::RgbaImage>> {
        if let Some(image) = self.remote_art.get(&url) {
            return image.clone();
        }
        if self.verify {
            return None;
        }
        if self.remote_art.len() >= 100 {
            self.remote_art.clear();
        }
        self.remote_art.insert(url.clone(), None);
        let tx = self.proxy.clone();
        thread::spawn(move || {
            let art = online::image(&url, avatar);
            let _ = tx.send_event(Event::Artwork(url, art));
        });
        None
    }

    fn frames(&mut self, now: Instant) {
        for (kind, key, prefix) in [
            (PanelKind::Settings, "SettingsGlass", "Settings"),
            (PanelKind::WidgetMenu, "MenuGlass", "Menu"),
        ] {
            let enabled = self.p.flag(key);
            let p = self.p.clone();
            let proxy = self.proxy.clone();
            let Some(panel) = self
                .panel_mut(kind)
                .filter(|panel| panel.ready && (kind != PanelKind::WidgetMenu || panel.shown))
            else {
                continue;
            };
            let handle = panel.hwnd();
            if !enabled {
                if panel.frame.take().is_some() {
                    panel.send(json!({"type":"glass","frame":null}));
                    panel.glass_cache = Arc::new(Mutex::new(glass::Cache::default()));
                }
                glass::exclude(handle, false);
                continue;
            }
            if panel.frame_in_flight || now < panel.next_frame || panel.window.is_minimized() {
                continue;
            }
            let Some(bounds) = glass::bounds(handle) else {
                continue;
            };
            if !glass::exclude(handle, true) {
                continue;
            }
            let id = panel.id;
            let dpi = native::dpi(handle);
            let params = glass::Options::settings(&p, prefix, dpi);
            let cache = panel.glass_cache.clone();
            panel.frame_in_flight = true;
            thread::spawn(move || {
                let frame = cache
                    .lock()
                    .ok()
                    .and_then(|mut cache| cache.frame(bounds, params));
                let _ = proxy.send_event(Event::Glass(kind, id, frame));
            });
        }
    }
}

fn colors(p: &Preferences, dpi: f64) -> Draw {
    let light = p.flag("LightTheme");
    let color = |key, dark, bright| {
        let saved = p.text(key);
        native::rgb(
            if light && saved.eq_ignore_ascii_case(dark) {
                bright
            } else {
                saved
            },
            0x222222,
        )
    };
    let glass = p.flag("Glass");
    let gradient = p.flag("Gradient");
    let glass_bg = if light { "#F0F4FA" } else { "#151D2B" };
    let background = if glass && !gradient {
        native::rgb(glass_bg, 0x222222)
    } else {
        color("Background", "#101929", "#F7F9FD")
    };
    let accent = color("Accent", "#54DAFE", "#2453A6");
    Draw {
        background,
        end: if gradient {
            color("BackgroundEnd", "#1F3851", "#D9E7FB")
        } else {
            background
        },
        angle: p.number("Angle"),
        opacity: if glass {
            p.number("GlassTint").max(0.76)
        } else {
            p.number("Opacity")
        },
        radius: p.number("Radius").round() as i32,
        air: p.flag("AirMode"),
        glass,
        border: p.flag("WidgetBorder"),
        text: color("Text", "#F1F7FF", "#152033"),
        accent,
        accent_end: if gradient {
            color("AccentEnd", "#81A5FF", "#6049A0")
        } else {
            accent
        },
        dpi,
        ..Draw::default()
    }
}

#[cfg(test)]
#[test]
fn widget_glass_keeps_both_saved_gradient_colors() {
    let mut p = Preferences::default();
    p.set("Glass", json!(true));
    p.set("Gradient", json!(true));
    p.set("Background", json!("#ffffff"));
    p.set("BackgroundEnd", json!("#171e2a"));
    p.set("GlassTint", json!(0));
    let draw = colors(&p, 1.);
    assert_eq!(draw.background, native::rgb("#ffffff", 0));
    assert_eq!(draw.end, native::rgb("#171e2a", 0));
    assert_eq!(draw.opacity, 0.76);
}

fn speed(rate: f64, bits: bool) -> String {
    let (amount, unit) = if bits {
        (rate * 8. / 1_000_000., "Mbps")
    } else if rate >= 1_048_576. {
        (rate / 1_048_576., "MiB/s")
    } else {
        (rate / 1024., "KiB/s")
    };
    format!("{} {unit}", format!("{amount:.1}").trim_end_matches(".0"))
}

fn duration(seconds: f64, lang: &str) -> String {
    let minutes = (seconds / 60.).ceil() as u64;
    if minutes >= 1440 {
        format!(
            "{} {} {} {}",
            minutes / 1440,
            tr(lang, "days"),
            minutes / 60 % 24,
            tr(lang, "hours")
        )
    } else if minutes >= 60 {
        format!(
            "{} {} {} {}",
            minutes / 60,
            tr(lang, "hours"),
            minutes % 60,
            tr(lang, "minutes")
        )
    } else if seconds >= 60. {
        format!("{minutes} {}", tr(lang, "minutes"))
    } else {
        format!("{} {}", seconds.ceil().max(1.), tr(lang, "seconds"))
    }
}
