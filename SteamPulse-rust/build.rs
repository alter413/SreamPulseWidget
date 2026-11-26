use std::{env, fs, io, path::PathBuf, process::Command};

const ICON: &[u8] = include_bytes!("assets/Pulse.ico");
const MANIFEST: &[u8] = include_bytes!("assets/app.manifest");

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=assets/Pulse.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let resources = out.join("SteamPulse.res");
    fs::write(&resources, make_resources())?;
    let input = if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        let object = out.join("SteamPulseResources.o");
        let target = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("x86_64") => "pe-x86-64",
            Ok("x86") => "pe-i386",
            _ => {
                return Err(io::Error::other(
                    "unsupported GNU Windows resource architecture",
                ));
            }
        };
        let windres = env::var_os("WINDRES").unwrap_or_else(|| "windres".into());
        let status = Command::new(windres)
            .args([
                "--input-format=res",
                "--output-format=coff",
                "--target",
                target,
            ])
            .arg(&resources)
            .arg(&object)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(
                "windres could not compile the Steam Pulse resources",
            ));
        }
        object
    } else {
        resources
    };
    println!("cargo:rustc-link-arg-bin=steam-pulse={}", input.display());
    Ok(())
}

fn make_resources() -> Vec<u8> {
    let mut resources = Vec::new();
    append_resource(&mut resources, 0, 0, &[]);
    let count = u16::from_le_bytes(ICON[4..6].try_into().expect("ICO header"));
    let mut group = ICON[..6].to_vec();
    for (i, entry) in ICON[6..]
        .as_chunks::<16>()
        .0
        .iter()
        .take(count as usize)
        .enumerate()
    {
        let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
        let start = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
        let id = 102 + i as u16;
        append_resource(&mut resources, 3, id, &ICON[start..start + len]);
        group.extend_from_slice(&entry[..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    append_resource(&mut resources, 14, 101, &group);
    append_resource(&mut resources, 24, 1, MANIFEST);
    append_resource(&mut resources, 16, 1, &version_resource());
    resources
}

fn append_resource(output: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
    for value in [data.len() as u32, 32] {
        output.extend_from_slice(&value.to_le_bytes());
    }
    for value in [
        0xffff,
        kind,
        0xffff,
        id,
        0,
        0,
        if kind == 0 { 0 } else { 0x1030 },
        0,
    ] {
        output.extend_from_slice(&value.to_le_bytes());
    }
    output.extend_from_slice(&[0; 8]);
    output.extend_from_slice(data);
    align(output);
}

fn align(output: &mut Vec<u8>) {
    output.resize(output.len().next_multiple_of(4), 0);
}

fn version_resource() -> Vec<u8> {
    fn wide(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    }
    fn block(key: &str, value: &[u8], text: bool, children: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = vec![0, 0];
        let length = if text { value.len() / 2 } else { value.len() };
        bytes.extend_from_slice(&(length as u16).to_le_bytes());
        bytes.extend_from_slice(&u16::from(text).to_le_bytes());
        bytes.extend_from_slice(&wide(key));
        align(&mut bytes);
        bytes.extend_from_slice(value);
        for child in children {
            align(&mut bytes);
            bytes.extend_from_slice(child);
        }
        let length = bytes.len() as u16;
        bytes[..2].copy_from_slice(&length.to_le_bytes());
        bytes
    }
    let ver = env!("CARGO_PKG_VERSION");
    let nums: Vec<u32> = ver
        .split('.')
        .map(|s| s.parse::<u32>().unwrap_or(0))
        .collect();
    let ms = nums[0] << 16 | nums[1];
    let ls = nums[2] << 16;
    let fixed: Vec<_> = [
        0xFEEF04BDu32,
        0x00010000,
        ms,
        ls,
        ms,
        ls,
        0x3f,
        0,
        0x40004,
        1,
        0,
        0,
        0,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect();
    let strings = [
        ("FileDescription", "Steam Pulse"),
        ("ProductName", "Steam Pulse"),
        ("FileVersion", ver),
        ("ProductVersion", ver),
        ("InternalName", "SteamPulse"),
        ("OriginalFilename", "SteamPulse.exe"),
    ]
    .map(|(k, v)| block(k, &wide(v), true, &[]));
    let string_table = block("040904B0", &[], true, &strings);
    let strings = block("StringFileInfo", &[], true, &[string_table]);
    let translation = block("Translation", &[0x09, 0x04, 0xb0, 0x04], false, &[]);
    let vars = block("VarFileInfo", &[], true, &[translation]);
    block("VS_VERSION_INFO", &fixed, false, &[strings, vars])
}
