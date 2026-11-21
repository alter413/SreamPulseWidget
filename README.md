# Steam Pulse

A Windows x64 tray utility that shows Steam activity in a free space on the taskbar.

## Run

Download the portable Windows x64 EXE from the release and open it on Windows 11. Use the Steam Pulse icon in the notification area to open settings, show or hide the widget, or lock its position.

Version 3.9.0 writes no application diagnostic or visibility logs. It still recovers the widget when the taskbar changes.

## Build

Run the commands in PowerShell from the extracted source directory. The build uses the .NET 10 SDK, Node.js 20 or later, Python 3 and the WebView2 SDK package from NuGet.

```powershell
python -m pip install "fonttools[woff]"
Set-Location settings-ui
npm ci
npm run setup
node fetch-icons.mjs
npm run build
Set-Location ..
dotnet publish SteamPulse -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true -p:EnableCompressionInSingleFile=true -o publish
```

The settings window uses the Microsoft Edge WebView2 Runtime. Windows 11 commonly ships it. The widget is native WPF and works independently of whether the settings window is open.

Settings and the WebView2 data folder are stored under `%LOCALAPPDATA%\SteamPulse`. The optional startup toggle registers the current EXE path for the current Windows user. Keep the executable in the same location while that toggle is enabled.

## Steam and the taskbar

Steam Pulse reads local Steam installation paths, manifests, running-app information and download logs. The ignore list is keyed by Steam AppID. It includes installed titles and titles known to the local client, which can differ from the complete library attached to an account. Store notifications use the public Steam storefront. The library shows Steam titles and apps added via EXE in the same search results. EXE entries have individual visibility switches and can be renamed or removed. Running games are matched by full path in the current Windows session. For versioned installs such as Discord, a matching EXE in an app-version subfolder of the same installation remains tracked after updates. CPU and physical RAM monitoring are optional. The Friends tab reads the visible Steam Community friends page and paginates current games; the compact widget rotates one friend and shows a count. No Steam cookies or API key are used.

The widget refreshes its position when Explorer recreates the taskbar and corrects its stacking if the taskbar covers it. The Position section pins the widget to a chosen display independently of where the active window is. If that display is unplugged, the widget uses the main display until it returns. The widget finds free space between the taskbar controls and applies the configured horizontal position within the nearest usable area. With click-through disabled, drag the widget directly. If no safe gap exists or the taskbar is temporarily unavailable, a compact click-through badge stays at the taskbar edge. Hide widget in the tray is the visibility control, and Windows Task Manager can stop the app. The Appearance section controls the white widget border and Floating widget mode. The widget and settings window have independent glass profiles. Shared glass values from older versions are copied to both profiles on first load. The footer shows the build version. The tray menu follows the selected theme. Freeze widget locks mouse dragging until you click Unfreeze widget.

The Vaso-based renderer uses pixels under the app windows as a live backdrop. Windows excludes those windows from capture while glass is on. The pixel buffers remain local and are used for rendering. Settings glass is enabled in fresh settings, while widget glass and gradients start off. During window motion, the settings backdrop is sampled more frequently and JPEG frames are prepared off the UI thread.

## Localization

English is the default. The General section offers system-language selection as well as 13 explicit language choices. Both the native widget and the settings interface use `SteamPulse/Locales.json`. Add the same keys to every translation and rebuild the embedded settings page after editing the catalog.

## Third-party notices

See `SteamPulse/ThirdPartyNotices.txt` and `SteamPulse/Vaso-LICENSE.txt`. The source archive fetches the upstream React Bits and Motion-Primitives components at build time. The icon subset is also generated at build time.

[UIcons by Flaticon](https://www.flaticon.com/uicons)
