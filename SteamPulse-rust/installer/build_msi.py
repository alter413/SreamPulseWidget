"""Build the per-user x64 MSI with Python 3.12 on Windows."""

import argparse
import msilib
from pathlib import Path
import tempfile
import tomllib
import uuid
from msilib import schema, sequence

PROJECT = Path(__file__).resolve().parents[1]
APP_ID = "SteamPulse.Desktop"
UPGRADE_ID = uuid.UUID("84e57b32-37b8-4c60-a99e-09b43f753e0f")


def guid(name: str) -> str:
    return "{" + str(uuid.uuid5(UPGRADE_ID, name)).upper() + "}"


def build(exe: Path, loader: Path, output: Path) -> None:
    version = tomllib.loads((PROJECT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    product = guid(version)
    output.parent.mkdir(parents=True, exist_ok=True)
    db = msilib.init_database(str(output), schema, "Steam Pulse", product, version, "Steam Pulse")
    msilib.add_tables(db, sequence)
    si = db.GetSummaryInformation(10)
    si.SetProperty(msilib.PID_TEMPLATE, "x64;1033")
    si.SetProperty(msilib.PID_PAGECOUNT, 500)
    si.SetProperty(msilib.PID_WORDCOUNT, 2 | 8)  # Embedded cabinet, per-user package without elevation.
    si.SetProperty(msilib.PID_APPNAME, "Windows Installer")
    si.Persist()
    def add(name, rows):
        msilib.add_data(db, name, rows)
    add("Property", [
        ("INSTALLLEVEL", "1"), ("ARPPRODUCTICON", "Pulse.ico"), ("ARPNOMODIFY", "1"),
        ("UpgradeCode", "{" + str(UPGRADE_ID).upper() + "}"),
        ("SecureCustomProperties", "OLDSTEAMPULSE"),
    ])
    add("LaunchCondition", [
        ("VersionNT64", "Steam Pulse requires 64-bit Windows."),
        ("NOT ALLUSERS", "Install Steam Pulse for the current user."),
    ])
    add("Directory", [
        ("TARGETDIR", None, "SourceDir"),
        ("LocalAppDataFolder", "TARGETDIR", "."),
        ("ProgramsRoot", "LocalAppDataFolder", "Programs"),
        ("INSTALLDIR", "ProgramsRoot", "STMPULSE|Steam Pulse"),
        ("ProgramMenuFolder", "TARGETDIR", "."),
    ])
    # Each file has its own 64-bit component so Windows can repair it independently.
    add("Component", [
        ("PulseExe", guid("exe"), "INSTALLDIR", 256, None, "PulseExe"),
        ("PulseLoader", guid("loader"), "INSTALLDIR", 256, None, "PulseLoader"),
        ("StartShortcut", guid("start-menu"), "ProgramMenuFolder", 4, None, "StartKey"),
    ])
    add("Feature", [("Desktop", None, "Steam Pulse", "Steam taskbar widget", 1, 1, "INSTALLDIR", 0)])
    add("FeatureComponents", [("Desktop", name) for name in ("PulseExe", "PulseLoader", "StartShortcut")])
    add("File", [
        ("PulseExe", "PulseExe", "STPULSE.EXE|SteamPulse.exe", exe.stat().st_size, version, None, 16384, 1),
        ("PulseLoader", "PulseLoader", "WVLOADER.DLL|WebView2Loader.dll", loader.stat().st_size, None, None, 16384, 2),
    ])
    add("Registry", [("StartKey", 1, r"Software\SteamPulse\Installer", "Installed", "#1", "StartShortcut")])
    add("Icon", [("Pulse.ico", msilib.Binary(str(PROJECT / "assets" / "Pulse.ico")))])
    add("Shortcut", [
        ("StartShortcut", "ProgramMenuFolder", "STMPULSE|Steam Pulse", "StartShortcut",
         "[#PulseExe]", "--settings", "Steam Pulse", None, "Pulse.ico", 0, 1, "INSTALLDIR"),
    ])
    db.OpenView(
        "CREATE TABLE `MsiShortcutProperty` (`MsiShortcutProperty` CHAR(72) NOT NULL, "
        "`Shortcut_` CHAR(72) NOT NULL, `PropertyKey` CHAR(255) NOT NULL, `PropVariantValue` CHAR(0) "
        "PRIMARY KEY `MsiShortcutProperty`)"
    ).Execute(None)
    add("MsiShortcutProperty", [("PulseAppId", "StartShortcut", "System.AppUserModel.ID", APP_ID)])
    add("Upgrade", [
        ("{" + str(UPGRADE_ID).upper() + "}", "0.0.0", version, None, 256, None, "OLDSTEAMPULSE"),
    ])
    with tempfile.TemporaryDirectory() as temporary:
        cab = Path(temporary) / "SteamPulse.cab"
        msilib.FCICreate(str(cab), [(str(exe), "PulseExe"), (str(loader), "PulseLoader")])
        add("Media", [(1, 2, None, "#SteamPulse.cab", None, None)])
        msilib.add_stream(db, "SteamPulse.cab", str(cab))
        db.Commit()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("exe", type=Path)
    parser.add_argument("loader", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    build(args.exe.resolve(), args.loader.resolve(), args.output.resolve())
