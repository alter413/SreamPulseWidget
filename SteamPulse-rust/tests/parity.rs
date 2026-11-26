use chrono::NaiveDateTime;
use serde_json::json;
use std::{collections::HashMap, fs, path::Path};
use steam_pulse::{
    layout,
    model::{self, Preferences},
    online,
    steam::{self, ContentSample, Game, SteamLog, Vdf},
};

#[test]
fn legacy_settings_preserve_hidden_state_and_migrate_optics() {
    let p =
        Preferences::load_json(r#"{"Widget":false,"FormatVersion":4,"GlassDepth":-2,"Width":970}"#)
            .unwrap();
    assert!(!p.flag("Widget"));
    assert_eq!(p.number("SettingsGlassDepth"), -2.0);
    assert_eq!(p.number("Width"), 550.0);
}

#[test]
fn vdf_reads_old_and_new_libraries_and_escaped_paths() {
    let vdf = Vdf::parse(r#" "libraryfolders" { "0" {"path" "C:\\Steam"} "1" "D:\\Games" } "#);
    let mut libraries = vdf.child("LIBRARYFOLDERS").unwrap().libraries();
    libraries.sort();
    assert_eq!(libraries, [r"C:\Steam", r"D:\Games"]);
}

#[test]
fn steam_download_stops_when_the_scheduler_finishes() {
    let s = ContentSample::default().parse("[2026-11-25 10:00:00] AppID 10 update started : download 100/1000\n[2026-11-25 10:00:05] Current download rate: 1 Kbps\n");
    assert_eq!(s.id, 10);
    assert_eq!(s.speed, 125.0);
    assert_eq!(
        s.parse("[2026-11-25 10:00:06] AppID 10 scheduler finished\n")
            .id,
        0
    );
}

#[test]
fn layout_uses_nearest_adequate_gap_without_intersecting_buttons() {
    let choose = |percent| {
        layout::pick(
            0..1000,
            &[0..50, 300..700, 900..1000],
            layout::Placement {
                width: 280,
                minimum: 150,
                badge: 32,
                gutter: 0,
                align: 2,
                offset: 0,
                percent: Some(percent),
            },
        )
        .unwrap()
    };
    assert_eq!(choose(80.).bounds, 700..900);
    assert_eq!(choose(72.).bounds, 700..850);
    assert_eq!(choose(87.).bounds, 750..900);
}

#[test]
fn layout_follows_multiple_points_inside_the_same_gap() {
    let choose = |percent| {
        layout::pick(
            0..1000,
            std::slice::from_ref(&(400..630)),
            layout::Placement {
                width: 160,
                minimum: 130,
                badge: 36,
                gutter: 5,
                align: 2,
                offset: 14,
                percent: Some(percent),
            },
        )
        .unwrap()
    };
    for (percent, expected) in [(12., 40..200), (19., 110..270), (26., 180..340)] {
        assert_eq!(choose(percent).bounds, expected);
        assert_eq!(choose(percent).safe, 5..395);
    }
}

#[test]
fn watchdog_uses_strict_thresholds() {
    assert!(!layout::should_restart(450_000_000, false));
    assert!(layout::should_restart(450_000_001, false));
    assert!(!layout::should_restart(400_000_000, true));
    assert!(layout::should_restart(400_000_001, true));
}

#[cfg(windows)]
#[test]
fn system_metrics_keep_the_webview_message_field_names() {
    let load = steam_pulse::windows_data::Load {
        cpu: Some(23),
        memory_percent: Some(55),
        memory_gib: Some(8.5),
    };
    assert_eq!(
        serde_json::to_value(load).unwrap(),
        json!({"cpu":23,"memoryPercent":55,"memoryGiB":8.5})
    );
}

#[test]
fn settings_validation_and_replacement_preserve_legacy_data() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Preferences::load_json(r##"{"FormatVersion":5,"Widget":false,"Background":"#ABCDEF","IgnoredGames":[420],"LocalGames":[{"Id":"x","Name":"App","Path":"C:/Sample/App.exe","Ignored":true}]}"##).unwrap();
    assert!(!p.edit("LocalGames", &json!([]), &[]));
    assert!(!p.edit("Background", &json!("red"), &[]));
    assert!(!p.edit("MonitorName", &json!("Unknown"), &["Display 1"]));
    assert!(p.edit("MonitorName", &json!("Display 1"), &["Display 1"]));
    p.save(dir.path()).unwrap();
    assert!(p.edit("Opacity", &json!(0.01), &[]));
    p.save(dir.path()).unwrap();
    let reread = Preferences::load(dir.path());
    assert!(!reread.flag("Widget"));
    assert_eq!(reread.text("Background"), "#ABCDEF");
    assert_eq!(reread.text("MonitorName"), "Display 1");
    assert_eq!(reread.number("Opacity"), 0.1);
    assert!(reread.ignored(420));
    assert!(reread.local_games()[0].ignored);
}

#[test]
fn versioned_exes_match_stable_launcher_but_not_an_unrelated_install() {
    let root = tempfile::tempdir().unwrap();
    for dirname in ["app-1.0.0", "app-1.20.0", "app-2-invalid", "unrelated"] {
        let path = root.path().join(dirname);
        fs::create_dir(&path).unwrap();
        fs::write(path.join("Client.exe"), []).unwrap();
    }
    let saved = root.path().join("Client.exe");
    let latest = root.path().join("app-1.20.0/Client.exe");
    assert!(model::matches_executable(&latest, &saved));
    assert!(!model::matches_executable(
        &root.path().join("unrelated/Client.exe"),
        &saved
    ));
    assert_eq!(model::resolve_launch_path(&saved), Some(latest.clone()));
    fs::write(&saved, []).unwrap();
    assert_eq!(model::resolve_launch_path(&latest), Some(saved));
    assert!(!model::standalone(Path::new(
        "STEAM/steamapps/common/Game.exe"
    )));
}

#[test]
fn steam_log_retries_incomplete_tail_and_handles_install_phase() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("content_log.txt");
    let started = "[2026-11-25 10:00:00] AppID 10 update started : download 100/1000\n";
    fs::write(&path, started).unwrap();
    let mut log = SteamLog::default();
    log.read(&path);
    let game = Game {
        id: 10,
        name: "Fixture".into(),
        art: None,
    };
    let time = NaiveDateTime::parse_from_str("2026-11-25 10:00:10", "%Y-%m-%d %H:%M:%S").unwrap();
    let progress = "[2026-11-25 10:00:09] AppID 10 update progress : download 1000/1000";
    fs::write(&path, format!("{started}{progress}")).unwrap();
    log.read(&path);
    assert_eq!(log.download(game.clone(), None, time).percent, Some(10.0));
    fs::write(&path, format!("{started}{progress}\n")).unwrap();
    log.read(&path);
    let current = log.download(game, None, time);
    assert_eq!(current.percent, Some(100.0));
    assert!(current.installing);
    assert_eq!(current.remaining, None);
}

#[test]
fn library_reads_manifest_and_indexed_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("steamapps")).unwrap();
    fs::create_dir(root.join("appcache")).unwrap();
    fs::write(
        root.join("steamapps/appmanifest_42.acf"),
        "\"AppState\" { \"name\" \"Installed title\" }",
    )
    .unwrap();
    let mut known = HashMap::new();
    known.insert(10, "Registry label".into());
    let mut binary = Vec::new();
    binary.extend(0x07564429u32.to_le_bytes());
    binary.extend([0; 12]);
    let mut kv = vec![0; 60];
    kv.push(0);
    kv.extend(0u32.to_le_bytes());
    kv.push(1);
    kv.extend(1u32.to_le_bytes());
    kv.extend(b"Cached title\0");
    kv.push(1);
    kv.extend(2u32.to_le_bytes());
    kv.extend(b"game\0");
    kv.extend([8, 8]);
    binary.extend(10u32.to_le_bytes());
    binary.extend((kv.len() as u32).to_le_bytes());
    binary.extend(kv);
    binary.extend(0u32.to_le_bytes());
    let index = binary.len() as u64;
    binary[8..16].copy_from_slice(&index.to_le_bytes());
    binary.extend(3u32.to_le_bytes());
    binary.extend(b"common\0name\0type\0");
    fs::write(root.join("appcache/appinfo.vdf"), binary).unwrap();
    let list = steam::library(root, known, 0);
    assert_eq!(list.len(), 2);
    assert_eq!((&*list[0].name, list[0].installed), ("Cached title", false));
    assert_eq!(
        (&*list[1].name, list[1].installed),
        ("Installed title", true)
    );
}

#[test]
fn friends_extract_public_presence_and_drop_untrusted_or_duplicate_entries() {
    let state = online::parse_friends(
        r#"
      <div id="friends_list">
        <div class="friend_block_v2 offline" data-steamid="2">
            <div class="friend_block_content">Someone offline</div>
        </div>
        <div class="friend_block_v2 in-game" data-steamid="1">
            <img src="https://bad.example.test/avatar.png">
            <img src="https://avatars.fastly.steamstatic.com/fixture.jpg">
            <div class="friend_block_content">Player &amp; One<br><span class="friend_small_text">In-Game<br>Portal 2</span></div>
        </div>
        <div class="friend_block_v2 online" data-steamid="1">
            <div class="friend_block_content">Duplicate</div>
        </div>
      </div>
    "#,
    );
    assert!(state.available);
    assert_eq!(state.people.len(), 2);
    assert_eq!(state.people[0].name, "Player & One");
    assert_eq!(state.playing[0].game, "Portal 2");
    assert_eq!(
        state.playing[0].avatar.as_deref(),
        Some("https://avatars.fastly.steamstatic.com/fixture.jpg")
    );
    assert!(!online::parse_friends("<h1>Private</h1>").available);
}

#[test]
fn updater_accepts_only_newer_stable_assets_with_expected_digest_and_url() {
    let digest = "ab".repeat(32);
    let mut release = json!({"tag_name":"v4.9.0","assets":[{"name":"SteamPulse-v4.9.0-win-x64.exe",
        "state":"uploaded","size":42,"digest":format!("sha256:{digest}"),
        "browser_download_url":"https://github.com/alter413/SreamPulseWidget/releases/download/v4.9.0/SteamPulse-v4.9.0-win-x64.exe"}]});
    assert_eq!(
        online::select_release(&release, [4, 8, 0], false)
            .1
            .unwrap()
            .version,
        "4.9.0"
    );
    assert!(
        online::select_release(&release, [4, 9, 0], false)
            .1
            .is_none()
    );
    release["prerelease"] = json!(true);
    assert_eq!(
        online::select_release(&release, [4, 8, 0], false).0.key,
        "updateNoRelease"
    );
    release["prerelease"] = json!(false);
    release["assets"][0]["digest"] = json!("sha256:invalid");
    assert_eq!(
        online::select_release(&release, [4, 8, 0], false).0.key,
        "updateNoAsset"
    );
    release["assets"][0]["digest"] = json!(format!("sha256:{digest}"));
    release["assets"][0]["browser_download_url"] = json!("https://example.com/fake.exe");
    assert!(
        online::select_release(&release, [4, 8, 0], false)
            .1
            .is_none()
    );
}
