using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text.Json;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Runtime.InteropServices;
using Microsoft.Web.WebView2.Core;
using Microsoft.Web.WebView2.Wpf;
using static SteamPulse.Localization;
using Wf = System.Windows.Forms;

namespace SteamPulse;

internal sealed class SettingsWindow : Window
{
    private static readonly JsonSerializerOptions Json = new(JsonSerializerDefaults.Web);
    private static readonly HashSet<string> Editable = new(("""
        Widget Downloads Games Icon Name Speed RemainingTime Percent Bar Network Bits Discounts DiscountCard
        Glass SettingsGlass Gradient AirMode WidgetBorder LightTheme ClickThrough MonitorName MinDiscount Opacity Width Radius
        MonitorSystem MonitorCpu MonitorRam RamGiB MetricsInterval Friends FriendCycle
        GlassTint SettingsTint GlassDepth GlassBlur GlassDispersion GlassLight Angle Shine
        SettingsGlassDepth SettingsGlassBlur SettingsGlassDispersion SettingsGlassLight SettingsShine Background BackgroundEnd
        Accent AccentEnd Text Country Language HorizontalPosition
        """).Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries), StringComparer.Ordinal);

    private readonly Preferences preferences;
    private readonly Action changed;
    private readonly Action reset;
    private readonly Func<SteamStatus> live;
    private readonly bool verify;
    private readonly WebView2 web = new();
    private readonly GlassBackdrop backdrop;
    private bool ready;
    private bool disposed;
    private bool libraryLoading;
    private bool libraryRequested;
    private string? libraryRoot;
    private LibraryGame[]? library;
    private SteamStatus? lastStatus;
    private const string GlassEndpoint = "https://glass.steampulse.invalid/frame/";
    private byte[]? glassFrame;
    private long glassSequence;
    private bool glassFrameSent;
    private bool startupEnabled;
    private bool saveFailed;
    private DisplayChoice[]? displays;
    private sealed record DisplayChoice(string Name, bool Primary, int Width, int Height);

    public SettingsWindow(Preferences preferences, Action changed, Action reset, Func<SteamStatus> live, bool verify = false)
    {
        this.preferences = preferences;
        this.changed = changed;
        this.reset = reset;
        this.live = live;
        this.verify = verify;
        startupEnabled = !verify && WindowsStartup.Enabled();
        Title = "Steam Pulse · " + T("settings");
        ShowInTaskbar = verify;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        Width = 910;
        Height = Math.Min(760, SystemParameters.WorkArea.Height);
        MinWidth = 650;
        MinHeight = 485;
        Background = new SolidColorBrush(Color.FromRgb(17, 18, 22));
        Content = web;
        backdrop = new GlassBackdrop(this, contentBounds: GlassBounds, presentEncoded: PresentGlass);
        Closed += (_, _) => { disposed = true; backdrop.Dispose(); web.Dispose(); };
        Loaded += async (_, _) => await Initialize();
        DpiChanged += (_, _) => UpdateGlass();
    }

    private async Task Initialize()
    {
        try
        {
            var environment = await CoreWebView2Environment.CreateAsync(userDataFolder: Path.Combine(Preferences.Folder, "WebView"));
            if (disposed) return;
            await web.EnsureCoreWebView2Async(environment);
            if (disposed) return;
            var core = web.CoreWebView2;
            core.Settings.AreDefaultContextMenusEnabled = false;
            core.Settings.AreDevToolsEnabled = false;
            core.Settings.AreBrowserAcceleratorKeysEnabled = false;
            core.Settings.AreHostObjectsAllowed = false;
            core.Settings.IsStatusBarEnabled = false;
            core.Settings.IsZoomControlEnabled = false;
            core.Settings.AreDefaultScriptDialogsEnabled = false;
            core.NewWindowRequested += (_, e) => e.Handled = true;
            core.NavigationStarting += (_, e) => e.Cancel = ready || e.IsUserInitiated ||
                !(e.Uri == "about:blank" || e.Uri.StartsWith("data:text/html", StringComparison.Ordinal));
            core.PermissionRequested += (_, e) => e.State = CoreWebView2PermissionState.Deny;
            core.DownloadStarting += (_, e) => e.Cancel = true;
            core.AddWebResourceRequestedFilter(GlassEndpoint + "*", CoreWebView2WebResourceContext.Image,
                CoreWebView2WebResourceRequestSourceKinds.Document);
            core.WebResourceRequested += (_, args) =>
            {
                var bytes = glassFrame;
                args.Response = bytes == null
                    ? environment.CreateWebResourceResponse(null, 404, "Not Found", "Cache-Control: no-store")
                    : environment.CreateWebResourceResponse(new MemoryStream(bytes, writable: false), 200, "OK",
                        "Content-Type: image/jpeg\r\nCache-Control: no-store");
            };
            core.WebMessageReceived += Receive;
            core.NavigationCompleted += (_, e) =>
            {
                if (!e.IsSuccess || disposed) return;
                ready = true;
                Sync();
                SetCaption();
                UpdateGlass();
            };
            using var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream("SteamPulse.settings.html")
                ?? throw new InvalidDataException("settings.html");
            using var reader = new StreamReader(stream);
            core.NavigateToString(await reader.ReadToEndAsync());
        }
        catch (Exception ex) when (ex is InvalidOperationException or IOException or COMException or WebView2RuntimeNotFoundException)
        {
            if (disposed) return;
            Content = new TextBlock
            {
                Text = T("runtimeError"), Foreground = Brushes.White, FontSize = 17,
                Margin = new Thickness(28), TextWrapping = TextWrapping.Wrap
            };
        }
    }

    private void Receive(object? sender, CoreWebView2WebMessageReceivedEventArgs e)
    {
        if (!ready || disposed || e.Source != "about:blank" || e.WebMessageAsJson.Length > 2048) return;
        try
        {
            using var doc = JsonDocument.Parse(e.WebMessageAsJson);
            var message = doc.RootElement;
            if (!message.TryGetProperty("action", out var action)) return;
            switch (action.GetString())
            {
                case "change":
                    var key = message.GetProperty("key").GetString();
                    var value = message.GetProperty("value");
                    if (key == "AutoStart" && value.ValueKind is JsonValueKind.True or JsonValueKind.False)
                    {
                        var enabled = value.GetBoolean();
                        if (verify || WindowsStartup.Set(enabled)) startupEnabled = enabled;
                        else Send(new { type = "error", key = "startupError" });
                        Sync();
                    }
                    else if (key != null && Apply(key, value))
                    {
                        changed();
                        if (key == "Language") OpenCulture();
                        if (key == "LightTheme") SetCaption();
                        UpdateGlass();
                        if (key == "Language") Sync();
                    }
                    break;
                case "addLocalGame": Dispatcher.BeginInvoke(AddLocalGame); break;
                case "removeLocalGame":
                    var localId = message.GetProperty("id").GetString();
                    preferences.LocalGames = preferences.LocalGames.Where(x => x.Id != localId).ToArray();
                    changed();
                    SendLocalGames();
                    break;
                case "ignoreLocalGame":
                    var ignoredId = message.GetProperty("id").GetString();
                    var ignoredLocal = message.GetProperty("ignored").GetBoolean();
                    preferences.LocalGames = preferences.LocalGames.Select(x => x.Id == ignoredId ? x with { Ignored = ignoredLocal } : x).ToArray();
                    changed();
                    SendLocalGames();
                    break;
                case "renameLocalGame":
                    var name = (message.GetProperty("name").GetString() ?? "").Trim();
                    var gameId = message.GetProperty("id").GetString();
                    if (name.Length is > 0 and <= 90)
                    {
                        preferences.LocalGames = preferences.LocalGames.Select(x => x.Id == gameId ? x with { Name = name } : x).ToArray();
                        changed();
                    }
                    SendLocalGames();
                    break;
                case "library":
                    _ = LoadLibrary(message.TryGetProperty("refresh", out var refresh) && refresh.ValueKind == JsonValueKind.True);
                    break;
                case "ignore":
                    if (message.GetProperty("id").TryGetUInt32(out var id) && library?.Any(x => x.Id == id) == true)
                    {
                        var ignored = message.GetProperty("ignored").GetBoolean();
                        preferences.IgnoredGames = ignored ? preferences.IgnoredGames.Append(id).Distinct().ToArray() :
                            preferences.IgnoredGames.Where(x => x != id).ToArray();
                        changed();
                    }
                    break;
                case "reset": Dispatcher.BeginInvoke(reset); break;
                case "close": Dispatcher.BeginInvoke(Close); break;
                case "ready": Sync(); break;
                case "glassReady": SendGlassFrame(); break;
                case "status": RenderPreview(force: true); break;
                case "icons":
                    System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo("https://www.flaticon.com/uicons")
                        { UseShellExecute = true })?.Dispose();
                    break;
            }
        }
        catch (Exception ex) when (ex is InvalidOperationException or KeyNotFoundException or JsonException or FormatException or System.ComponentModel.Win32Exception)
        {
        }
    }

    private bool Apply(string key, JsonElement value)
    {
        if (!Editable.Contains(key)) return false;
        var prop = typeof(Preferences).GetProperty(key)!;
        if (prop.PropertyType == typeof(bool))
        {
            if (value.ValueKind is not (JsonValueKind.True or JsonValueKind.False)) return false;
            prop.SetValue(preferences, value.GetBoolean());
        }
        else if (prop.PropertyType == typeof(string))
        {
            if (value.ValueKind != JsonValueKind.String) return false;
            var text = value.GetString() ?? "";
            if (key == "Language" && text.Length != 0 && !All.ContainsKey(text)) return false;
            if (key == "Country" && text.Length != 0 && (text.Length != 2 || !text.All(char.IsAsciiLetter))) return false;
            if (key == "MonitorName" && text.Length != 0 && !Wf.Screen.AllScreens.Any(s => s.DeviceName == text)) return false;
            if (key is not ("Language" or "Country" or "MonitorName") &&
                (text.Length != 7 || text[0] != '#' || !text[1..].All(Uri.IsHexDigit))) return false;
            prop.SetValue(preferences, text);
        }
        else
        {
            if (key == "HorizontalPosition" && value.ValueKind == JsonValueKind.Null)
            {
                preferences.HorizontalPosition = null;
                return true;
            }
            if (value.ValueKind != JsonValueKind.Number || !value.TryGetDouble(out var n) || !double.IsFinite(n)) return false;
            var (min, max) = key switch
            {
                "HorizontalPosition" => (0d, 100d), "MinDiscount" => (5, 95),
                "MetricsInterval" => (1, 5), "FriendCycle" => (3, 20),
                "Width" => (200, 550), "Radius" => (0, 11), "GlassDepth" or "SettingsGlassDepth" => (-5, 5), "GlassBlur" or "SettingsGlassBlur" => (0, 10),
                "GlassDispersion" or "SettingsGlassDispersion" => (0, 3), "GlassLight" or "SettingsGlassLight" or "Angle" => (0, 360),
                "Opacity" => (.1, 1), "SettingsTint" => (.05, 1), _ => (0, 1)
            };
            n = Math.Clamp(n, min, max);
            if (prop.PropertyType == typeof(int)) prop.SetValue(preferences, (int)n);
            else prop.SetValue(preferences, n);
        }
        return true;
    }

    private void AddLocalGame()
    {
        if (disposed || preferences.LocalGames.Length >= 64) return;
        var dialog = new Microsoft.Win32.OpenFileDialog
        {
            Title = T("addLocalGame"), Filter = "Executable (*.exe)|*.exe",
            CheckFileExists = true, Multiselect = false, DereferenceLinks = true
        };
        if (dialog.ShowDialog(this) != true) return;
        var path = dialog.FileName;
        if (!Path.GetExtension(path).Equals(".exe", StringComparison.OrdinalIgnoreCase)) return;
        if (preferences.LocalGames.Any(x => GameActivity.MatchesExecutable(x.Path, path))) return;
        var title = Path.GetFileNameWithoutExtension(path);
        if (title.Length > 90) title = title[..90];
        preferences.LocalGames = preferences.LocalGames.Append(new LocalGame(Guid.NewGuid().ToString("N"), title, path)).ToArray();
        changed();
        SendLocalGames();
    }

    private void SendLocalGames() => Send(new { type = "localGames", games = preferences.LocalGames
        .Select(g => new { g.Id, g.Name, g.Ignored, file = Path.GetFileName(g.Path) }).ToArray() });

    private async Task LoadLibrary(bool refresh)
    {
        libraryRequested = true;
        if (libraryLoading) return;
        var root = live().Root;
        if (!refresh && library != null && libraryRoot == root) { Send(new { type = "library", games = library }); return; }
        libraryLoading = true;
        libraryRoot = root;
        try
        {
            library = await Task.Run(() => SteamLibrary.Read(root));
            Send(new { type = "library", games = library });
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or System.Security.SecurityException)
        {
            Send(new { type = "libraryError" });
        }
        finally { libraryLoading = false; }
    }

    public void Sync()
    {
        var entries = Editable.ToDictionary(x => x, x => typeof(Preferences).GetProperty(x)!.GetValue(preferences));
        entries["AutoStart"] = startupEnabled;
        entries["IgnoredGames"] = preferences.IgnoredGames;
        Send(new { type = "settings", entries, language = Localization.Language, saveFailed,
            version = typeof(SettingsWindow).Assembly.GetName().Version?.ToString(3) });
        SendDisplays(force: true);
        SendLocalGames();
        RenderPreview(force: true);
    }

    private void SendDisplays(bool force = false)
    {
        var now = Wf.Screen.AllScreens.Select(s => new DisplayChoice(s.DeviceName, s.Primary, s.Bounds.Width, s.Bounds.Height)).ToArray();
        if (!force && displays != null && displays.SequenceEqual(now)) return;
        displays = now;
        Send(new { type = "monitors", displays = now });
    }

    public void RenderPreview(bool force = false)
    {
        if (!ready) return;
        SendDisplays();
        var data = live();
        if (!force && data == lastStatus) return;
        lastStatus = data;
        if (libraryRequested && data.Root != libraryRoot) _ = LoadLibrary(false);
        var d = data.Download;
        Send(new { type = "status", load = data.Load, friends = data.Friends, active = data.Active, available = data.Root != null,
            game = data.Game is { } g ? new { g.Id, g.Name } : null,
            download = d == null ? null : new { d.App.Id, d.Name, d.Percent, d.Estimated, d.Installing,
                speed = d.Speed, remaining = d.Remaining?.TotalSeconds } });
    }

    public void SaveFailed(bool failed)
    {
        if (failed == saveFailed) return;
        saveFailed = failed;
        Send(new { type = "save", failed });
    }

    private void UpdateGlass()
    {
        if (!ready) return;
        var h = new WindowInteropHelper(this).Handle;
        backdrop.Configure(preferences.SettingsGlass, preferences.Optics(true, Math.Max(1, Native.GetDpiForWindow(h) / 96d)));
    }

    private Native.Rect? GlassBounds()
    {
        if (!ready || disposed || !web.IsLoaded || web.ActualWidth <= 0 || web.ActualHeight <= 0) return null;
        var topLeft = web.PointToScreen(new Point());
        var bottomRight = web.PointToScreen(new Point(web.ActualWidth, web.ActualHeight));
        return new Native.Rect { Left = (int)Math.Round(topLeft.X), Top = (int)Math.Round(topLeft.Y),
            Right = (int)Math.Round(bottomRight.X), Bottom = (int)Math.Round(bottomRight.Y) };
    }

    private void PresentGlass(byte[]? frame)
    {
        if (!ready || (ReferenceEquals(glassFrame, frame) && glassFrameSent)) return;
        glassFrame = frame;
        glassFrameSent = true;
        glassSequence++;
        SendGlassFrame();
    }

    private void SendGlassFrame() =>
        Send(new { type = "glass", frame = glassFrame == null ? null : GlassEndpoint + glassSequence });

    private void OpenCulture()
    {
        Title = "Steam Pulse · " + T("settings");
        lastStatus = null;
    }

    private void SetCaption()
    {
        var h = new WindowInteropHelper(this).Handle;
        var dark = preferences.LightTheme ? 0 : 1;
        if (h != IntPtr.Zero) DwmSetWindowAttribute(h, 20, ref dark, sizeof(int));
    }

    private void Send(object message)
    {
        if (ready && !disposed) web.CoreWebView2.PostWebMessageAsJson(JsonSerializer.Serialize(message, Json));
    }

    [DllImport("dwmapi.dll")] private static extern int DwmSetWindowAttribute(IntPtr hwnd, int flag, ref int value, int size);
}
