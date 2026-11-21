using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Automation;
using System.Windows.Interop;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;

using static SteamPulse.Localization;
using Wf = System.Windows.Forms;

namespace SteamPulse;

internal sealed class PulseCard : Border
{
    private static readonly Dictionary<string, ImageSource?> Images = new();
    private static readonly FontFamily Font = new("Segoe UI");
    private bool airMode;
    private Brush tint = Brushes.Transparent;
    private Brush plain = Brushes.Transparent;
    private readonly Border glass = new() { IsHitTestVisible = false };
    private readonly Border coating = new() { IsHitTestVisible = false };
    private readonly StackPanel content = new();
    private readonly Grid layers = new();
    private Brush foreground = Brushes.White;
    private Brush muted = Brushes.LightGray;
    private Brush accent = Brushes.Cyan;
    private Brush track = Brushes.DimGray;

    public PulseCard()
    {
        BorderThickness = new Thickness(1);
        layers.Children.Add(glass);
        layers.Children.Add(coating);
        layers.Children.Add(content);
        Child = layers;
    }

    public bool Render(Preferences p, SteamStatus status, Offer? offer, bool compact, bool badgeOnly = false)
    {
        var fg = Preferences.Color(Theme(p, p.Text, "#F1F7FF", "#152033"), Colors.White);
        var bg = Preferences.Color(Theme(p, p.Background, "#101929", "#F7F9FD"), Colors.Black);
        var bgEnd = Preferences.Color(Theme(p, p.BackgroundEnd, "#1F3851", "#D9E7FB"), Colors.DarkSlateGray);
        var a = Preferences.Color(Theme(p, p.Accent, "#54DAFE", "#2453A6"), Colors.Cyan);
        var z = Preferences.Color(Theme(p, p.AccentEnd, "#81A5FF", "#6049A0"), Colors.LightBlue);
        foreground = new SolidColorBrush(fg);
        muted = new SolidColorBrush(Alpha(fg, .70));
        track = new SolidColorBrush(Alpha(fg, .18));
        accent = p.Gradient ? Gradient(a, z, p.Angle) : new SolidColorBrush(a);
        var radius = new CornerRadius(compact ? Math.Min(11, p.Radius) : p.Radius);
        CornerRadius = radius;
        plain = p.Gradient ? Gradient(Alpha(bg, p.Opacity), Alpha(bgEnd, p.Opacity), p.Angle) : new SolidColorBrush(Alpha(bg, p.Opacity));
        tint = new SolidColorBrush(Alpha(bg, p.GlassTint));
        airMode = p.AirMode;
        if (airMode) glass.Background = null;
        BorderThickness = new Thickness(p.WidgetBorder && !airMode ? 1 : 0);
        BorderBrush = p.WidgetBorder && !airMode ? new SolidColorBrush(Alpha(Colors.White, p.Glass ? .36 : .26)) : null;
        glass.CornerRadius = coating.CornerRadius = radius;
        ApplyBackdrop();
        content.Children.Clear();
        content.Margin = compact ? new Thickness(8, 3, 8, 3) : new Thickness(12, 8, 12, 8);

        var download = p.Downloads && status.Download is { } dload && !p.Ignores(dload.App.Id) ? dload : null;
        var game = p.Games && status.Game is { } active && !p.Ignores(active.Id) ? active : null;
        var playing = p.Friends ? status.Friends?.Playing : null;
        var friendCount = playing?.Length ?? 0;
        var friend = friendCount > 0 ? playing![Math.Clamp(status.FriendOffset, 0, friendCount - 1)] : null;
        var friendSummary = friend == null ? "" : friend.Name + " · " + friend.Game + (friendCount > 1 ? $"  +{friendCount - 1}" : "");
        var dense = compact && (download != null && game != null || friend != null && (download != null || game != null));
        if (badgeOnly)
        {
            var discounted = offer != null && !p.Ignores(offer.Id) && p.Discounts && p.DiscountCard;
            var caption = download != null ? (p.Name ? download.Name : T("download")) :
                game != null ? (p.Name ? game.Name : T("inGame")) : discounted ? (p.Name ? offer!.Name : T("sale")) :
                T(status.Active ? "online" : status.Root == null ? "notFound" : "closed");
            var symbol = download != null ? "↓" : game != null ? "▶" : friend != null ? "♟" : discounted ? "%" : "•";
            var label = Label(symbol, 15, accent, FontWeights.SemiBold);
            label.HorizontalAlignment = HorizontalAlignment.Center;
            content.Children.Add(label);
            ToolTip = friend == null ? caption : caption + "\n" + friendSummary;
            AutomationProperties.SetName(this, caption);
            layers.InvalidateMeasure();
            InvalidateMeasure();
            return p.Widget;
        }
        ToolTip = friend == null ? null : friendSummary;
        AutomationProperties.SetName(this, "Steam Pulse");
        if (download != null)
        {
            var d = download;
            var stats = new List<string>();
            if (p.Percent && d.Percent.HasValue) stats.Add((d.Estimated ? "≈" : "") + d.Percent.Value.ToString("0.0", Culture) + "%");
            if (p.Speed && d.Speed.HasValue) stats.Add(Speed(d.Speed.Value, p.Bits));
            if (p.RemainingTime && d.Remaining is { } remaining) stats.Add("≈" + Duration(remaining));
            var state = T(d.Installing ? "installing" : "download");
            content.Children.Add(Row(p, d.App, state, string.Join("   ", stats), p.Bar ? d.Percent : null, compact, dense));
        }
        if (game != null)
        {
            var metrics = Metrics(p, status.Load);
            if (friendCount > 0 && download != null && compact) metrics = (metrics + $"  ♟{friendCount}").Trim();
            var row = Row(p, game, T("inGame"), metrics, null, compact, dense);
            if (content.Children.Count > 0) row.Margin = new Thickness(0, dense ? 2 : 7, 0, 0);
            content.Children.Add(row);
        }
        if (offer != null && !p.Ignores(offer.Id) && p.Discounts && p.DiscountCard && (!compact || content.Children.Count == 0))
        {
            var row = Row(p, new(offer.Id, offer.Name, offer.Image), T("sale"), $"−{offer.Discount}%", null, compact);
            if (content.Children.Count > 0) row.Margin = new Thickness(0, 7, 0, 0);
            content.Children.Add(row);
        }
        if (friend != null && (!compact || content.Children.Count < 2))
        {
            var row = Label("♟  " + friendSummary, compact ? 10.5 : 12, foreground);
            row.Margin = new Thickness(0, content.Children.Count == 0 ? 0 : 2, 0, 0);
            content.Children.Add(row);
        }
        if (content.Children.Count == 0)
        {
            var idle = T(status.Root == null ? "notFound" : status.Active ? "online" : "closed");
            content.Children.Add(Label(idle, 12, foreground, FontWeights.SemiBold));
        }
        if (p.Network && status.Network.HasValue && (!compact || download == null && content.Children.Count < 2))
        {
            var net = Label("↓ " + T("network") + "   " + Speed(status.Network.Value, p.Bits), 10, muted);
            net.Margin = new Thickness(0, 3, 0, 0);
            content.Children.Add(net);
        }
        layers.InvalidateMeasure();
        InvalidateMeasure();
        return p.Widget && content.Children.Count > 0;
    }

    internal static string Metrics(Preferences p, SystemLoad? load)
    {
        if (!p.MonitorSystem || load == null) return "";
        var values = new List<string>(2);
        if (p.MonitorCpu && load.Cpu is int cpu) values.Add($"CPU {cpu}%");
        if (p.MonitorRam && load.MemoryPercent is int ram)
        {
            var amount = p.RamGiB && load.MemoryGiB is double used ? used.ToString("0.0", Culture) + " GiB" : ram + "%";
            values.Add("RAM " + amount);
        }
        return string.Join("  ", values);
    }

    private static string Theme(Preferences p, string color, string dark, string light) =>
        p.LightTheme && color.Equals(dark, StringComparison.OrdinalIgnoreCase) ? light : color;

    public void SetGlass(ImageSource? image)
    {
        glass.Background = airMode || image == null ? null : new ImageBrush(image) { Stretch = Stretch.Fill };
        ApplyBackdrop();
    }

    private void ApplyBackdrop() => coating.Background = airMode ? null : glass.Background == null ? plain : tint;

    internal static string Duration(TimeSpan duration)
    {
        var minutes = (int)Math.Ceiling(duration.TotalMinutes);
        if (minutes >= 24 * 60) return $"{minutes / (24 * 60)} {T("days")} {minutes / 60 % 24} {T("hours")}";
        if (minutes >= 60) return $"{minutes / 60} {T("hours")} {minutes % 60} {T("minutes")}";
        if (duration.TotalSeconds >= 60) return $"{minutes} {T("minutes")}";
        return $"{Math.Max(1, Math.Ceiling(duration.TotalSeconds))} {T("seconds")}";
    }

    private Grid Row(Preferences p, GameInfo app, string state, string metric, double? progress, bool compact, bool dense = false)
    {
        var row = new Grid();
        if (p.Icon)
        {
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var edge = dense ? 16 : compact ? 23 : 33;
            var source = Art(app.Art);
            var tile = new Border
            {
                Width = edge, Height = edge, CornerRadius = new CornerRadius(compact ? 6 : 9),
                Background = source != null ? new ImageBrush(source) { Stretch = Stretch.UniformToFill } : track,
                Margin = new Thickness(0, 0, compact ? 7 : 9, 0), VerticalAlignment = VerticalAlignment.Center,
                Child = source == null ? new TextBlock { Text = state == T("inGame") ? "▶" : state == T("sale") ? "%" : "↓",
                    HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
                    FontSize = 13, Foreground = accent } : null
            };
            row.Children.Add(tile);
        }
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var body = new Grid();
        Grid.SetColumn(body, p.Icon ? 1 : 0);
        row.Children.Add(body);
        var details = new Grid();
        details.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        details.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var stack = new StackPanel();
        if (p.Name) stack.Children.Add(Label(app.Name, compact ? 10.5 : 12.5, foreground, FontWeights.SemiBold));
        if (!compact || !p.Name) stack.Children.Add(Label(state, compact ? 10 : 10.5, muted));
        details.Children.Add(stack);
        if (metric.Length > 0)
        {
            var value = Label(metric, compact ? 10 : 10.5, accent, FontWeights.SemiBold);
            value.Margin = new Thickness(7, 0, 0, 0);
            value.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(value, 1);
            details.Children.Add(value);
        }
        var vertical = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
        vertical.Children.Add(details);
        body.Children.Add(vertical);
        if (progress.HasValue)
        {
            var ratio = Math.Clamp(progress.Value / 100, 0, 1);
            var meter = new Grid { Height = compact ? 2 : 3, Margin = new Thickness(0, compact ? 2 : 4, 0, 0) };
            meter.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = track });
            meter.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(ratio, GridUnitType.Star) });
            meter.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1 - ratio, GridUnitType.Star) });
            Grid.SetColumnSpan(meter.Children[0], 2);
            if (ratio > 0) meter.Children.Add(new Border { CornerRadius = new CornerRadius(2), Background = accent });
            vertical.Children.Add(meter);
        }
        return row;
    }

    public static TextBlock Label(string text, double size, Brush brush, FontWeight? weight = null) => new()
    {
        Text = text, FontFamily = Font, FontSize = size, Foreground = brush,
        FontWeight = weight ?? FontWeights.Normal, TextTrimming = TextTrimming.CharacterEllipsis
    };

    public static string Speed(double bytes, bool bits)
    {
        var unit = bits ? (bytes * 8 / 1_000_000, "Mbps") :
            bytes >= 1024 * 1024 ? (bytes / (1024 * 1024), "MiB/s") : (bytes / 1024, "KiB/s");
        return unit.Item1.ToString("0.#", Culture) + " " + unit.Item2;
    }

    public static Color Alpha(Color c, double alpha) => Color.FromArgb((byte)Math.Clamp(Math.Round(alpha * c.A), 0, 255), c.R, c.G, c.B);

    public static LinearGradientBrush Gradient(Color a, Color b, double degrees)
    {
        var angle = degrees * Math.PI / 180;
        var dx = Math.Cos(angle) * .5;
        var dy = Math.Sin(angle) * .5;
        return new LinearGradientBrush(a, b, new Point(.5 - dx, .5 - dy), new Point(.5 + dx, .5 + dy));
    }

    private static ImageSource? Art(string? path)
    {
        if (path == null || !Path.IsPathFullyQualified(path)) return null;
        if (Images.TryGetValue(path, out var existing)) return existing;
        try
        {
            if (Path.GetExtension(path).Equals(".exe", StringComparison.OrdinalIgnoreCase))
            {
                using var icon = System.Drawing.Icon.ExtractAssociatedIcon(path);
                if (icon == null) return null;
                var image = Imaging.CreateBitmapSourceFromHIcon(icon.Handle, Int32Rect.Empty, BitmapSizeOptions.FromWidthAndHeight(48, 48));
                image.Freeze();
                if (Images.Count >= 100) Images.Clear();
                Images[path] = image;
                return image;
            }
            using var stream = File.Open(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            var source = new BitmapImage();
            source.BeginInit();
            source.StreamSource = stream;
            source.DecodePixelWidth = 84;
            source.CacheOption = BitmapCacheOption.OnLoad;
            source.EndInit();
            source.Freeze();
            if (Images.Count >= 100) Images.Clear();
            Images[path] = source;
            return source;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or NotSupportedException or FormatException or ArgumentException or System.ComponentModel.Win32Exception)
        {
            return null;
        }
    }
}

internal sealed class WidgetWindow : Window
{
    private readonly PulseCard card = new();
    private readonly TaskbarLayout layout = new();
    private readonly GlassBackdrop backdrop;
    private readonly DispatcherTimer layoutTimer = new(DispatcherPriority.Background) { Interval = TimeSpan.FromMilliseconds(240) };
    private readonly DispatcherTimer animate = new(DispatcherPriority.Render) { Interval = TimeSpan.FromMilliseconds(16) };
    private readonly Action<double>? moved;
    private TaskbarLayout.Slot target;
    private double shownWidth;
    private double shownCenter;
    private double dpi = 1;
    private Native.Rect bar;
    private double? drag;
    private int dragFrom;
    private double dragBase;
    private bool dragging;
    private bool useClickThrough;
    private IntPtr handle;
    private IntPtr taskbar;
    private static readonly uint TaskbarCreated = Native.RegisterWindowMessage("TaskbarCreated");
    private Preferences? placementSettings;
    private SteamStatus? stateShown;
    private Offer? offerShown;
    private bool overlayFallback;
    private readonly Dictionary<string, (Native.Rect Bar, TaskbarLayout.Slot Slot)> anchors = new();
    private bool badgeOnly;
    private bool closed;
    private WidgetEvents? windowEvents;
    public bool IsClosed => closed;

    public WidgetWindow(Action showSettings, Action<double>? moved = null)
    {
        this.moved = moved;
        WindowStyle = WindowStyle.None;
        ResizeMode = ResizeMode.NoResize;
        ShowInTaskbar = false;
        ShowActivated = false;
        AllowsTransparency = true;
        Background = Brushes.Transparent;
        Topmost = true;
        SizeToContent = SizeToContent.Height;
        card.ClipToBounds = true;
        Content = card;
        MouseLeftButtonDown += BeginDrag;
        MouseLeftButtonUp += (_, _) => EndDrag();
        animate.Tick += (_, _) =>
        {
            if (dragging) MoveDrag();
            ApplyPosition(true);
        };
        backdrop = new GlassBackdrop(this, card.SetGlass);
        MouseRightButtonUp += (_, _) => showSettings();
        SourceInitialized += (_, _) =>
        {
            handle = new WindowInteropHelper(this).Handle;
            Native.RemoveSystemMenu(handle);
            Native.ProtectFromPeek(handle);
            windowEvents = new WidgetEvents(Dispatcher, () => taskbar, Position);
            HwndSource.FromHwnd(handle)?.AddHook(ShellMessage);
        };
        layoutTimer.Tick += (_, _) => Position();
        Closed += (_, _) => { closed = true; windowEvents?.Dispose(); layoutTimer.Stop(); animate.Stop(); backdrop.Dispose(); };
    }

    public void Refresh(Preferences p, SteamStatus status, Offer? offer)
    {
        if (placementSettings != p || status != stateShown || offer != offerShown)
        {
            placementSettings = p with { };
            stateShown = status;
            offerShown = offer;
            card.Render(p, status, offer, true, badgeOnly);
        }
        if (p.Widget) layoutTimer.Start();
        else layoutTimer.Stop();
        Position();
    }

    private IntPtr ShellMessage(IntPtr hwnd, int message, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (message == 0x0112 && Native.BlockWidgetSystemCommand(wParam)) // WM_SYSCOMMAND
        {
            handled = true;
            return IntPtr.Zero;
        }
        if (message == TaskbarCreated || message == 0x007E) // WM_DISPLAYCHANGE
        {
            if (message == TaskbarCreated) layout.Rescan();
            else layout.Invalidate();
            Dispatcher.BeginInvoke(DispatcherPriority.Background, new Action(Position));
        }
        return IntPtr.Zero;
    }

    private void Position()
    {
        if (closed) return;
        if (placementSettings is not { Widget: true } p) { Conceal(); return; }
        var screen = ResolveScreen(p.MonitorName, Wf.Screen.AllScreens);
        taskbar = TaskbarLayout.ShellBar(screen);
        var visibleBar = Native.VisibleTaskbar(taskbar, screen.Bounds, out bar);
        dpi = Native.GetDpiForScreen(screen);
        var remembered = anchors.GetValueOrDefault(screen.DeviceName);
        if (!visibleBar) bar = ProjectTaskbar(screen.Bounds, remembered.Bar, dpi);
        if (handle == IntPtr.Zero)
        {
            Left = screen.WorkingArea.Left + 8;
            Top = screen.WorkingArea.Bottom - 45;
            new WindowInteropHelper(this).EnsureHandle();
        }
        var desired = (int)Math.Round(p.Width * dpi);
        var minimum = Math.Min(desired, (int)Math.Round(176 * dpi));
        var slot = visibleBar ? layout.Position(taskbar, bar, desired, minimum, (int)Math.Ceiling(36 * dpi), p.Placement,
            (int)(p.Offset * dpi), (int)Math.Ceiling(5 * dpi), drag ?? p.HorizontalPosition) : null;
        if (slot is { } safe) anchors[screen.DeviceName] = (bar, safe);
        overlayFallback = slot == null;
        var fit = slot ?? FallbackSlot(bar, dpi, drag ?? p.HorizontalPosition, remembered.Slot);
        if (overlayFallback && dragging)
        {
            dragging = false;
            drag = null;
            if (IsMouseCaptured) ReleaseMouseCapture();
        }
        useClickThrough = overlayFallback || (p.ClickThrough && !dragging);
        Cursor = useClickThrough || p.WidgetFrozen ? Cursors.Arrow : dragging ? Cursors.SizeWE : Cursors.Hand;
        var laneChanged = target.Safe != fit.Safe;
        target = fit;
        if (!IsVisible)
        {
            shownWidth = fit.Width;
            shownCenter = fit.Start + fit.Width / 2.0;
            if (ApplyPosition(false))
            {
                Show();
                ApplyPosition(false);
            }
        }
        else if (!SystemParameters.ClientAreaAnimation || laneChanged)
        {
            shownWidth = laneChanged && SystemParameters.ClientAreaAnimation ? Math.Max(Math.Min(minimum, fit.Width), fit.Width * .76) : fit.Width;
            shownCenter = fit.Start + fit.Width / 2d;
            if (ApplyPosition(false) && shownWidth < fit.Width) animate.Start();
        }
        else if (Math.Abs(shownWidth - fit.Width) > .3 || Math.Abs(shownCenter - (fit.Start + fit.Width / 2d)) > .3)
        {
            animate.Start();
        }
        else ApplyPosition(false);
    }

    internal static Wf.Screen ResolveScreen(string monitorName, Wf.Screen[] screens) =>
        screens.FirstOrDefault(s => string.Equals(s.DeviceName, monitorName, StringComparison.OrdinalIgnoreCase))
        ?? screens.FirstOrDefault(s => s.Primary) ?? screens[0];

    internal static Native.Rect ProjectTaskbar(System.Drawing.Rectangle display, Native.Rect remembered, double dpi)
    {
        var height = Math.Clamp(remembered.Bottom > remembered.Top ? remembered.Bottom - remembered.Top : (int)Math.Round(48 * dpi),
            1, Math.Max(1, display.Height));
        var top = remembered.Bottom > remembered.Top &&
            Math.Abs(remembered.Top - display.Top) < Math.Abs(remembered.Bottom - display.Bottom);
        return new Native.Rect { Left = display.Left, Right = display.Right, Top = top ? display.Top : display.Bottom - height,
            Bottom = top ? display.Top + height : display.Bottom };
    }

    internal static TaskbarLayout.Slot FallbackSlot(Native.Rect bar, double dpi, double? preferred, TaskbarLayout.Slot remembered)
    {
        var width = Math.Clamp((int)Math.Round(36 * dpi), 1, Math.Max(1, bar.Right - bar.Left));
        var center = preferred is double slider ? bar.Left + (bar.Right - bar.Left) * slider / 100 :
            remembered.Width > 0 ? remembered.Start + remembered.Width / 2d : bar.Left + (bar.Right - bar.Left) * .79;
        var x = Math.Clamp((int)Math.Round(center - width / 2d), bar.Left, Math.Max(bar.Left, bar.Right - width));
        return new(x, width, new TaskbarLayout.Interval(x, x + width));
    }

    private bool ApplyPosition(bool interpolate)
    {
        if (placementSettings is not { } p || target.Width < 1 || target.Safe.Length < 1) return false;
        var newCenter = target.Start + target.Width / 2d;
        if (interpolate)
        {
            shownWidth += (target.Width - shownWidth) * .26;
            shownCenter += (newCenter - shownCenter) * .36;
        }
        if (Math.Abs(shownWidth - target.Width) < .65 && Math.Abs(shownCenter - newCenter) < .65)
        {
            if (!dragging) animate.Stop();
            shownWidth = target.Width;
            shownCenter = newCenter;
        }
        var physicalWidth = Math.Clamp((int)Math.Round(shownWidth), 1, target.Safe.Length);
        var x = Math.Clamp((int)Math.Round(shownCenter - physicalWidth / 2d), target.Safe.Start, target.Safe.End - physicalWidth);
        var requested = physicalWidth / dpi;
        if (!Width.Equals(requested)) Width = requested;
        var small = overlayFallback || requested < 150 || (bar.Bottom - bar.Top) / dpi < 45;
        if (badgeOnly != small && stateShown != null)
        {
            badgeOnly = small;
            card.Render(p, stateShown, offerShown, true, badgeOnly);
        }
        card.Measure(new Size(requested, double.PositiveInfinity));
        var height = Math.Clamp((int)Math.Ceiling(card.DesiredSize.Height * dpi), 1, Math.Max(1, bar.Bottom - bar.Top));
        var y = bar.Top + (bar.Bottom - bar.Top - height) / 2;
        backdrop.Configure(p.Glass && !p.AirMode, p.Optics(false, dpi));
        if (IsVisible && Native.VisibilityProblem(handle) != null) Native.EnsureShown(handle);
        Native.Place(handle, x, y, useClickThrough, physicalWidth, height, taskbar);
        return true;
    }

    private void BeginDrag(object sender, MouseButtonEventArgs e)
    {
        if (useClickThrough || placementSettings?.WidgetFrozen != false) return;
        dragging = true;
        TryCaptureMouse();
        animate.Start();
        dragFrom = Wf.Control.MousePosition.X;
        dragBase = shownCenter;
        e.Handled = true;
    }

    private void TryCaptureMouse() => Mouse.Capture(this);

    private void MoveDrag()
    {
        if (placementSettings?.WidgetFrozen != false) { drag = null; EndDrag(); return; }
        if (!Native.LeftPressed()) { EndDrag(); return; }
        if (bar.Right <= bar.Left) return;
        var delta = Wf.Control.MousePosition.X - dragFrom;
        if (drag == null && Math.Abs(delta) < 3) return;
        var next = Math.Clamp((dragBase + delta - bar.Left) * 100 / (bar.Right - bar.Left), 0, 100);
        if (drag == next) return;
        drag = next;
        Position();
        if (dragging) animate.Start();
    }

    private void EndDrag()
    {
        if (!dragging) return;
        dragging = false;
        if (IsMouseCaptured) ReleaseMouseCapture();
        if (drag is double pos) moved?.Invoke(Math.Round(pos, 2));
        drag = null;
        Position();
    }

    private void Conceal()
    {
        animate.Stop();
        if (dragging)
        {
            dragging = false;
            drag = null;
            if (IsMouseCaptured) ReleaseMouseCapture();
        }
        if (IsVisible) Hide();
    }
}

internal static class Native
{
    [StructLayout(LayoutKind.Sequential)]
    internal struct Rect
    {
        public int Left, Top, Right, Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct AppBarData
    {
        public uint Size;
        public IntPtr Window;
        public uint Callback;
        public int Edge;
        public Rect Bounds;
        public IntPtr Param;
    }

    [DllImport("shell32.dll")] private static extern UIntPtr SHAppBarMessage(uint message, ref AppBarData data);
    [DllImport("user32.dll")] internal static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string name, string? title);
    [DllImport("user32.dll")] internal static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] private static extern IntPtr GetLong(IntPtr h, int index);
    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")] private static extern IntPtr SetLong(IntPtr h, int index, IntPtr flags);
    [DllImport("user32.dll")] private static extern bool SetWindowPos(IntPtr h, IntPtr insertAfter, int x, int y, int cx, int cy, uint flags);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    internal static extern uint RegisterWindowMessage(string message);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window, uint relation);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] private static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr window, int command);

    internal static string? VisibilityProblem(IntPtr handle) => IsIconic(handle) ? "Windows minimized the widget" :
        !IsWindowVisible(handle) ? "native window became hidden; trigger unknown" : null;

    internal static void EnsureShown(IntPtr handle)
    {
        if (VisibilityProblem(handle) != null) ShowWindow(handle, 4); // SW_SHOWNOACTIVATE
    }

    internal static bool Above(IntPtr candidate, IntPtr other)
    {
        if (candidate == IntPtr.Zero || other == IntPtr.Zero || candidate == other) return false;
        var h = other;
        for (var i = 0; i < 256; i++)
        {
            h = GetWindow(h, 3); // GW_HWNDPREV, a window above the current one
            if (h == IntPtr.Zero) break;
            if (h == candidate) return true;
        }
        return false;
    }

    internal static bool VisibleTaskbar(IntPtr hwnd, System.Drawing.Rectangle monitor, out Rect bar)
    {
        bar = default;
        if (hwnd == IntPtr.Zero || !IsWindowVisible(hwnd) || !GetWindowRect(hwnd, out var actual)) return false;
        var height = actual.Bottom - actual.Top;
        if (height < 22 || actual.Right - actual.Left <= height ||
            actual.Top < monitor.Top || actual.Bottom > monitor.Bottom + 1) return false;
        bar = actual;
        return true;
    }

    [DllImport("dwmapi.dll")]
    private static extern int DwmSetWindowAttribute(IntPtr handle, int attribute, ref int value, int size);

    internal static bool ProtectFromPeek(IntPtr handle)
    {
        var enabled = 1;
        var result = DwmSetWindowAttribute(handle, 12, ref enabled, sizeof(int)); // DWMWA_EXCLUDED_FROM_PEEK
        DwmSetWindowAttribute(handle, 3, ref enabled, sizeof(int)); // DWMWA_TRANSITIONS_FORCEDISABLED
        return result == 0;
    }

    internal static bool BlockWidgetSystemCommand(IntPtr command) => (command.ToInt64() & 0xFFF0) is
        0xF000 or 0xF010 or 0xF020 or 0xF030 or 0xF060 or 0xF090 or 0xF100 or 0xF120;

    internal static void RemoveSystemMenu(IntPtr handle)
    {
        const int style = -16;
        const int systemMenu = 0x00080000;
        var previous = GetLong(handle, style).ToInt64();
        if ((previous & systemMenu) == 0) return;
        SetLong(handle, style, new IntPtr(previous & ~systemMenu));
        SetWindowPos(handle, IntPtr.Zero, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0004 | 0x0010 | 0x0020);
    }

    internal static void Place(IntPtr handle, int x, int y, bool clickThrough, int width, int height, IntPtr taskbar = default)
    {
        const int noActivate = 0x08000000;
        const int tool = 0x80;
        const int transparent = 0x20;
        var existing = GetLong(handle, -20).ToInt64();
        var next = (existing | noActivate | tool) & ~transparent;
        if (clickThrough) next |= transparent;
        if (next != existing) SetLong(handle, -20, new IntPtr(next));
        if (!GetWindowRect(handle, out var old) || old.Left != x || old.Top != y || old.Right - old.Left != width || old.Bottom - old.Top != height)
            SetWindowPos(handle, new IntPtr(-1), x, y, width, height, 0x0010 | 0x0200);
        else if (Above(taskbar, handle) || ForegroundCovers(handle, x, y, width, height))
            SetWindowPos(handle, new IntPtr(-1), 0, 0, 0, 0, 0x0010 | 0x0200 | 0x0001 | 0x0002);
    }

    [DllImport("user32.dll")] private static extern short GetAsyncKeyState(int key);
    internal static bool LeftPressed() => GetAsyncKeyState(1) < 0;

    [DllImport("user32.dll")] private static extern IntPtr MonitorFromPoint(System.Drawing.Point point, uint fallback);
    [DllImport("shcore.dll")] private static extern int GetDpiForMonitor(IntPtr monitor, int type, out uint x, out uint y);

    internal static double GetDpiForScreen(Wf.Screen screen)
    {
        var b = screen.Bounds;
        var monitor = MonitorFromPoint(new System.Drawing.Point(b.Left + b.Width / 2, b.Top + b.Height / 2), 2);
        return GetDpiForMonitor(monitor, 0, out var x, out _) == 0 && x > 0 ? x / 96d : 1;
    }

    internal static bool ForegroundCovers(IntPtr handle, int x, int y, int width, int height)
    {
        var foreground = GetForegroundWindow();
        if (foreground == IntPtr.Zero || foreground == handle) return false;
        GetWindowThreadProcessId(foreground, out var process);
        if (process == Environment.ProcessId || !GetWindowRect(foreground, out var r) ||
            r.Left >= x + width || r.Right <= x || r.Top >= y + height || r.Bottom <= y) return false;
        return Above(foreground, handle);
    }

    internal static (Rect Rect, int Edge) Taskbar(Wf.Screen screen)
    {
        if (screen.Primary)
        {
            var data = new AppBarData { Size = (uint)Marshal.SizeOf<AppBarData>() };
            if (SHAppBarMessage(5, ref data) != UIntPtr.Zero) return (data.Bounds, data.Edge);
        }
        var h = IntPtr.Zero;
        while ((h = FindWindowEx(IntPtr.Zero, h, "Shell_SecondaryTrayWnd", null)) != IntPtr.Zero)
        {
            if (Wf.Screen.FromHandle(h).DeviceName != screen.DeviceName || !GetWindowRect(h, out var r)) continue;
            var b = screen.Bounds;
            var vertical = r.Bottom - r.Top > r.Right - r.Left;
            var edge = vertical ? (r.Left < b.Left + b.Width / 2 ? 0 : 2) : (r.Top < b.Top + b.Height / 2 ? 1 : 3);
            return (r, edge);
        }
        var work = screen.WorkingArea;
        return (new Rect { Left = work.Left, Right = work.Right, Top = work.Bottom, Bottom = work.Bottom }, 3);
    }
}
