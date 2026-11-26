# Steam Pulse Widget

Steam Pulse Widget is a Windows 11 x64 taskbar utility for Steam downloads, game activity, and sale alerts. It also tracks standalone applications and shows Steam friends' public activity.

## Install or run

- Install the per-user MSI and open Steam Pulse from the Start menu.
- For portable use, extract the entire ZIP and run the EXE with WebView2Loader.dll in the same folder.

The settings window and widget menu require the Microsoft Edge WebView2 Runtime. Double-click the tray icon or right-click an interactive widget to open Settings. Left-click the widget for recent launches, notifications, and friends. English is the default, with Russian and other languages available in Settings.

The Rust port uses the same local settings as the C# version at `%LOCALAPPDATA%\SteamPulse\settings.json`. Close the existing instance before switching. For the first upgrade from C# to Rust, use the MSI or the complete portable ZIP. The current updater expects a standalone EXE asset.

## Build on Windows x64

With Rust and the appropriate Windows linker installed, run:

```powershell
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

The GNU build uses the x86_64-pc-windows-gnu toolchain and MinGW-w64 binutils. Put dlltool.exe and windres.exe on PATH. For MSVC, install Visual Studio Build Tools with the C++ workload and Windows SDK.

The built program is `target/release/steam-pulse.exe`. Place `WebView2Loader.dll` from the x64 subdirectory of the webview2-com-sys build output alongside it when distributing the GNU build. Use Python 3.12 to build the MSI:

```powershell
python installer/build_msi.py target/release/steam-pulse.exe target/release/WebView2Loader.dll SteamPulse-v4.8.4-rust-win-x64.msi
```

The compiled settings interface is included. To edit or regenerate it, install Node.js and Python with `fonttools[woff]`. From settings-ui, run `npm ci`, `npm run setup`, `node fetch-icons.mjs`, and `npm run build`, then rebuild with Cargo.

Use `--smoke --verify-folder <folder>` to check startup with a separate profile. The app writes smoke.json and exits after about nine seconds. See the third-party notices and license files under assets.
