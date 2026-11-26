use chrono::NaiveDateTime;
use regex::Regex;
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::SystemTime,
};

#[derive(Default, Debug)]
pub struct Vdf {
    children: BTreeMap<String, Vdf>,
    values: BTreeMap<String, String>,
}

impl Vdf {
    pub fn child(&self, key: &str) -> Option<&Vdf> {
        self.children.get(&key.to_ascii_lowercase())
    }
    pub fn value(&self, key: &str) -> Option<&str> {
        self.values
            .get(&key.to_ascii_lowercase())
            .map(String::as_str)
    }
    fn number(&self, key: &str) -> u64 {
        self.value(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or_default()
    }
    pub fn libraries(&self) -> Vec<String> {
        self.children
            .values()
            .filter_map(|v| v.value("path"))
            .chain(
                self.values
                    .iter()
                    .filter(|(k, _)| k.parse::<u32>().is_ok())
                    .map(|(_, v)| v.as_str()),
            )
            .map(str::to_owned)
            .collect()
    }
    pub fn parse(raw: &str) -> Self {
        static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"//[^\r\n]*|"((?:\\.|[^"\\])*)"|[{}]|[^\s{}"]+"#).unwrap()
        });
        static ESCAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\\([\\"])"#).unwrap());
        fn parse_node<'a>(tokens: &mut regex::CaptureMatches<'_, 'a>, depth: usize) -> Vdf {
            let mut output = Vdf::default();
            while let Some(k) = tokens.find(|m| !m[0].starts_with("//")) {
                if &k[0] == "}" {
                    break;
                }
                let Some(v) = tokens.find(|m| !m[0].starts_with("//")) else {
                    break;
                };
                let key = k
                    .get(1)
                    .map_or_else(
                        || k[0].to_owned(),
                        |m| ESCAPE.replace_all(m.as_str(), "$1").into_owned(),
                    )
                    .to_ascii_lowercase();
                match &v[0] {
                    "{" if depth < 48 => {
                        output.children.insert(key, parse_node(tokens, depth + 1));
                    }
                    "{" | "}" => break,
                    text => {
                        output.values.insert(
                            key,
                            v.get(1).map_or_else(
                                || text.to_owned(),
                                |m| ESCAPE.replace_all(m.as_str(), "$1").into_owned(),
                            ),
                        );
                    }
                }
            }
            output
        }
        parse_node(
            &mut TOKEN.captures_iter(raw.trim_start_matches('\u{feff}')),
            0,
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct ContentSample {
    pub id: u32,
    pub done: u64,
    pub total: u64,
    pub speed: f64,
    anchor: Option<NaiveDateTime>,
    last_rate: Option<NaiveDateTime>,
    integrated: f64,
}

impl ContentSample {
    pub fn parse(mut self, text: &str) -> Self {
        static APP: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)^AppID (\d+)\s+(.*)").unwrap());
        static BYTES: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)(?:update started|update progress)\s*:\s*download (\d+)/(\d+)")
                .unwrap()
        });
        static RATE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"(?i)^Current download rate:\s+([\d.]+)\s+([KMG]?bps)").unwrap()
        });
        static ADJUST: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)rate was [\d.]+, now ([\d.]+)").unwrap());
        for line in text.lines() {
            let Some((date, body)) = line.strip_prefix('[').and_then(|line| line.split_once(']'))
            else {
                continue;
            };
            let Ok(time) = NaiveDateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S") else {
                continue;
            };
            let body = body.trim();
            if let Some(app) = APP.captures(body) {
                let Ok(id) = app[1].parse::<u32>() else {
                    continue;
                };
                let message = app[2].to_ascii_lowercase();
                if message.contains("update changed") {
                    let flags: Vec<_> = message
                        .split_once(':')
                        .map_or("", |(_, v)| v)
                        .split(',')
                        .map(str::trim)
                        .collect();
                    let running = flags.iter().any(|s| {
                        [
                            "running update",
                            "running",
                            "downloading",
                            "staging",
                            "committing",
                        ]
                        .contains(s)
                    });
                    let stopped = flags
                        .iter()
                        .any(|s| ["stopping", "none", "paused"].contains(s));
                    if running && !stopped && id != self.id {
                        self = Self {
                            id,
                            anchor: Some(time),
                            ..Self::default()
                        };
                    }
                    if (!running || stopped) && id == self.id {
                        self = Self::default();
                    }
                }
                if let Some(bytes) = BYTES.captures(&message)
                    && let (Ok(done), Ok(total)) = (bytes[1].parse(), bytes[2].parse())
                {
                    self = Self {
                        id,
                        done,
                        total,
                        anchor: Some(time),
                        ..Self::default()
                    };
                }
                if id == self.id
                    && ["scheduler finished", "update canceled", "finished update"]
                        .iter()
                        .any(|s| message.starts_with(s))
                {
                    self = Self::default();
                }
            } else if self.id != 0 {
                let reading = RATE
                    .captures(body)
                    .and_then(|m| {
                        let scale = match m[2].to_ascii_lowercase().as_str() {
                            "kbps" => 125.,
                            "mbps" => 125_000.,
                            "gbps" => 125_000_000.,
                            _ => 0.125,
                        };
                        m[1].parse::<f64>().ok().map(|n| n * scale)
                    })
                    .or_else(|| {
                        ADJUST
                            .captures(body)?
                            .get(1)?
                            .as_str()
                            .parse::<f64>()
                            .ok()
                            .map(|n| n * 125_000.)
                    });
                if let Some(speed) = reading.filter(|n| n.is_finite()) {
                    let elapsed = (time - self.last_rate.or(self.anchor).unwrap_or(time))
                        .num_seconds()
                        .clamp(0, 90);
                    self.integrated += speed * elapsed as f64;
                    self.speed = speed;
                    self.last_rate = Some(time);
                }
            }
        }
        self
    }

    fn estimate(&self, now: NaiveDateTime) -> f64 {
        let age = self.last_rate.map_or(0, |time| (now - time).num_seconds());
        self.done as f64
            + (self.integrated + self.speed * age.clamp(0, 68) as f64)
                .clamp(0., self.total.saturating_sub(self.done) as f64)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: u32,
    pub name: String,
    #[serde(skip)]
    pub art: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    #[serde(flatten)]
    pub game: Game,
    pub percent: Option<f64>,
    pub estimated: bool,
    pub speed: Option<f64>,
    pub installing: bool,
    pub remaining: Option<f64>,
}

#[derive(Default)]
pub struct SteamLog {
    file: PathBuf,
    metadata: Option<(u64, SystemTime)>,
    sample: ContentSample,
    exact: Option<(u32, Option<NaiveDateTime>, u64, NaiveDateTime, f64)>,
}

impl SteamLog {
    pub fn read(&mut self, path: &Path) {
        let Ok(mut stream) = File::open(path) else {
            *self = Self::default();
            return;
        };
        let Ok((len, modified)) = stream.metadata().and_then(|m| Ok((m.len(), m.modified()?)))
        else {
            return;
        };
        if self.file == path && self.metadata == Some((len, modified)) {
            return;
        }
        const MAX: u64 = 524_288;
        let previous = self
            .metadata
            .map(|m| m.0)
            .filter(|&old| self.file == path && len > old && len - old <= MAX);
        let start = previous.unwrap_or_else(|| len.saturating_sub(MAX));
        if stream.seek(SeekFrom::Start(start)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        if stream.take(len - start).read_to_end(&mut bytes).is_err()
            || bytes.len() as u64 != len - start
        {
            return;
        }
        if !bytes.is_empty() && !bytes.ends_with(b"\n") {
            return;
        }
        let text = String::from_utf8_lossy(&bytes);
        let text = if previous.is_none() && start > 0 {
            text.split_once('\n').map_or("", |(_, t)| t)
        } else {
            &text
        };
        let initial = if previous.is_some() {
            self.sample.clone()
        } else {
            ContentSample::default()
        };
        self.sample = initial.parse(text);
        self.file = path.to_owned();
        self.metadata = Some((len, modified));
    }

    pub fn id(&self) -> u32 {
        self.sample.id
    }

    pub fn download(&mut self, game: Game, state: Option<&Vdf>, now: NaiveDateTime) -> Download {
        let s = &self.sample;
        let done = s.done.max(state.map_or(0, |m| m.number("BytesDownloaded")));
        let total = if s.total > 0 {
            s.total
        } else {
            state.map_or(0, |m| m.number("BytesToDownload"))
        };
        let exact = self.exact.get_or_insert((s.id, s.anchor, done, now, 0.));
        if exact.0 != s.id || exact.1 != s.anchor || done < exact.2 {
            *exact = (s.id, s.anchor, done, now, 0.);
        } else if done > exact.2 {
            exact.4 = (done - exact.2) as f64 / (now - exact.3).num_seconds().max(1) as f64;
            exact.2 = done;
            exact.3 = now;
        }
        let mut speed = s
            .last_rate
            .filter(|t| (0..95).contains(&(now - *t).num_seconds()))
            .map_or(0., |_| s.speed);
        if speed == 0. && (now - exact.3).num_seconds() < 6 {
            speed = exact.4;
        }
        let extrapolated = s.estimate(now);
        let estimated = done < total && extrapolated > done as f64;
        let percent = (total > 0).then(|| {
            if estimated {
                (100. * extrapolated / total as f64).min(99.8)
            } else {
                (100. * done as f64 / total as f64).min(100.)
            }
        });
        let installing = total > 0 && done >= total;
        let received = percent.map_or(done as f64, |p| total as f64 * p / 100.);
        let remaining = (total as f64 - received) / speed;
        Download {
            game,
            percent,
            estimated,
            speed: (speed > 0. && !installing).then_some(speed),
            installing,
            remaining: (remaining > 0. && remaining <= 31_536_000. && !installing)
                .then(|| remaining.ceil()),
        }
    }
}

pub fn read_text(path: &Path) -> String {
    let mut data = Vec::new();
    let Ok(file) = File::open(path) else {
        return String::new();
    };
    if file.take(64 * 1024 * 1024).read_to_end(&mut data).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&data).into_owned()
}

pub fn libraries(root: &Path) -> Vec<PathBuf> {
    let folders = Vdf::parse(&read_text(&root.join("steamapps/libraryfolders.vdf")));
    let mut seen = HashSet::new();
    std::iter::once(root.to_owned())
        .chain(
            folders
                .child("libraryfolders")
                .into_iter()
                .flat_map(Vdf::libraries)
                .map(PathBuf::from),
        )
        .filter(|p| seen.insert(p.to_string_lossy().to_ascii_lowercase()))
        .collect()
}

pub fn manifest(folders: &[PathBuf], id: u32) -> Option<Vdf> {
    folders
        .iter()
        .map(|root| {
            Vdf::parse(&read_text(
                &root.join(format!("steamapps/appmanifest_{id}.acf")),
            ))
        })
        .find_map(|mut root| {
            root.children
                .remove("appstate")
                .filter(|s| s.value("name").is_some())
        })
}

pub fn art(root: &Path, id: u32) -> Option<PathBuf> {
    let cache = root.join("appcache/librarycache");
    let folder = cache.join(id.to_string());
    let items = || {
        fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
    };
    items()
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpg")))
        .or_else(|| {
            items()
                .filter(|p| p.is_dir())
                .flat_map(|p| [p.join("library_600x900.jpg"), p.join("library_header.jpg")])
                .find(|p| p.is_file())
        })
        .or_else(|| {
            ["_icon.jpg", "_library_600x900.jpg", "_header.jpg"]
                .iter()
                .map(|suffix| cache.join(format!("{id}{suffix}")))
                .find(|p| p.is_file())
        })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryGame {
    pub id: u32,
    pub name: String,
    pub installed: bool,
}

pub fn library(root: &Path, mut known: HashMap<u32, String>, account: u32) -> Vec<LibraryGame> {
    let mut installed = HashSet::new();
    for folder in libraries(root) {
        for entry in fs::read_dir(folder.join("steamapps"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let file = entry.file_name();
            let Some(id) = file.to_str().and_then(|f| {
                f.strip_prefix("appmanifest_")?
                    .strip_suffix(".acf")?
                    .parse()
                    .ok()
            }) else {
                continue;
            };
            let state = Vdf::parse(&read_text(&entry.path()));
            if let Some(name) = state
                .child("appstate")
                .and_then(|s| s.value("name"))
                .filter(|n| !n.is_empty())
            {
                known.insert(id, name.to_owned());
                installed.insert(id);
            }
        }
    }
    if account != 0 {
        let file = root.join(format!("userdata/{account}/config/localconfig.vdf"));
        let config = Vdf::parse(&read_text(&file));
        let apps = ["userlocalconfigstore", "software", "valve", "steam", "apps"]
            .iter()
            .try_fold(&config, |v, k| v.child(k));
        for id in apps
            .into_iter()
            .flat_map(|v| v.children.keys())
            .filter_map(|id| id.parse().ok())
        {
            known.entry(id).or_default();
        }
    }
    let meta = parse_appinfo(&root.join("appcache/appinfo.vdf"), &known);
    let mut result: Vec<_> = known
        .into_iter()
        .filter_map(|(id, name)| {
            if id == 228980 {
                return None;
            }
            let (title, kind) = meta.get(&id).cloned().unwrap_or_default();
            if kind.is_some_and(|t| {
                !["game", "application", "demo", "mod", "beta"]
                    .contains(&t.to_ascii_lowercase().as_str())
            }) {
                return None;
            }
            let name = title.unwrap_or(name);
            (!name.is_empty()).then(|| LibraryGame {
                id,
                name,
                installed: installed.contains(&id),
            })
        })
        .collect();
    result.sort_by_key(|g| (g.name.to_lowercase(), g.id));
    result
}

type AppInfo = (Option<String>, Option<String>);

fn parse_appinfo(path: &Path, known: &HashMap<u32, String>) -> HashMap<u32, AppInfo> {
    fn nul(input: &mut &[u8]) -> Option<String> {
        let end = input.iter().take(65_536).position(|b| *b == 0)?;
        let text = String::from_utf8_lossy(&input[..end]).into_owned();
        *input = &input[end + 1..];
        Some(text)
    }
    fn uint(input: &mut &[u8]) -> Option<u32> {
        Some(u32::from_le_bytes(input.split_off(..4)?.try_into().ok()?))
    }
    fn section(
        input: &mut &[u8],
        keys: Option<&[String]>,
        parent: &str,
        depth: u8,
        info: &mut AppInfo,
    ) -> Option<()> {
        if depth > 24 {
            return None;
        }
        while let Some((&token, rest)) = input.split_first() {
            *input = rest;
            if matches!(token, 8 | 11) {
                return Some(());
            }
            let key = match keys {
                Some(keys) => keys.get(uint(input)? as usize)?.clone(),
                None => nul(input)?,
            };
            match token {
                0 => section(input, keys, &key, depth + 1, info)?,
                1 => {
                    let text = nul(input)?;
                    if parent == "common" && key == "name" {
                        info.0 = Some(text);
                    } else if parent == "common" && key == "type" {
                        info.1 = Some(text);
                    }
                }
                2 | 3 | 4 | 6 => {
                    input.split_off(..4)?;
                }
                7 | 10 => {
                    input.split_off(..8)?;
                }
                _ => return None,
            }
        }
        Some(())
    }
    let mut result = HashMap::new();
    let Ok(file) = File::open(path) else {
        return result;
    };
    let Ok(metadata) = file.metadata() else {
        return result;
    };
    let mut end = metadata.len();
    let mut file = BufReader::new(file);
    let read_u32 = |file: &mut BufReader<File>| -> Option<u32> {
        let mut buffer = [0; 4];
        file.read_exact(&mut buffer).ok()?;
        Some(u32::from_le_bytes(buffer))
    };
    let magic = read_u32(&mut file);
    if !matches!(magic, Some(0x07564428 | 0x07564429)) || read_u32(&mut file).is_none() {
        return result;
    }
    let mut keys = None;
    let mut position = 8;
    if magic == Some(0x07564429) {
        let mut index_bytes = [0; 8];
        if file.read_exact(&mut index_bytes).is_err() {
            return result;
        }
        let index = u64::from_le_bytes(index_bytes);
        if index < 16 || index > end.saturating_sub(4) || file.seek(SeekFrom::Start(index)).is_err()
        {
            return result;
        }
        let Some(count) = read_u32(&mut file) else {
            return result;
        };
        if count > 250_000 {
            return result;
        }
        let mut table = Vec::with_capacity(count as usize);
        let mut key = Vec::new();
        for _ in 0..count {
            key.clear();
            if (&mut file).take(65_536).read_until(0, &mut key).is_err() || key.pop() != Some(0) {
                return result;
            }
            table.push(String::from_utf8_lossy(&key).into_owned());
        }
        if file.seek(SeekFrom::Start(16)).is_err() {
            return result;
        }
        keys = Some(table);
        end = index;
        position = 16;
    }
    let mut entry = Vec::new();
    while position + 8 <= end {
        let Some(id) = read_u32(&mut file).filter(|id| *id != 0) else {
            break;
        };
        let Some(size) = read_u32(&mut file).filter(|n| *n >= 60) else {
            break;
        };
        let end_entry = position + 8 + u64::from(size);
        if end_entry > end {
            break;
        }
        if known.contains_key(&id) && size <= 32 * 1024 * 1024 {
            entry.resize(size as usize, 0);
            if file.read_exact(&mut entry).is_err() {
                break;
            }
            let mut info = (None, None);
            if section(&mut &entry[60..], keys.as_deref(), "", 0, &mut info).is_some() {
                result.insert(id, info);
            }
        } else if file.seek(SeekFrom::Start(end_entry)).is_err() {
            break;
        }
        position = end_entry;
    }
    result
}
