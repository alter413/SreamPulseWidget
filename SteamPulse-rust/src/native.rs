use crate::Event;
use serde::Serialize;
use std::{
    cell::{Cell, RefCell},
    collections::{LinkedList, VecDeque},
    mem::{size_of, zeroed},
    ops::Range,
    path::PathBuf,
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use steam_pulse::{layout, model::Preferences, windows_data::wide};
use tao::event_loop::EventLoopProxy;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{
        Dwm::{
            DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
            DwmSetWindowAttribute,
        },
        Gdi::*,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    },
    UI::{
        Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
        Controls::{
            DRAWITEMSTRUCT, Dialogs::*, MEASUREITEMSTRUCT, ODS_DISABLED, ODS_GRAYED, ODS_SELECTED,
            ODT_MENU,
        },
        HiDpi::{GetDpiForMonitor, GetDpiForWindow, MDT_EFFECTIVE_DPI},
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON},
        Shell::*,
        WindowsAndMessaging::*,
    },
};

const TRAY_MESSAGE: u32 = WM_APP + 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
thread_local! {
    static PROXY: RefCell<Option<EventLoopProxy<Event>>> = const { RefCell::new(None) };
    static DRAG_ALLOWED: Cell<bool> = const { Cell::new(false) };
    static PRESSED: RefCell<Option<Press>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: RefCell<u32> = const { RefCell::new(0) };
}

pub fn send(event: Event) {
    PROXY.with(|proxy| {
        if let Some(proxy) = proxy.borrow().as_ref() {
            let _ = proxy.send_event(event);
        }
    });
}

pub struct Shell {
    pub owner: HWND,
    icon: HICON,
    tray: bool,
    tray_retry: Cell<Option<Instant>>,
}

const ICON_ID: *const u16 = 101usize as *const u16;

fn application_icon(small: bool) -> HICON {
    unsafe {
        let module = GetModuleHandleW(null());
        let (x, y) = if small {
            (SM_CXSMICON, SM_CYSMICON)
        } else {
            (SM_CXICON, SM_CYICON)
        };
        let icon = LoadImageW(
            module,
            ICON_ID,
            IMAGE_ICON,
            GetSystemMetrics(x),
            GetSystemMetrics(y),
            LR_SHARED,
        );
        if icon.is_null() {
            LoadIconW(null_mut(), IDI_APPLICATION)
        } else {
            icon
        }
    }
}

pub fn app_identity() {
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(wide("SteamPulse.Desktop").as_ptr());
    }
}

pub fn window_icons(hwnd: HWND) {
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETICON,
            ICON_BIG as usize,
            application_icon(false) as isize,
        );
        SendMessageW(
            hwnd,
            WM_SETICON,
            ICON_SMALL as usize,
            application_icon(true) as isize,
        );
    }
}

fn class() -> Vec<u16> {
    wide("SteamPulseRust.Native")
}

impl Shell {
    pub fn new(proxy: EventLoopProxy<Event>, tray: bool) -> std::io::Result<Self> {
        PROXY.with(|v| v.replace(Some(proxy)));
        let name = class();
        let module = unsafe { GetModuleHandleW(null()) };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc),
            hInstance: module,
            hIcon: application_icon(false),
            lpszClassName: name.as_ptr(),
            hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
            ..unsafe { zeroed() }
        };
        unsafe {
            RegisterClassW(&wc);
        }
        let owner = create(WS_EX_TOOLWINDOW, WS_POPUP);
        if owner.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let shell = Self {
            owner,
            icon: application_icon(true),
            tray,
            tray_retry: Cell::new(None),
        };
        TASKBAR_CREATED.with(|v| {
            v.replace(unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) })
        });
        shell.readd();
        Ok(shell)
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.owner,
            uID: 1,
            ..unsafe { zeroed() }
        }
    }

    pub fn readd(&self) {
        if !self.tray {
            return;
        }
        let mut d = self.data();
        d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
        d.hIcon = self.icon;
        d.uCallbackMessage = TRAY_MESSAGE;
        write_wide("Steam Pulse", &mut d.szTip);
        unsafe {
            let registered =
                Shell_NotifyIconW(NIM_ADD, &d) != 0 || Shell_NotifyIconW(NIM_MODIFY, &d) != 0;
            if registered {
                d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &d);
            }
            self.tray_retry
                .set((!registered).then(|| Instant::now() + Duration::from_secs(2)));
        }
    }

    pub fn maintain_tray(&self, now: Instant) {
        if self.tray_retry.get().is_some_and(|retry| now >= retry) {
            self.readd();
        }
    }

    pub fn notify(&self, title: &str, text: &str) {
        if !self.tray {
            return;
        }
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        write_wide(title, &mut data.szInfoTitle);
        write_wide(text, &mut data.szInfo);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    pub fn tooltip(&self, line: &str) {
        if !self.tray {
            return;
        }
        let mut d = self.data();
        d.uFlags = NIF_TIP | NIF_SHOWTIP;
        write_wide(line, &mut d.szTip);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }

    pub fn menu(&self, items: &[MenuItem], light: bool) -> u32 {
        // Owner-drawn rows keep the native keyboard navigation, popup tracking and submenus.
        unsafe fn fill(items: &[MenuItem], style: Option<&mut MenuStyle>) -> HMENU {
            let menu = unsafe { CreatePopupMenu() };
            if menu.is_null() {
                return menu;
            }
            let mut style = style;
            if let Some(style) = style.as_ref() {
                let info = MENUINFO {
                    cbSize: size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND,
                    hbrBack: style.background,
                    ..unsafe { zeroed() }
                };
                unsafe {
                    SetMenuInfo(menu, &info);
                }
            }
            for item in items {
                unsafe {
                    if let Some(style) = style.as_deref_mut() {
                        let (label, id, state, submenu, sep) = match item {
                            MenuItem::Separator => ("", 0, MFS_DISABLED, null_mut(), true),
                            MenuItem::Command(id, title, enabled, checked) => (
                                title.as_str(),
                                *id,
                                (if *enabled { 0 } else { MFS_DISABLED })
                                    | if *checked { MFS_CHECKED } else { 0 },
                                null_mut(),
                                false,
                            ),
                            MenuItem::Submenu(title, children) => {
                                (title.as_str(), 0, 0, fill(children, Some(style)), false)
                            }
                        };
                        let mut title = wide(label);
                        style.rows.push_back(MenuRow {
                            title: label.to_owned(),
                            sep,
                            arrow: !submenu.is_null(),
                            dpi: style.dpi,
                        });
                        let row = style.rows.back().unwrap() as *const MenuRow;
                        let mi = MENUITEMINFOW {
                            cbSize: size_of::<MENUITEMINFOW>() as u32,
                            fMask: MIIM_FTYPE
                                | MIIM_ID
                                | MIIM_STATE
                                | MIIM_DATA
                                | MIIM_STRING
                                | if submenu.is_null() { 0 } else { MIIM_SUBMENU },
                            fType: MFT_OWNERDRAW,
                            fState: state,
                            wID: id,
                            hSubMenu: submenu,
                            dwItemData: row as usize,
                            dwTypeData: title.as_mut_ptr(),
                            cch: title.len() as u32 - 1,
                            ..zeroed()
                        };
                        if InsertMenuItemW(menu, u32::MAX, 1, &mi) == 0 && !submenu.is_null() {
                            DestroyMenu(submenu);
                        }
                        continue;
                    }
                    match item {
                        MenuItem::Separator => {
                            AppendMenuW(menu, MF_SEPARATOR, 0, null());
                        }
                        MenuItem::Command(id, title, enabled, checked) => {
                            let flags = if *enabled { 0 } else { MF_GRAYED };
                            AppendMenuW(
                                menu,
                                flags | if *checked { MF_CHECKED } else { 0 },
                                *id as usize,
                                wide(title).as_ptr(),
                            );
                        }
                        MenuItem::Submenu(title, children) => {
                            let submenu = fill(children, None);
                            if !submenu.is_null()
                                && AppendMenuW(
                                    menu,
                                    MF_POPUP,
                                    submenu as usize,
                                    wide(title).as_ptr(),
                                ) == 0
                            {
                                DestroyMenu(submenu);
                            }
                        }
                    }
                }
            }
            menu
        }
        unsafe {
            let mut hc = HIGHCONTRASTW {
                cbSize: size_of::<HIGHCONTRASTW>() as u32,
                ..zeroed()
            };
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                hc.cbSize,
                (&mut hc as *mut HIGHCONTRASTW).cast(),
                0,
            );
            let p = cursor();
            let mut style = (!light && hc.dwFlags & HCF_HIGHCONTRASTON == 0).then(|| MenuStyle {
                background: CreateSolidBrush(rgb("#1E1F24", 0x242424)),
                rows: LinkedList::new(),
                dpi: displays()
                    .into_iter()
                    .find(|d| {
                        p.x >= d.bounds.left
                            && p.x < d.bounds.right
                            && p.y >= d.bounds.top
                            && p.y < d.bounds.bottom
                    })
                    .map_or(1., |d| d.dpi),
            });
            let menu = fill(items, style.as_mut());
            if menu.is_null() {
                return 0;
            }
            SetForegroundWindow(self.owner);
            let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY;
            let id = TrackPopupMenu(menu, flags, p.x, p.y, 0, self.owner, null());
            PostMessageW(self.owner, WM_NULL, 0, 0);
            DestroyMenu(menu);
            id as u32
        }
    }
}

// The menu stores pointers to the list nodes until the menu is destroyed.
struct MenuStyle {
    background: HBRUSH,
    rows: LinkedList<MenuRow>,
    dpi: f64,
}
impl Drop for MenuStyle {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.background);
        }
    }
}

struct MenuRow {
    title: String,
    sep: bool,
    arrow: bool,
    dpi: f64,
}

impl MenuRow {
    fn size(&self, n: f64) -> i32 {
        (n * self.dpi).round() as i32
    }
    unsafe fn font(&self) -> HFONT {
        unsafe {
            CreateFontW(
                -self.size(13.),
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                ANTIALIASED_QUALITY as u32,
                0,
                wide("Segoe UI").as_ptr(),
            )
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        unsafe {
            if self.tray {
                Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
            DestroyWindow(self.owner);
        }
    }
}

pub enum MenuItem {
    Separator,
    Command(u32, String, bool, bool),
    Submenu(String, Vec<MenuItem>),
}

fn write_wide(s: &str, out: &mut [u16]) {
    let mut at = 0;
    for ch in s.chars() {
        let mut temp = [0; 2];
        let units = ch.encode_utf16(&mut temp);
        if at + units.len() >= out.len() {
            break;
        }
        out[at..at + units.len()].copy_from_slice(units);
        at += units.len();
    }
    out[at] = 0;
}

fn create(ex: u32, style: u32) -> HWND {
    unsafe {
        CreateWindowExW(
            ex,
            class().as_ptr(),
            wide("Steam Pulse").as_ptr(),
            style,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    }
}

pub fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        GetCursorPos(&mut p);
    }
    p
}

fn point_in(p: POINT, r: RECT) -> bool {
    p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
}

fn dismiss_popup(p: POINT, popup: RECT, anchor: Option<RECT>, left: bool, other: bool) -> bool {
    !point_in(p, popup) && (other || left && !anchor.is_some_and(|r| point_in(p, r)))
}

pub fn outside_popup_press(hwnd: HWND, anchor: Option<RECT>) -> bool {
    let pressed = |button| unsafe { GetAsyncKeyState(i32::from(button)) } < 0;
    let left = pressed(VK_LBUTTON);
    let other = pressed(VK_MBUTTON) || pressed(VK_RBUTTON);
    if !left && !other {
        return false;
    }
    dismiss_popup(cursor(), window_rect(hwnd), anchor, left, other)
}

pub fn client_area_animation() -> bool {
    let mut enabled = 0;
    unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        ) != 0
            && enabled != 0
    }
}

#[cfg(test)]
#[test]
fn a_left_press_on_the_widget_leaves_closing_to_the_click_handler() {
    let popup = RECT {
        left: 10,
        top: 10,
        right: 110,
        bottom: 110,
    };
    let anchor = Some(RECT {
        left: 20,
        top: 120,
        right: 100,
        bottom: 150,
    });
    assert!(!dismiss_popup(
        POINT { x: 50, y: 130 },
        popup,
        anchor,
        true,
        false
    ));
    assert!(dismiss_popup(
        POINT { x: 50, y: 130 },
        popup,
        anchor,
        false,
        true
    ));
    assert!(dismiss_popup(
        POINT { x: 115, y: 130 },
        popup,
        anchor,
        true,
        false
    ));
    assert!(!dismiss_popup(
        POINT { x: 80, y: 50 },
        popup,
        anchor,
        true,
        false
    ));
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if TASKBAR_CREATED.with(|v| *v.borrow() != 0 && msg == *v.borrow()) {
            send(Event::Taskbar);
            return 0;
        }
        match msg {
            WM_MEASUREITEM if l != 0 => {
                let item = &mut *(l as *mut MEASUREITEMSTRUCT);
                if item.CtlType == ODT_MENU && item.itemData != 0 {
                    let row = &*(item.itemData as *const MenuRow);
                    let dc = GetDC(hwnd);
                    let font = row.font();
                    let old = SelectObject(dc, font);
                    let title = wide(&row.title);
                    let mut width = SIZE::default();
                    GetTextExtentPoint32W(dc, title.as_ptr(), title.len() as i32 - 1, &mut width);
                    SelectObject(dc, old);
                    DeleteObject(font);
                    ReleaseDC(hwnd, dc);
                    item.itemWidth = (width.cx + row.size(if row.arrow { 52. } else { 35. }))
                        .max(row.size(130.)) as u32;
                    item.itemHeight = row.size(if row.sep { 8. } else { 26. }) as u32;
                    return 1;
                }
                DefWindowProcW(hwnd, msg, w, l)
            }
            WM_DRAWITEM if l != 0 => {
                let item = &*(l as *const DRAWITEMSTRUCT);
                if item.CtlType == ODT_MENU && item.itemData != 0 {
                    let row = &*(item.itemData as *const MenuRow);
                    let dc = item.hDC;
                    let r = item.rcItem;
                    let selected = item.itemState & ODS_SELECTED != 0;
                    let disabled = item.itemState & (ODS_DISABLED | ODS_GRAYED) != 0;
                    fill(
                        dc,
                        r,
                        rgb(
                            if selected && !disabled {
                                "#353841"
                            } else {
                                "#1E1F24"
                            },
                            0x242424,
                        ),
                    );
                    if row.sep {
                        let y = (r.top + r.bottom) / 2;
                        fill(
                            dc,
                            RECT {
                                left: r.left + row.size(7.),
                                right: r.right - row.size(7.),
                                top: y,
                                bottom: y + 1,
                            },
                            rgb("#42444B", 0x404040),
                        );
                    } else {
                        let font = row.font();
                        let old = SelectObject(dc, font);
                        SetBkMode(dc, TRANSPARENT as i32);
                        let color = rgb(if disabled { "#888C97" } else { "#F3F4F8" }, 0xeeeeee);
                        text(
                            dc,
                            RECT {
                                left: r.left + row.size(10.),
                                right: r.right - row.size(17.),
                                ..r
                            },
                            &row.title,
                            color,
                            DT_LEFT,
                        );
                        if row.arrow {
                            text(
                                dc,
                                RECT {
                                    left: r.right - row.size(17.),
                                    right: r.right - row.size(5.),
                                    ..r
                                },
                                "›",
                                color,
                                DT_CENTER,
                            );
                        }
                        SelectObject(dc, old);
                        DeleteObject(font);
                    }
                    return 1;
                }
                DefWindowProcW(hwnd, msg, w, l)
            }
            TRAY_MESSAGE => {
                match l as u32 & 0xffff {
                    WM_LBUTTONDBLCLK | NIN_KEYSELECT => send(Event::OpenSettings),
                    WM_CONTEXTMENU => send(Event::Tray),
                    WM_RBUTTONUP if (l as u32) >> 16 == 0 => send(Event::Tray),
                    NIN_BALLOONUSERCLICK => send(Event::Notification),
                    _ => {}
                }
                0
            }
            WM_PAINT => {
                ValidateRect(hwnd, null());
                0
            }
            WM_ERASEBKGND => 1,
            WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
            WM_RBUTTONUP => {
                send(Event::OpenSettings);
                0
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let bounds = window_rect(hwnd);
                let center = (f64::from(bounds.left) + f64::from(bounds.right)) / 2.;
                PRESSED.with(|v| {
                    v.replace(Some(Press {
                        start: cursor(),
                        center,
                        dragged: false,
                    }))
                });
                SetTimer(hwnd, 1, 16, None);
                0
            }
            WM_MOUSEMOVE => {
                track_press(hwnd, false);
                0
            }
            WM_TIMER if w == 1 => {
                if PRESSED.with(|v| v.borrow().is_some()) {
                    track_press(hwnd, GetAsyncKeyState(i32::from(VK_LBUTTON)) >= 0);
                }
                0
            }
            WM_LBUTTONUP => {
                track_press(hwnd, true);
                0
            }
            WM_CLOSE => 0,
            WM_SYSCOMMAND if matches!(w & 0xFFF0, 0xF020 | 0xF030 | 0xF060) => 0,
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

struct Press {
    start: POINT,
    center: f64,
    dragged: bool,
}

fn track_press(hwnd: HWND, finished: bool) {
    let pointer = cursor();
    let event = PRESSED.with(|cell| {
        let mut borrowed = cell.borrow_mut();
        let press = borrowed.as_mut()?;
        let dx = pointer.x - press.start.x;
        if DRAG_ALLOWED.with(Cell::get) && (press.dragged || dx.abs() >= 3) {
            press.dragged = true;
        }
        let event = if press.dragged {
            Some(Event::Drag {
                center: press.center + f64::from(dx),
                commit: finished,
            })
        } else if finished && dx.abs() < 3 && (pointer.y - press.start.y).abs() < 4 {
            Some(Event::WidgetMenu)
        } else {
            None
        };
        if finished {
            *borrowed = None;
        }
        event
    });
    if finished {
        unsafe {
            KillTimer(hwnd, 1);
        }
    }
    if let Some(event) = event {
        send(event);
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Display {
    pub name: String,
    pub primary: bool,
    pub width: i32,
    pub height: i32,
    #[serde(skip)]
    pub bounds: RECT,
    #[serde(skip)]
    pub work: RECT,
    #[serde(skip)]
    pub dpi: f64,
}

pub fn displays() -> Vec<Display> {
    unsafe extern "system" fn gather(handle: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        unsafe {
            let mut i: MONITORINFOEXW = zeroed();
            i.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            if GetMonitorInfoW(handle, &mut i.monitorInfo) != 0 {
                let r = i.monitorInfo.rcMonitor;
                let out = &mut *(data as *mut Vec<Display>);
                let name = String::from_utf16_lossy(
                    i.szDevice.split(|n| *n == 0).next().unwrap_or_default(),
                );
                let (mut x, mut y) = (96, 96);
                let _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut x, &mut y);
                out.push(Display {
                    name,
                    width: r.right - r.left,
                    height: r.bottom - r.top,
                    primary: i.monitorInfo.dwFlags & 1 != 0,
                    bounds: r,
                    work: i.monitorInfo.rcWork,
                    dpi: x as f64 / 96.,
                });
            }
        }
        1
    }
    let mut output = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(gather),
            &mut output as *mut _ as LPARAM,
        );
    }
    output
}

pub fn taskbar(d: &Display) -> (isize, RECT) {
    unsafe {
        let class = wide(if d.primary {
            "Shell_TrayWnd"
        } else {
            "Shell_SecondaryTrayWnd"
        });
        let mut hwnd = null_mut();
        loop {
            hwnd = FindWindowExW(null_mut(), hwnd, class.as_ptr(), null());
            if hwnd.is_null() {
                break;
            }
            let mut r: RECT = zeroed();
            if IsWindowVisible(hwnd) != 0
                && GetWindowRect(hwnd, &mut r) != 0
                && r.right > d.bounds.left
                && r.left < d.bounds.right
                && r.bottom > d.bounds.top
                && r.top < d.bounds.bottom
                && r.bottom - r.top > 4
                && r.right - r.left > 4
            {
                return (hwnd as isize, r);
            }
        }
    }
    let mut bar = d.bounds;
    let height = (42. * d.dpi).round() as i32;
    if d.work.top > d.bounds.top {
        bar.bottom = d.work.top.max(d.bounds.top + height);
    } else {
        bar.top = (d.bounds.bottom - height).min(d.work.bottom);
    }
    (0, bar)
}

pub fn scan_taskbar(hwnd: isize, bounds: RECT) -> Vec<Range<i32>> {
    use windows::Win32::{
        Foundation::HWND as Hwnd,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, UIA_ButtonControlTypeId, UIA_MenuItemControlTypeId,
            UIA_TabItemControlTypeId,
        },
    };
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return Vec::new();
        }
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let Ok(ui): windows::core::Result<IUIAutomation> =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        else {
            return Vec::new();
        };
        let Ok(root) = ui.ElementFromHandle(Hwnd(hwnd as _)) else {
            return Vec::new();
        };
        let Ok(walk) = ui.ControlViewWalker() else {
            return Vec::new();
        };
        let mut hits = Vec::new();
        let mut queue = VecDeque::from([(root, 0)]);
        for _ in 0..260 {
            let Some((item, depth)) = queue.pop_front() else {
                break;
            };
            let kind = item.CurrentControlType();
            if kind.is_ok_and(|t| {
                [
                    UIA_ButtonControlTypeId,
                    UIA_MenuItemControlTypeId,
                    UIA_TabItemControlTypeId,
                ]
                .contains(&t)
            }) {
                if let Ok(r) = item.CurrentBoundingRectangle()
                    && r.right > r.left
                    && r.bottom > bounds.top
                    && r.top < bounds.bottom
                {
                    hits.push(r.left..r.right);
                }
            } else if depth < 6
                && let Ok(mut child) = walk.GetFirstChildElement(&item)
            {
                while queue.len() < 260 {
                    queue.push_back((child.clone(), depth + 1));
                    match walk.GetNextSiblingElement(&child) {
                        Ok(next) => child = next,
                        Err(_) => break,
                    }
                }
            }
        }
        if !hits.is_empty() {
            let tray = FindWindowExW(
                hwnd as HWND,
                null_mut(),
                wide("TrayNotifyWnd").as_ptr(),
                null(),
            );
            let mut r: RECT = zeroed();
            if !tray.is_null() && GetWindowRect(tray, &mut r) != 0 {
                hits.push(r.left..r.right);
            }
        }
        hits
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct Row {
    pub title: String,
    pub state: String,
    pub detail: String,
    pub progress: Option<f64>,
    pub art: Option<std::sync::Arc<image::RgbaImage>>,
    pub glyph: &'static str,
    pub badge: &'static str,
    pub friend: bool,
    pub simple: bool,
}

#[derive(Clone, Default, PartialEq)]
pub struct Draw {
    pub rows: Vec<Row>,
    pub radius: i32,
    pub opacity: f64,
    pub air: bool,
    pub glass: bool,
    pub border: bool,
    pub background: u32,
    pub end: u32,
    pub angle: f64,
    pub text: u32,
    pub accent: u32,
    pub accent_end: u32,
    pub badge: bool,
    pub dpi: f64,
}

pub fn rgb(s: &str, fallback: u32) -> u32 {
    s.strip_prefix('#')
        .and_then(|text| u32::from_str_radix(text, 16).ok())
        .filter(|n| *n <= 0xffffff)
        .map_or(fallback, swap)
}

fn swap(n: u32) -> u32 {
    ((n & 255) << 16) | (n & 0xff00) | ((n >> 16) & 255)
}

pub struct Widget {
    pub hwnd: HWND,
    pub bounds: RECT,
    render: crate::widget_render::Renderer,
    last_draw: Option<Draw>,
    remembered_center: Option<f64>,
    lane: Option<Range<i32>>,
    shown: Option<(f64, f64)>,
    pub animating: bool,
}

impl Widget {
    pub fn new() -> Option<Self> {
        let render = crate::widget_render::Renderer::new().ok()?;
        let hwnd = create(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            WS_POPUP | WS_CLIPSIBLINGS,
        );
        if hwnd.is_null() {
            return None;
        }
        unsafe {
            for attr in [3u32, 12u32] {
                DwmSetWindowAttribute(hwnd, attr, (&1i32 as *const i32).cast(), 4);
            }
        }
        Some(Self {
            hwnd,
            bounds: RECT::default(),
            render,
            last_draw: None,
            remembered_center: None,
            lane: None,
            shown: None,
            animating: false,
        })
    }

    pub fn place(
        &mut self,
        p: &Preferences,
        bar: RECT,
        occupied: &[Range<i32>],
        drag: Option<f64>,
        mut draw: Draw,
    ) {
        let scale = draw.dpi;
        let unit = |n: f64| (n * scale).round() as i32;
        let vertical = bar.bottom - bar.top > bar.right - bar.left;
        let desired = unit(p.number("Width"));
        let percent = drag.or_else(|| p.get("HorizontalPosition").as_f64());
        let slot = (!vertical && !occupied.is_empty())
            .then(|| {
                layout::pick(
                    bar.left..bar.right,
                    occupied,
                    layout::Placement {
                        width: desired,
                        minimum: desired.min(unit(176.)),
                        badge: unit(36.),
                        gutter: unit(5.),
                        align: p.number("Placement") as i32,
                        offset: unit(p.number("Offset")),
                        percent,
                    },
                )
            })
            .flatten();
        let fallback = slot.is_none();
        let slot = slot.unwrap_or_else(|| {
            let width = unit(36.).clamp(1, (bar.right - bar.left).max(1));
            let center = percent
                .map(|percent| {
                    f64::from(bar.left) + f64::from(bar.right - bar.left) * percent / 100.
                })
                .or(self.remembered_center)
                .unwrap_or(f64::from(bar.left) + f64::from(bar.right - bar.left) * 0.79);
            let left = (center - f64::from(width) / 2.).round() as i32;
            let left = left.clamp(bar.left, (bar.right - width).max(bar.left));
            layout::Slot {
                bounds: left..left + width,
                safe: left..left + width,
            }
        });
        let target_width = f64::from(slot.bounds.end - slot.bounds.start);
        let target_center = f64::from(slot.bounds.start) + target_width / 2.;
        if !fallback {
            self.remembered_center = Some(target_center);
        }
        let animate = client_area_animation();
        let (mut center, mut width) = self.shown.unwrap_or((target_center, target_width));
        if !animate || self.lane.as_ref() != Some(&slot.safe) {
            center = target_center;
            width = if animate && self.shown.is_some() && !fallback {
                (target_width * 0.76).max(f64::from(unit(176.)).min(target_width))
            } else {
                target_width
            };
        } else {
            center += (target_center - center) * 0.36;
            width += (target_width - width) * 0.26;
        }
        self.animating =
            (center - target_center).abs() >= 0.65 || (width - target_width).abs() >= 0.65;
        if !self.animating {
            center = target_center;
            width = target_width;
        }
        self.lane = Some(slot.safe.clone());
        self.shown = Some((center, width));
        let width = (width.round() as i32).clamp(1, slot.safe.end - slot.safe.start);
        let x = ((center - f64::from(width) / 2.).round() as i32)
            .clamp(slot.safe.start, slot.safe.end - width);
        let bar_height = bar.bottom - bar.top;
        draw.badge = fallback || width < unit(150.) || (!vertical && bar_height < unit(45.));
        let height = if vertical {
            unit(34.)
        } else {
            let h = if draw.badge {
                26.
            } else {
                let dense = draw.rows.len() > 1;
                let rows = draw
                    .rows
                    .iter()
                    .map(|r| {
                        if r.simple || r.friend {
                            14.
                        } else if dense {
                            if r.progress.is_some() { 20. } else { 16. }
                        } else {
                            23.
                        }
                    })
                    .sum::<f64>();
                rows + draw.rows.len().saturating_sub(1) as f64 * 2.
                    + 6.
                    + if draw.border && !draw.air { 2. } else { 0. }
            };
            unit(h).clamp(1, bar_height.max(1))
        };
        let y = if vertical {
            bar.bottom - height - unit(8.)
        } else {
            bar.top + (bar_height - height) / 2
        };
        let click = p.flag("ClickThrough") || fallback;
        DRAG_ALLOWED.with(|v| v.set(!click && !p.flag("WidgetFrozen")));
        let bounds = RECT {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        };
        let redraw = self.last_draw.as_ref() != Some(&draw);
        unsafe {
            let ex = GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE);
            let next = if click {
                ex | WS_EX_TRANSPARENT as isize
            } else {
                ex & !(WS_EX_TRANSPARENT as isize)
            };
            if ex != next {
                SetWindowLongPtrW(self.hwnd, GWL_EXSTYLE, next);
            }
            let moved = (bounds.left, bounds.top, bounds.right, bounds.bottom)
                != (
                    self.bounds.left,
                    self.bounds.top,
                    self.bounds.right,
                    self.bounds.bottom,
                );
            let update = redraw || moved;
            if update && !self.render.present(self.hwnd, bounds, &draw, redraw) {
                self.last_draw = None;
                return;
            }
            if update || !self.animating {
                SetWindowPos(
                    self.hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
        self.bounds = bounds;
        self.last_draw = Some(draw);
    }
}

impl Drop for Widget {
    fn drop(&mut self) {
        PRESSED.with(|v| v.replace(None));
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &rect, brush);
        DeleteObject(brush);
    }
}

unsafe fn text(dc: HDC, mut r: RECT, s: &str, color: u32, align: u32) {
    if s.is_empty() || r.right <= r.left {
        return;
    }
    let chars: Vec<_> = s.encode_utf16().collect();
    unsafe {
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            chars.as_ptr(),
            chars.len() as i32,
            &mut r,
            align | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

pub fn load_art(path: &std::path::Path) -> Option<image::RgbaImage> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
    {
        let mut icon = null_mut();
        unsafe {
            if SHDefExtractIconW(wide(path).as_ptr(), 0, 0, &mut icon, null_mut(), 48) != 0
                || icon.is_null()
            {
                return None;
            }
        }
        let pixels = capture_icon(icon);
        unsafe {
            DestroyIcon(icon);
        }
        return pixels;
    }
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if w > 4096 || h > 4096 {
        return None;
    }
    Some(
        image::open(path)
            .ok()?
            .resize_to_fill(84, 84, image::imageops::FilterType::Lanczos3)
            .to_rgba8(),
    )
}

fn capture_icon(icon: HICON) -> Option<image::RgbaImage> {
    let mut bgra = vec![0u8; 48 * 48 * 4];
    let ok = unsafe {
        let mem = CreateCompatibleDC(null_mut());
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 48,
                biHeight: -48,
                biPlanes: 1,
                biBitCount: 32,
                ..zeroed()
            },
            ..zeroed()
        };
        let mut ptr = null_mut();
        let bmp = CreateDIBSection(mem, &info, DIB_RGB_COLORS, &mut ptr, null_mut(), 0);
        if bmp.is_null() {
            DeleteDC(mem);
            return None;
        }
        let old = SelectObject(mem, bmp);
        let ok = DrawIconEx(mem, 0, 0, icon, 48, 48, 0, null_mut(), DI_NORMAL) != 0;
        if ok {
            GdiFlush();
            std::ptr::copy_nonoverlapping(ptr.cast::<u8>(), bgra.as_mut_ptr(), bgra.len());
        }
        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ok
    };
    if !ok {
        return None;
    }
    let alpha = bgra.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        if !alpha && px[..3] != [0, 0, 0] {
            px[3] = 255;
        }
        if alpha && px[3] > 0 && px[3] < 255 {
            for i in 0..3 {
                px[i] = (u16::from(px[i]) * 255 / u16::from(px[3])).min(255) as u8;
            }
        }
    }
    image::RgbaImage::from_raw(48, 48, bgra)
}

pub fn choose_exe(owner: HWND, title: &str) -> Option<PathBuf> {
    let mut buffer = [0u16; 32768];
    let caption = wide(title);
    let filter = wide("Executable (*.exe)\0*.exe\0");
    let mut dialog = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: buffer.as_mut_ptr(),
        nMaxFile: buffer.len() as u32,
        lpstrTitle: caption.as_ptr(),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..unsafe { zeroed() }
    };
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    use std::os::windows::ffi::OsStringExt;
    let name = std::ffi::OsString::from_wide(buffer.split(|&n| n == 0).next()?);
    Some(PathBuf::from(name))
}

pub fn working_set() -> Option<u64> {
    let mut info: PROCESS_MEMORY_COUNTERS = unsafe { zeroed() };
    let bytes = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    (unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut info, bytes) } != 0)
        .then_some(info.WorkingSetSize as u64)
}

pub fn dpi(hwnd: HWND) -> f64 {
    (unsafe { GetDpiForWindow(hwnd) } as f64 / 96.).max(1.)
}

pub fn window_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut rect);
    }
    rect
}

pub fn caption(hwnd: HWND, light: bool) {
    let dark = i32::from(!light);
    let color = rgb(if light { "#f3f5f8" } else { "#111216" }, 0x161211);
    let no_border = 0xfffffffeu32;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&dark as *const i32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            (&no_border as *const u32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR as u32,
            (&color as *const u32).cast(),
            4,
        );
    }
}

pub fn place_popup(hwnd: HWND, bounds: RECT, radius: f64) {
    unsafe {
        let old = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let border = (WS_CAPTION
            | WS_THICKFRAME
            | WS_BORDER
            | WS_DLGFRAME
            | WS_SYSMENU
            | WS_MAXIMIZEBOX
            | WS_MINIMIZEBOX) as isize;
        let style = (old & !border) | (WS_POPUP | WS_CLIPSIBLINGS | WS_CLIPCHILDREN) as isize;
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let clean_ex = ex
            & !((WS_EX_CLIENTEDGE | WS_EX_WINDOWEDGE | WS_EX_STATICEDGE | WS_EX_DLGMODALFRAME)
                as isize);
        if old != style || ex != clean_ex {
            SetWindowLongPtrW(hwnd, GWL_STYLE, style);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, clean_ex);
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        let disabled = 2u32;
        let no_border = 0xfffffffeu32;
        DwmSetWindowAttribute(hwnd, 2, (&disabled as *const u32).cast(), 4);
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            (&no_border as *const u32).cast(),
            4,
        );
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            SWP_NOACTIVATE,
        );
        let mut rect = RECT::default();
        if GetClientRect(hwnd, &mut rect) != 0 && rect.right > 0 && rect.bottom > 0 {
            let round = (2. * radius * dpi(hwnd)).round() as i32;
            let clip = CreateRoundRectRgn(0, 0, rect.right, rect.bottom, round, round);
            if !clip.is_null() && SetWindowRgn(hwnd, clip, 1) == 0 {
                DeleteObject(clip);
            }
        }
    }
}

pub fn error(message: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("Steam Pulse").as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}
