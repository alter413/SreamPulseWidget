use crate::{
    model::{self, Preferences},
    steam::{self, Download, Game, LibraryGame, SteamLog, Vdf},
};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::null_mut,
    time::{Duration, Instant},
};
use sysinfo::{Networks, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    Globalization::GetUserDefaultLocaleName,
    System::{RemoteDesktop::ProcessIdToSessionId, Threading::GetCurrentProcessId},
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::*},
};
use winreg::{
    RegKey,
    enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE},
};

const STEAM: &str = r"Software\Valve\Steam";
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP: &str = "SteamPulse";

pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain([0]).collect()
}

pub fn system_locale() -> String {
    let mut buf = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..len.saturating_sub(1) as usize])
}

pub fn steam_app(id: u32) -> bool {
    shell_open(format!("steam://url/StoreAppPage/{id}"))
}

pub fn shell_open(s: impl AsRef<OsStr>) -> bool {
    let target = wide(s);
    unsafe {
        ShellExecuteW(
            null_mut(),
            wide("open").as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        ) as isize
            > 32
    }
}

fn startup_command() -> std::io::Result<String> {
    Ok(format!("\"{}\"", std::env::current_exe()?.display()))
}

pub fn startup() -> bool {
    let user = RegKey::predef(HKEY_CURRENT_USER);
    let current = user
        .open_subkey(RUN)
        .and_then(|k| k.get_value::<String, _>(STARTUP));
    current
        .ok()
        .zip(startup_command().ok())
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(&b))
}

pub fn set_startup(enabled: bool) -> std::io::Result<()> {
    let user = RegKey::predef(HKEY_CURRENT_USER);
    if enabled {
        let (key, _) = user.create_subkey(RUN)?;
        key.set_value(STARTUP, &startup_command()?)
    } else if startup() {
        user.open_subkey_with_flags(RUN, KEY_SET_VALUE)?
            .delete_value(STARTUP)
    } else {
        Ok(())
    }
}

pub fn active_account() -> u32 {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(format!(r"{STEAM}\ActiveProcess"))
        .and_then(|k| k.get_value("ActiveUser"))
        .unwrap_or_default()
}

fn steam_key() -> Option<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(STEAM).ok()
}

pub fn read_library(root: Option<&Path>) -> Vec<LibraryGame> {
    let Some(root) = root else {
        return Vec::new();
    };
    let apps = steam_key().and_then(|k| k.open_subkey("Apps").ok());
    let known = apps.as_ref().map_or_else(HashMap::new, |apps| {
        apps.enum_keys()
            .flatten()
            .filter_map(|name| {
                let id = name.parse().ok()?;
                let label = apps
                    .open_subkey(name)
                    .and_then(|k| k.get_value("Name"))
                    .unwrap_or_default();
                Some((id, label))
            })
            .collect()
    });
    steam::library(root, known, active_account())
}

fn discover() -> Option<PathBuf> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let machine = [r"SOFTWARE\WOW6432Node\Valve\Steam", r"SOFTWARE\Valve\Steam"].map(|path| {
        hklm.open_subkey(path)
            .and_then(|k| k.get_value::<String, _>("InstallPath"))
            .ok()
            .map(PathBuf::from)
    });
    let user = steam_key()
        .and_then(|k| k.get_value::<String, _>("SteamPath").ok())
        .map(PathBuf::from);
    let default = std::env::var_os("ProgramFiles(x86)").map(|s| PathBuf::from(s).join("Steam"));
    [user, machine[0].clone(), machine[1].clone(), default]
        .into_iter()
        .flatten()
        .find(|p| p.join("steam.exe").is_file())
}

#[derive(Clone, Copy, Debug)]
pub struct AppWindow {
    pub handle: isize,
    pub pid: u32,
}

pub fn focus(window: AppWindow) -> bool {
    let hwnd = window.handle as HWND;
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != window.pid || IsWindowVisible(hwnd) == 0 {
            return false;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindowAsync(hwnd, SW_RESTORE);
        }
        if SetForegroundWindow(hwnd) == 0 {
            FlashWindow(hwnd, 1);
        }
    }
    true
}

fn windows() -> HashMap<u32, AppWindow> {
    unsafe extern "system" fn gather(hwnd: HWND, data: LPARAM) -> i32 {
        unsafe {
            if IsWindowVisible(hwnd) != 0
                && GetWindow(hwnd, GW_OWNER).is_null()
                && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW as isize == 0
            {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                let windows = &mut *(data as *mut HashMap<u32, AppWindow>);
                let row = AppWindow {
                    handle: hwnd as isize,
                    pid,
                };
                if hwnd == GetForegroundWindow() {
                    windows.insert(pid, row);
                } else {
                    windows.entry(pid).or_insert(row);
                }
            }
        }
        1
    }
    let mut out = HashMap::new();
    unsafe {
        EnumWindows(Some(gather), &mut out as *mut _ as LPARAM);
    }
    out
}

fn session(pid: u32) -> Option<u32> {
    let mut session = 0;
    (unsafe { ProcessIdToSessionId(pid, &mut session) } != 0).then_some(session)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Recent {
    pub id: String,
    pub started: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Load {
    pub cpu: Option<u32>,
    pub memory_percent: Option<u32>,
    #[serde(rename = "memoryGiB")]
    pub memory_gib: Option<f64>,
}

#[derive(Default, Clone)]
pub struct Status {
    pub root: Option<PathBuf>,
    pub active: bool,
    pub game: Option<Game>,
    pub download: Option<Download>,
    pub load: Option<Load>,
    pub network: Option<f64>,
    pub account: u64,
    pub windows: HashMap<String, AppWindow>,
    pub launches: Vec<Recent>,
}

pub struct Reader {
    root: Option<PathBuf>,
    search_at: Instant,
    libraries_at: Instant,
    libraries: Vec<PathBuf>,
    steam_log: SteamLog,
    system: System,
    networks: Networks,
    previous_network: Option<(Instant, u64)>,
}

impl Default for Reader {
    fn default() -> Self {
        Self {
            root: None,
            search_at: Instant::now(),
            libraries_at: Instant::now(),
            libraries: Vec::new(),
            steam_log: SteamLog::default(),
            system: System::new(),
            networks: Networks::new(),
            previous_network: None,
        }
    }
}

impl Reader {
    pub fn sample(&mut self, p: &Preferences, visuals: bool, recent: bool) -> Status {
        let now = Instant::now();
        if !visuals && !recent {
            self.previous_network = None;
            return Status {
                root: self.root.clone(),
                ..Status::default()
            };
        }
        let configured = p.local_games();
        let candidates: HashSet<_> = configured
            .iter()
            .filter_map(|g| Path::new(&g.path).file_name())
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .collect();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::Never),
        );
        let pids: Vec<_> = self
            .system
            .processes()
            .iter()
            .filter(|(_, proc)| {
                let name = proc.name().to_string_lossy().to_ascii_lowercase();
                name == "steam.exe" || candidates.contains(&name)
            })
            .map(|(pid, _)| *pid)
            .collect();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            false,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
        );
        let current_session = session(unsafe { GetCurrentProcessId() });
        let processes: Vec<_> = pids
            .iter()
            .filter(|pid| current_session.is_some() && session(pid.as_u32()) == current_session)
            .filter_map(|pid| self.system.process(*pid))
            .collect();
        let steam = processes
            .iter()
            .find(|proc| proc.name().eq_ignore_ascii_case("steam.exe"));
        if now >= self.search_at {
            self.search_at = now + Duration::from_secs(8);
            let found = discover()
                .or_else(|| steam.and_then(|proc| proc.exe()?.parent().map(Path::to_owned)));
            if self.root != found {
                self.root = found;
                self.libraries_at = now;
                self.steam_log = SteamLog::default();
            }
        }
        let mut status = Status {
            active: steam.is_some(),
            root: self.root.clone(),
            ..Status::default()
        };
        if let Some(root) = self.root.as_ref().filter(|_| status.active && visuals) {
            if now >= self.libraries_at {
                self.libraries_at = now + Duration::from_secs(30);
                self.libraries = steam::libraries(root);
            }
            let key = steam_key();
            let apps = key.as_ref().and_then(|k| k.open_subkey("Apps").ok());
            let app = |id: u32| apps.as_ref()?.open_subkey(id.to_string()).ok();
            let selected: Option<u32> = key
                .as_ref()
                .and_then(|k| k.get_value::<u32, _>("RunningAppID").ok())
                .filter(|id| {
                    *id != 0
                        && app(*id).and_then(|k| k.get_value::<u32, _>("Running").ok()) != Some(0)
                });
            let running = selected.or_else(|| {
                apps.as_ref()?.enum_keys().flatten().find_map(|id| {
                    let id: u32 = id.parse().ok()?;
                    (app(id)?.get_value::<u32, _>("Running").ok()? == 1).then_some(id)
                })
            });
            let game = |id: u32, state: Option<&Vdf>| Game {
                id,
                art: steam::art(root, id),
                name: state
                    .and_then(|s| s.value("name"))
                    .map(str::to_owned)
                    .or_else(|| app(id)?.get_value("Name").ok())
                    .unwrap_or_else(|| format!("Steam App {id}")),
            };
            if let Some(id) = running.filter(|id| !p.ignored(*id)) {
                status.game = Some(game(id, steam::manifest(&self.libraries, id).as_ref()));
            }
            if p.flag("Downloads") {
                self.steam_log.read(&root.join("logs/content_log.txt"));
                let id = self.steam_log.id();
                if id != 0 && !p.ignored(id) {
                    let m = steam::manifest(&self.libraries, id);
                    status.download = Some(self.steam_log.download(
                        game(id, m.as_ref()),
                        m.as_ref(),
                        Local::now().naive_local(),
                    ));
                }
            }
        }
        let account = active_account();
        if account != 0 {
            status.account = 76_561_197_960_265_728 + u64::from(account);
        }
        let visible = if configured.is_empty() {
            HashMap::new()
        } else {
            windows()
        };
        let foreground = unsafe { GetForegroundWindow() } as isize;
        let mut local = None;
        for configured in configured
            .into_iter()
            .filter(|game| !game.ignored && Path::new(&game.path).is_absolute())
        {
            let mut matched: Vec<_> = processes
                .iter()
                .filter_map(|proc| {
                    let path = proc.exe().filter(|path| {
                        model::matches_executable(path, Path::new(&configured.path))
                    })?;
                    let hwnd = visible.get(&proc.pid().as_u32())?;
                    Some((*hwnd, proc.start_time(), path))
                })
                .collect();
            matched
                .sort_unstable_by_key(|(hwnd, started, _)| (hwnd.handle != foreground, *started));
            let Some((hwnd, _, path)) = matched.first() else {
                continue;
            };
            if visuals && (local.is_none() || hwnd.handle == foreground) {
                local = Some(Game {
                    id: 0,
                    name: configured.name.clone(),
                    art: Some(path.to_path_buf()),
                });
            }
            if recent && model::standalone(path) {
                status.windows.insert(configured.id.clone(), *hwnd);
                if let Some(started) = matched
                    .iter()
                    .filter_map(|(_, start, _)| DateTime::from_timestamp(*start as i64, 0))
                    .min()
                {
                    status.launches.push(Recent {
                        id: configured.id,
                        started,
                    });
                }
            }
        }
        if local.is_some() {
            status.game = local;
        }
        if visuals && p.flag("MonitorSystem") {
            let mut load = Load::default();
            if p.flag("MonitorCpu") {
                self.system.refresh_cpu_usage();
                load.cpu = Some(self.system.global_cpu_usage().round() as u32);
            }
            if p.flag("MonitorRam") {
                self.system.refresh_memory();
                let total = self.system.total_memory();
                let used = total.saturating_sub(self.system.available_memory());
                if total > 0 {
                    load.memory_percent = Some((100. * used as f64 / total as f64).round() as u32);
                    load.memory_gib = Some(used as f64 / 1_073_741_824.);
                }
            }
            status.load = Some(load);
        }
        if visuals && p.flag("Network") {
            self.networks.refresh(true);
            let total = self
                .networks
                .iter()
                .filter(|(name, _)| !name.to_ascii_lowercase().contains("loopback"))
                .map(|(_, n)| n.total_received())
                .fold(0u64, u64::saturating_add);
            if let Some((last, received)) = self.previous_network {
                let elapsed = now.duration_since(last).as_secs_f64();
                status.network = ((0.1..15.).contains(&elapsed) && total >= received)
                    .then(|| (total - received) as f64 / elapsed);
            }
            self.previous_network = Some((now, total));
        } else {
            self.previous_network = None;
        }
        status
    }
}
