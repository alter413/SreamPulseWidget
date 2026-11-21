using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Threading;
using static SteamPulse.Localization;
using Wf = System.Windows.Forms;

namespace SteamPulse;

internal static class Program
{
    [STAThread]
    public static void Main(string[] args)
    {
        var verify = args.Contains("--verify");
        if (verify) Preferences.Folder = Path.Combine(Path.GetTempPath(), "SteamPulse-verify-" + Environment.ProcessId);
        var name = @"Local\SteamPulse." + (verify ? "Verify." : "");
        using var instance = new Mutex(true, name + "App", out var first);
        if (!first)
        {
            try { using var signal = EventWaitHandle.OpenExisting(name + "Settings"); signal.Set(); }
            catch (WaitHandleCannotBeOpenedException) { }
            return;
        }
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        using var host = new PulseHost(app, name, verify, verify || args.Contains("--settings"));
        app.Run();
    }

}

internal sealed partial class PulseHost : IDisposable
{
    private readonly Application app;
    private WidgetWindow widget;
    private readonly Wf.NotifyIcon tray;
    private readonly Wf.ContextMenuStrip menu;
    private readonly Wf.ToolStripRenderer darkTray = new DarkTrayRenderer();
    private readonly Wf.ToolStripRenderer lightTray = new Wf.ToolStripSystemRenderer();
    private readonly Wf.ToolStripMenuItem showWidget;
    private readonly Wf.ToolStripMenuItem freezeWidget;
    private readonly Wf.ToolStripMenuItem steamStatus = new() { Enabled = false };
    private readonly Wf.ToolStripMenuItem saleList = new TraySubmenuItem();
    private readonly Wf.ToolStripMenuItem openSettingsItem;
    private readonly Wf.ToolStripMenuItem openDownloadsItem;
    private readonly DispatcherTimer liveTimer = new() { Interval = TimeSpan.FromSeconds(2) };
    private readonly DispatcherTimer storeTimer = new() { Interval = TimeSpan.FromMinutes(35) };
    private readonly DispatcherTimer saveTimer = new() { Interval = TimeSpan.FromMilliseconds(350) };
    private readonly SteamReader reader = new();
    private readonly GameActivity monitor = new();
    private FriendsSnapshot? friends;
    private ulong friendsAccount;
    private DateTime nextFriendsRefresh;
    private bool friendsBusy;
    private readonly Icon icon;
    private readonly EventWaitHandle request;
    private readonly RegisteredWaitHandle listener;
    private readonly bool verify;
    private Preferences settings = Preferences.Load();
    private SettingsWindow? settingsWindow;
    private SteamStatus status = SteamStatus.Empty;
    private Offer? cardOffer;
    private DateTime expires;
    private uint notificationId;
    private bool reading;
    private bool disposed;
    private (bool Enabled, string Country, int Minimum) lastMarket;

    private (bool, string, int) Market => (settings.Discounts, settings.Country, settings.MinDiscount);

    public PulseHost(Application app, string instance, bool verify, bool openSettings)
    {
        this.app = app;
        this.verify = verify;
        Set(settings.Language);
        lastMarket = Market;
        widget = CreateWidget();
        menu = new Wf.ContextMenuStrip { ShowImageMargin = false };
        TrayPopups.Attach(menu);
        showWidget = new Wf.ToolStripMenuItem(T("Widget"), null, (_, _) =>
        {
            settings.Widget = !settings.Widget;
            Changed();
            settingsWindow?.Sync();
        });
        PopulateOffers("loadingSales");
        menu.Items.Add(steamStatus);
        menu.Items.Add(new Wf.ToolStripSeparator());
        menu.Items.Add(openSettingsItem = new Wf.ToolStripMenuItem(T("settings") + "…", null, (_, _) => OpenSettings()));
        menu.Items.Add(showWidget);
        menu.Items.Add(freezeWidget = new Wf.ToolStripMenuItem(T("freezeWidget"), null, (_, _) =>
        {
            settings.WidgetFrozen = !settings.WidgetFrozen;
            Changed();
        }));
        menu.Items.Add(openDownloadsItem = new Wf.ToolStripMenuItem(T("openDownloads"), null, (_, _) => OpenUri("steam://open/downloads")));
        menu.Items.Add(saleList);
        ApplyTrayTheme();
        menu.Opening += (_, _) => UpdateMenu();
        icon = Icon.ExtractAssociatedIcon(Environment.ProcessPath!) ?? (Icon)SystemIcons.Application.Clone();
        tray = new Wf.NotifyIcon { Icon = icon, Text = "Steam Pulse", ContextMenuStrip = menu, Visible = !verify };
        tray.MouseDoubleClick += (_, e) => { if (e.Button == Wf.MouseButtons.Left) OpenSettings(); };
        tray.BalloonTipClicked += (_, _) => { if (notificationId != 0) OpenOffer(notificationId); };
        request = new EventWaitHandle(false, EventResetMode.AutoReset, instance + "Settings");
        listener = ThreadPool.RegisterWaitForSingleObject(request, (_, _) => app.Dispatcher.BeginInvoke(OpenSettings),
            null, Timeout.Infinite, executeOnlyOnce: false);
        liveTimer.Interval = MetricsDelay();
        liveTimer.Tick += (_, _) => _ = Poll();
        liveTimer.Start();
        storeTimer.Tick += (_, _) => _ = RefreshOffers(true);
        if (!verify) storeTimer.Start();
        saveTimer.Tick += (_, _) => { saveTimer.Stop(); Save(); };

        app.Dispatcher.BeginInvoke(() =>
        {
            if (openSettings) OpenSettings();
            _ = Poll();
            if (!verify && settings.Discounts) _ = RefreshOffers(true);
        });
    }

    private WidgetWindow CreateWidget() => new(OpenSettings, x =>
    {
        settings.HorizontalPosition = x;
        Changed();
        settingsWindow?.Sync();
    });

    private void RefreshWidget()
    {
        if (widget.IsClosed && settings.Widget)
        {
            widget = CreateWidget();
        }
        widget.Refresh(settings, status, cardOffer);
    }

    private async Task Poll()
    {
        if (reading || disposed) return;
        reading = true;
        var options = settings with { };
        try
        {
            status = await Task.Run(() =>
            {
                var steam = reader.Read(options.Network);
                var game = monitor.Running(options.LocalGames) ?? steam.Game;
                var metrics = options.Games && game != null && !options.Ignores(game.Id) && options.MonitorSystem
                    ? monitor.Sample(options.MonitorCpu, options.MonitorRam) : monitor.Sample(false, false);
                return steam with { Game = game, Load = metrics };
            });
            if (disposed) return;
            if (settings.Friends && status.Active) _ = UpdateFriends();
            ApplyFriendSnapshot();
            if (DateTime.Now >= expires) cardOffer = null;
            RefreshWidget();
            settingsWindow?.RenderPreview();
            UpdateMenu();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or
            System.ComponentModel.Win32Exception or System.Net.NetworkInformation.NetworkInformationException)
        {
        }
        finally { reading = false; }
    }

    private TimeSpan MetricsDelay() => TimeSpan.FromSeconds(settings.MonitorSystem ? settings.MetricsInterval : 2);

    private void ApplyFriendSnapshot() => status = status with
    {
        Friends = settings.Friends && status.Active ? friends : null,
        FriendOffset = friends is { Playing.Length: > 0 }
            ? (int)(DateTimeOffset.UtcNow.ToUnixTimeSeconds() / settings.FriendCycle % friends.Playing.Length) : 0
    };

    private async Task UpdateFriends()
    {
        var account = FriendsActivity.ActiveAccount();
        if (account != friendsAccount)
        {
            friendsAccount = account;
            friends = null;
            nextFriendsRefresh = default;
        }
        if (friendsBusy || DateTime.UtcNow < nextFriendsRefresh) return;
        nextFriendsRefresh = DateTime.UtcNow.AddMinutes(2);
        friendsBusy = true;
        try
        {
            var snapshot = await FriendsActivity.Read(account);
            if (disposed || account != friendsAccount || !settings.Friends) return;
            friends = snapshot;
            ApplyFriendSnapshot();
            RefreshWidget();
            settingsWindow?.RenderPreview();
        }
        finally { friendsBusy = false; }
    }

    private void OpenSettings()
    {
        if (disposed) return;
        if (settingsWindow == null)
        {
            var window = new SettingsWindow(settings, Changed, Reset, () => status, verify);
            window.ShowInTaskbar = verify;
            settingsWindow = window;
            window.Closed += (_, _) => { settingsWindow = null; };
            window.Show();
        }
        if (settingsWindow.WindowState == WindowState.Minimized) settingsWindow.WindowState = WindowState.Normal;
        settingsWindow.Activate();
    }

    private void Reset()
    {
        if (MessageBox.Show(settingsWindow, T("resetPrompt"),
            "Steam Pulse", MessageBoxButton.YesNo, MessageBoxImage.Question) != MessageBoxResult.Yes) return;
        if (!verify && WindowsStartup.Enabled()) WindowsStartup.Set(false);
        settings = new Preferences { SeenOffers = settings.SeenOffers, Widget = settings.Widget, LocalGames = settings.LocalGames };
        settingsWindow?.Close();
        Changed();
        OpenSettings();
    }

    private void Changed()
    {
        var beforeLanguage = Language;
        Set(settings.Language);
        if (beforeLanguage != Language) PopulateOffers(offerMessage);
        saveTimer.Stop();
        saveTimer.Start();
        var interval = MetricsDelay();
        if (liveTimer.Interval != interval) liveTimer.Interval = interval;
        ApplyFriendSnapshot();
        if (status.Game is { LocalId: { } id } && !settings.LocalGames.Any(g => g.Id == id && !g.Ignored))
            status = status with { Game = null, Load = null };
        RefreshWidget();
        settingsWindow?.RenderPreview();
        UpdateMenu();
        if (settings.Discounts && !verify && Market != lastMarket)
        {
            storeTimer.Stop();
            storeTimer.Interval = TimeSpan.FromSeconds(4);
            storeTimer.Start();
        }
        lastMarket = Market;
    }

    private void Save()
    {
        try
        {
            settings.Save();
            settingsWindow?.SaveFailed(false);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            settingsWindow?.SaveFailed(true);
        }
    }

    private void ApplyTrayTheme()
    {
        var renderer = settings.LightTheme || Wf.SystemInformation.HighContrast ? lightTray : darkTray;
        if (menu.Renderer != renderer) menu.Renderer = renderer;
        if (saleList.DropDown.Renderer != renderer) saleList.DropDown.Renderer = renderer;
    }

    private void UpdateMenu()
    {
        ApplyTrayTheme();
        showWidget.Text = T(settings.Widget ? "hideWidget" : "Widget");
        freezeWidget.Text = T(settings.WidgetFrozen ? "unfreezeWidget" : "freezeWidget");
        openSettingsItem.Text = T("settings") + "…";
        openDownloadsItem.Text = T("openDownloads");
        saleList.Text = T("sales");
        steamStatus.Text = T(status.Active ? "connected" : status.Root == null ? "notFound" : "closed");
        var line = status.Download is { } d && settings.Downloads && settings.Name && !settings.Ignores(d.App.Id) ? T("download") + " · " + d.Name :
            status.Game is { } game && settings.Games && settings.Name && !settings.Ignores(game.Id) ? T("inGame") + " · " + game.Name : steamStatus.Text;
        var hint = "Steam Pulse" + Environment.NewLine + line;
        var shortHint = hint.Length > 63 ? hint[..62] + "…" : hint;
        if (tray.Text != shortHint) tray.Text = shortHint;
    }

    public void Dispose()
    {
        disposed = true;
        liveTimer.Stop();
        storeTimer.Stop();
        if (saveTimer.IsEnabled) Save();
        saveTimer.Stop();
        listener.Unregister(null);
        request.Dispose();
        tray.Visible = false;
        tray.Dispose();
        menu.Dispose();
        icon.Dispose();
    }
}