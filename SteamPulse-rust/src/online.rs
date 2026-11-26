use reqwest::blocking::{Client, Response};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    error::Error,
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};
use url::Url;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const REPO: &str = "alter413/SreamPulseWidget";
const RELEASE_HOSTS: &[&str] = &[
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
    "github-releases.githubusercontent.com",
];
const AVATARS: &[&str] = &[
    "avatars.akamai.steamstatic.com",
    "avatars.cloudflare.steamstatic.com",
    "avatars.fastly.steamstatic.com",
    "avatars.steamstatic.com",
];

fn trusted(url: &Url, hosts: &[&str]) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| hosts.contains(&host))
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .user_agent("SteamPulseWidget")
        .build()?)
}

fn bounded_body(response: Response, limit: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    response
        .error_for_status()?
        .take(limit + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        return Err("response exceeded size limit".into());
    }
    Ok(data)
}

#[cfg(windows)]
pub fn image(url: &str, avatar: bool) -> Option<image::RgbaImage> {
    const STORE_IMAGES: &[&str] = &[
        "cdn.cloudflare.steamstatic.com",
        "cdn.akamai.steamstatic.com",
        "shared.fastly.steamstatic.com",
        "shared.akamai.steamstatic.com",
        "shared.cloudflare.steamstatic.com",
        "cdn.fastly.steamstatic.com",
        "store.akamai.steamstatic.com",
        "store.cloudflare.steamstatic.com",
        "store.fastly.steamstatic.com",
    ];
    let url = Url::parse(url).ok()?;
    if !trusted(&url, if avatar { AVATARS } else { STORE_IMAGES }) {
        return None;
    }
    let body = bounded_body(client().ok()?.get(url).send().ok()?, 512 * 1024).ok()?;
    let reader = image::ImageReader::new(io::Cursor::new(&body))
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if w > 2048 || h > 2048 {
        return None;
    }
    Some(
        image::load_from_memory(&body)
            .ok()?
            .thumbnail_exact(32, 32)
            .to_rgba8(),
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Offer {
    pub id: u32,
    pub name: String,
    pub discount: i32,
    pub image: Option<String>,
}

pub fn offers(country: &str, locale: &str) -> Result<Vec<Offer>> {
    let language = match locale {
        "ru" => "russian",
        "es" => "spanish",
        "de" => "german",
        "fr" => "french",
        "pt" => "brazilian",
        "ja" => "japanese",
        "ko" => "koreana",
        "zh" => "schinese",
        "id" => "indonesian",
        _ => "english",
    };
    let response = client()?
        .get("https://store.steampowered.com/api/featuredcategories")
        .query(&[("cc", country), ("l", language)])
        .send()?;
    let value: Value = serde_json::from_slice(&bounded_body(response, 2 * 1024 * 1024)?)?;
    Ok(value["specials"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let discount = i32::try_from(row["discount_percent"].as_i64()?).ok()?;
            let id = u32::try_from(row["id"].as_u64()?).ok()?;
            (discount > 0 && id != 0).then(|| Offer {
                id,
                discount,
                name: row["name"].as_str().unwrap_or("Steam").to_owned(),
                image: row["small_capsule_image"].as_str().map(str::to_owned),
            })
        })
        .collect())
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Friend {
    #[serde(skip)]
    pub id: u64,
    pub name: String,
    pub online: bool,
    pub game: String,
    pub avatar: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Friends {
    pub available: bool,
    pub people: Vec<Friend>,
    pub playing: Vec<Friend>,
}

pub fn friends(account: u64) -> Friends {
    fn fetch(account: u64) -> Result<Friends> {
        let response = client()?
            .get(format!(
                "https://steamcommunity.com/profiles/{account}/friends/?l=english"
            ))
            .send()?;
        Ok(parse_friends(&String::from_utf8_lossy(&bounded_body(
            response,
            5 * 1024 * 1024,
        )?)))
    }
    if account == 0 {
        Friends::default()
    } else {
        fetch(account).unwrap_or_default()
    }
}

pub fn parse_friends(html: &str) -> Friends {
    static LIST: LazyLock<Selector> = LazyLock::new(|| Selector::parse("#friends_list").unwrap());
    static ROW: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse("div.friend_block_v2").unwrap());
    static CONTENT: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse("div.friend_block_content").unwrap());
    static DETAIL: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse("span.friend_small_text").unwrap());
    static IMG: LazyLock<Selector> = LazyLock::new(|| Selector::parse("img[src]").unwrap());
    fn lines(element: ElementRef<'_>) -> Vec<String> {
        let mut output = String::new();
        for node in element.descendants().skip(1) {
            if node.value().as_element().is_some_and(|e| e.name() == "br") {
                output.push('\n');
            }
            if let Some(text) = node.value().as_text() {
                output.extend(
                    text.chars()
                        .map(|c| if c.is_whitespace() { ' ' } else { c }),
                );
            }
        }
        output
            .split('\n')
            .map(|line| {
                line.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(128)
                    .collect::<String>()
            })
            .filter(|line| !line.is_empty())
            .collect()
    }
    let doc = Html::parse_document(html);
    let Some(list) = doc.select(&LIST).next() else {
        return Friends::default();
    };
    let mut result = Friends {
        available: true,
        ..Friends::default()
    };
    let mut seen = HashSet::new();
    for row in list.select(&ROW) {
        let class = |name| row.value().classes().any(|c| c.eq_ignore_ascii_case(name));
        let playing = class("in-game") || class("ingame");
        let Some(content) = row.select(&CONTENT).next() else {
            continue;
        };
        let mut content_lines = lines(content).into_iter();
        let Some(name) = content_lines.next() else {
            continue;
        };
        let game = if playing {
            content
                .select(&DETAIL)
                .next()
                .into_iter()
                .flat_map(lines)
                .chain(content_lines)
                .find(|line| {
                    !line.eq_ignore_ascii_case("in-game") && !line.eq_ignore_ascii_case("in game")
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        let avatar = row
            .select(&IMG)
            .filter_map(|image| Url::parse(image.value().attr("src")?).ok())
            .find(|url| trusted(url, AVATARS))
            .map(Into::into);
        let id = row
            .value()
            .attr("data-steamid")
            .and_then(|id| id.parse().ok())
            .unwrap_or(0);
        let row = Friend {
            id,
            name,
            online: playing || class("online"),
            game,
            avatar,
        };
        if !row.game.is_empty()
            && !result.playing.iter().any(|other| {
                other.name == row.name && other.game == row.game && other.avatar == row.avatar
            })
        {
            result.playing.push(row.clone());
        }
        if id != 0 && seen.insert(id) {
            result.people.push(row);
        }
    }
    result.playing.sort_by_key(|f| f.name.to_lowercase());
    result
        .people
        .sort_by_key(|f| (!f.online, f.name.to_lowercase()));
    result
}

#[derive(Clone, Debug)]
pub struct Update {
    pub key: &'static str,
    pub version: Option<String>,
    pub path: Option<PathBuf>,
}

impl Update {
    pub fn status(key: &'static str) -> Self {
        Self {
            key,
            version: None,
            path: None,
        }
    }
}

#[derive(Debug)]
pub struct ReleaseAsset {
    pub version: String,
    url: Url,
    size: u64,
    digest: String,
}

fn version(tag: &str) -> Option<[u32; 3]> {
    if !(3..=30).contains(&tag.len()) {
        return None;
    }
    let tag = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    let mut numbers = tag.split('.');
    let read = |s: &str| {
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())
            .flatten()
    };
    let v = [
        read(numbers.next()?)?,
        read(numbers.next()?)?,
        numbers.next().map_or(Some(0), read)?,
    ];
    numbers.next().is_none().then_some(v)
}

pub fn select_release(
    release: &Value,
    current: [u32; 3],
    preview: bool,
) -> (Update, Option<ReleaseAsset>) {
    let tag = release["tag_name"].as_str().unwrap_or_default();
    let Some(v) =
        version(tag).filter(|_| release["draft"] != true && release["prerelease"] != true)
    else {
        return (Update::status("updateNoRelease"), None);
    };
    if v < current || v == current && !preview {
        return (Update::status("updateCurrent"), None);
    }
    let short = format!("{}.{}", v[0], v[1]);
    let label = format!("{short}.{}", v[2]);
    let names = [&label, &short]
        .into_iter()
        .flat_map(|v| {
            ["-win-x64.exe", "-release.exe", ".exe"].map(|ext| format!("SteamPulse-v{v}{ext}"))
        })
        .chain(std::iter::once("SteamPulse.exe".to_owned()));
    let item = names.into_iter().find_map(|name| {
        release["assets"].as_array()?.iter().find_map(|asset| {
            let published_name = asset["name"].as_str()?;
            if !published_name.eq_ignore_ascii_case(&name) || asset["state"] != "uploaded" {
                return None;
            }
            let size = asset["size"]
                .as_u64()
                .filter(|&n| (1..=250 * 1024 * 1024).contains(&n))?;
            let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?;
            if digest.len() != 64 || !digest.bytes().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            let url = Url::parse(asset["browser_download_url"].as_str()?).ok()?;
            let expected =
                format!("https://github.com/{REPO}/releases/download/{tag}/{published_name}");
            if !url.as_str().eq_ignore_ascii_case(&expected) {
                return None;
            }
            Some(ReleaseAsset {
                version: label.clone(),
                url,
                size,
                digest: digest.to_owned(),
            })
        })
    });
    (
        Update {
            key: if item.is_some() {
                "updateDownloading"
            } else {
                "updateNoAsset"
            },
            version: Some(label),
            path: None,
        },
        item,
    )
}

pub fn update(folder: &Path, progress: impl Fn(Update)) -> Update {
    fn run(folder: &Path, progress: impl Fn(Update)) -> Result<Update> {
        let http = Client::builder()
            .user_agent("SteamPulseWidget")
            .timeout(Duration::from_secs(720))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let response = http
            .get(format!(
                "https://api.github.com/repos/{REPO}/releases/latest"
            ))
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(30))
            .send()?;
        if response.status().as_u16() == 404 {
            return Ok(Update::status("updateNoRelease"));
        }
        let release = serde_json::from_slice(&bounded_body(response, 2 * 1024 * 1024)?)?;
        let (status, asset) =
            select_release(&release, version(env!("CARGO_PKG_VERSION")).unwrap(), false);
        let Some(asset) = asset else {
            return Ok(status);
        };
        progress(status);
        let path = download(&http, &asset, folder)?;
        Ok(Update {
            key: "updateReady",
            version: Some(asset.version),
            path: Some(path),
        })
    }
    run(folder, progress).unwrap_or_else(|_| Update::status("updateFailed"))
}

fn transfer(input: impl Read, mut output: impl Write, asset: &ReleaseAsset) -> Result<()> {
    let mut input = input.take(asset.size + 1);
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut bytes = [0; 65_536];
    loop {
        let n = input.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > asset.size {
            return Err("update exceeds published size".into());
        }
        hash.update(&bytes[..n]);
        output.write_all(&bytes[..n])?;
    }
    if total != asset.size || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&asset.digest)
    {
        return Err("update size or SHA-256 mismatch".into());
    }
    Ok(())
}

fn download(http: &Client, asset: &ReleaseAsset, folder: &Path) -> Result<PathBuf> {
    fs::create_dir_all(folder)?;
    let final_path = folder.join(format!("SteamPulse-v{}-win-x64.exe", asset.version));
    if let Ok(existing) = File::open(&final_path)
        && transfer(existing, io::sink(), asset).is_ok()
    {
        return Ok(final_path);
    }
    let mut url = asset.url.clone();
    for _ in 0..5 {
        let response = http
            .get(url.clone())
            .header("Accept", "application/octet-stream")
            .send()?;
        if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            url = url.join(
                response
                    .headers()
                    .get("Location")
                    .ok_or("missing redirect")?
                    .to_str()?,
            )?;
            if !trusted(&url, RELEASE_HOSTS) {
                return Err("untrusted update redirect".into());
            }
            continue;
        }
        if !response.status().is_success()
            || response.content_length().is_some_and(|n| n != asset.size)
        {
            return Err("unexpected update response".into());
        }
        let mut file = tempfile::Builder::new()
            .suffix(".part")
            .tempfile_in(folder)?;
        transfer(response, &mut file, asset)?;
        file.as_file().sync_all()?;
        file.persist(&final_path).map_err(|error| error.error)?;
        return Ok(final_path);
    }
    Err("too many update redirects".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_transfer_rejects_tampering_and_size_mismatches() {
        let payload = b"Steam Pulse release fixture";
        let mut asset = ReleaseAsset {
            version: "9.0.0".to_owned(),
            url: Url::parse("https://github.com/").unwrap(),
            size: payload.len() as u64,
            digest: format!("{:x}", Sha256::digest(payload)),
        };
        let mut stored = Vec::new();
        transfer(payload.as_slice(), &mut stored, &asset).unwrap();
        assert_eq!(stored, payload);
        stored[0] ^= 1;
        assert!(transfer(stored.as_slice(), io::sink(), &asset).is_err());
        asset.size -= 1;
        assert!(transfer(payload.as_slice(), io::sink(), &asset).is_err());
        asset.size += 2;
        assert!(transfer(payload.as_slice(), io::sink(), &asset).is_err());
    }
}
