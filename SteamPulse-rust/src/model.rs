use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::LazyLock,
};

static DEFAULTS: LazyLock<Map<String, Value>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/defaults.json")).expect("valid embedded defaults")
});
static LOCALES: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../assets/Locales.json")).expect("valid embedded locales")
});
const EDITABLE: &str = include_str!("../assets/editable.txt");

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LocalGame {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub ignored: bool,
}

#[derive(Clone)]
pub struct Preferences(Map<String, Value>);

impl Default for Preferences {
    fn default() -> Self {
        Self(DEFAULTS.clone())
    }
}

impl Preferences {
    pub fn load_json(raw: &str) -> serde_json::Result<Self> {
        let input: Map<String, Value> = serde_json::from_str(raw.trim_start_matches('\u{feff}'))?;
        let mut p = Self::default();
        for (key, value) in &input {
            if let Some(default) = DEFAULTS.get(key) {
                if key == "HorizontalPosition" && value.is_number() || same_kind(default, value) {
                    p.set(key, value.clone());
                }
            } else {
                p.set(key, value.clone());
            }
        }
        let version = input.get("FormatVersion").and_then(Value::as_i64);
        if version.is_none() {
            for key in [
                "Glass",
                "Gradient",
                "ClickThrough",
                "SettingsGlass",
                "GlassTint",
                "SettingsTint",
                "GlassDepth",
                "GlassBlur",
                "GlassDispersion",
                "GlassLight",
                "Shine",
            ] {
                p.set(key, DEFAULTS[key].clone());
            }
        }
        if version.unwrap_or(0) < 5 {
            for (out, source) in [
                ("SettingsGlassDepth", "GlassDepth"),
                ("SettingsGlassBlur", "GlassBlur"),
                ("SettingsGlassDispersion", "GlassDispersion"),
                ("SettingsGlassLight", "GlassLight"),
                ("SettingsShine", "Shine"),
            ] {
                p.set(out, p.get(source).clone());
            }
            p.set("FormatVersion", json!(5));
        }
        for key in EDITABLE.split_whitespace().chain(["Placement", "Offset"]) {
            if let Some(value) = p.get(key).as_f64() {
                p.set(key, bounded(key, value));
            }
        }
        Ok(p)
    }

    pub fn load(folder: &Path) -> Self {
        fs::read_to_string(folder.join("settings.json"))
            .ok()
            .and_then(|text| Self::load_json(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, folder: &Path) -> io::Result<()> {
        fs::create_dir_all(folder)?;
        let mut file = tempfile::NamedTempFile::new_in(folder)?;
        serde_json::to_writer_pretty(&mut file, &self.0)?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(folder.join("settings.json"))
            .map_err(|error| error.error)?;
        Ok(())
    }

    pub fn set(&mut self, key: &str, value: Value) {
        self.0.insert(key.to_owned(), value);
    }
    pub fn get(&self, key: &str) -> &Value {
        self.0.get(key).unwrap_or(&Value::Null)
    }
    pub fn flag(&self, key: &str) -> bool {
        self.get(key).as_bool().unwrap_or(false)
    }
    pub fn number(&self, key: &str) -> f64 {
        self.get(key).as_f64().unwrap_or(0.0)
    }
    pub fn text(&self, key: &str) -> &str {
        self.get(key).as_str().unwrap_or_default()
    }
    pub fn ignored(&self, id: u32) -> bool {
        self.get("IgnoredGames")
            .as_array()
            .is_some_and(|ids| ids.contains(&json!(id)))
    }
    pub fn local_games(&self) -> Vec<LocalGame> {
        self.get("LocalGames")
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect()
    }

    pub fn edit(&mut self, key: &str, value: &Value, displays: &[&str]) -> bool {
        if !EDITABLE.split_whitespace().any(|k| k == key) {
            return false;
        }
        if key == "HorizontalPosition" && value.is_null() {
            self.set(key, Value::Null);
        } else if let Some(n) = value.as_f64() {
            if !DEFAULTS[key].is_number() && key != "HorizontalPosition" {
                return false;
            }
            self.set(key, bounded(key, n));
        } else if value.is_boolean() && DEFAULTS[key].is_boolean() {
            self.set(key, value.clone());
        } else if let Some(s) = value.as_str().filter(|_| DEFAULTS[key].is_string()) {
            let valid = match key {
                "Language" => s.is_empty() || LOCALES.get(s).is_some(),
                "MonitorName" => s.is_empty() || displays.contains(&s),
                "Country" => {
                    s.is_empty() || s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic())
                }
                _ => {
                    s.len() == 7
                        && s.starts_with('#')
                        && s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
                }
            };
            if !valid {
                return false;
            }
            self.set(key, value.clone());
        } else {
            return false;
        }
        true
    }

    pub fn entries(&self, startup: bool) -> Value {
        let mut entries: Map<String, Value> = EDITABLE
            .split_whitespace()
            .map(|key| (key.to_owned(), self.get(key).clone()))
            .collect();
        entries.insert("AutoStart".into(), json!(startup));
        entries.insert("IgnoredGames".into(), self.get("IgnoredGames").clone());
        entries.into()
    }
}

pub fn language<'a>(requested: &'a str, system: &'a str) -> &'a str {
    let candidate = if requested.is_empty() {
        system.split(['-', '_']).next().unwrap_or("en")
    } else {
        requested
    };
    if LOCALES.get(candidate).is_some() {
        candidate
    } else {
        "en"
    }
}

pub fn tr<'a>(language: &str, key: &'a str) -> &'a str {
    LOCALES[language][key]
        .as_str()
        .or_else(|| LOCALES["en"][key].as_str())
        .unwrap_or(key)
}

fn same_kind(a: &Value, b: &Value) -> bool {
    a.is_number() && b.is_number() || std::mem::discriminant(a) == std::mem::discriminant(b)
}

fn bounded(key: &str, n: f64) -> Value {
    let (min, max) = match key {
        "HorizontalPosition" => (0., 100.),
        "MinDiscount" => (5., 95.),
        "MetricsInterval" => (1., 5.),
        "FriendCycle" => (3., 20.),
        "Placement" => (0., 2.),
        "Width" => (200., 550.),
        "Radius" => (0., 11.),
        "MenuRadius" => (0., 36.),
        "Offset" => (-1000., 1000.),
        "MenuGlassTint" => (0.25, 1.),
        "GlassDepth" | "SettingsGlassDepth" | "MenuGlassDepth" => (-5., 5.),
        "GlassBlur" | "SettingsGlassBlur" | "MenuGlassBlur" => (0., 10.),
        "GlassDispersion" | "SettingsGlassDispersion" | "MenuGlassDispersion" => (0., 3.),
        "GlassLight" | "SettingsGlassLight" | "MenuGlassLight" | "Angle" => (0., 360.),
        "Opacity" => (0.1, 1.),
        "SettingsTint" => (0.05, 1.),
        _ => (0., 1.),
    };
    let n = n.clamp(min, max);
    match key {
        "MinDiscount" | "MetricsInterval" | "FriendCycle" | "Placement" => json!(n as i32),
        _ => json!(n),
    }
}

pub fn matches_executable(running: &Path, saved: &Path) -> bool {
    if same_path(running, saved) {
        return true;
    }
    let names_match = running
        .file_name()
        .zip(saved.file_name())
        .is_some_and(|(a, b)| same_name(&a.to_string_lossy(), &b.to_string_lossy()));
    let (a, b) = (versioned_root(running), versioned_root(saved));
    names_match
        && (a.is_some() || b.is_some())
        && a.or_else(|| running.parent())
            .zip(b.or_else(|| saved.parent()))
            .is_some_and(|(a, b)| same_path(a, b))
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let lhs = std::path::absolute(a).unwrap_or_else(|_| a.to_path_buf());
    let rhs = std::path::absolute(b).unwrap_or_else(|_| b.to_path_buf());
    same_name(&lhs.to_string_lossy(), &rhs.to_string_lossy())
}

fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
        || !a.is_ascii() && !b.is_ascii() && a.to_lowercase() == b.to_lowercase()
}

pub fn versioned_root(path: &Path) -> Option<&Path> {
    let folder = path.parent()?;
    let name = folder.file_name()?.to_str()?.to_ascii_lowercase();
    app_version(&name)?;
    folder.parent()
}

fn app_version(name: &str) -> Option<Vec<u32>> {
    let tail = name.strip_prefix("app-")?.split('-').next()?;
    let parts = tail
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect::<Option<Vec<_>>>()?;
    (2..=4).contains(&parts.len()).then_some(parts)
}

pub fn standalone(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        && !path
            .components()
            .any(|part| part.as_os_str().eq_ignore_ascii_case("steamapps"))
        && !path.file_stem().is_some_and(|e| {
            [
                "steam",
                "steamwebhelper",
                "steamservice",
                "steamerrorreporter",
                "steamerrorreporter64",
                "gameoverlayui",
            ]
            .iter()
            .any(|name| e.eq_ignore_ascii_case(name))
        })
}

pub fn resolve_launch_path(path: &Path) -> Option<std::path::PathBuf> {
    if !path.is_absolute() || !standalone(path) {
        return None;
    }
    let root = versioned_root(path).or_else(|| path.parent())?;
    let filename = path.file_name()?;
    let stable = root.join(filename);
    if stable.is_file() {
        return Some(stable);
    }
    fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let version = app_version(&entry.file_name().to_string_lossy().to_ascii_lowercase())?;
            let candidate = entry.path().join(filename);
            candidate.is_file().then_some((version, candidate))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, path)| path)
}
