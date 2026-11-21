using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Automation;
using Wf = System.Windows.Forms;

namespace SteamPulse;

internal sealed class TaskbarLayout
{
    private IntPtr pendingHandle;
    private Task<Interval[]>? scan;
    private IntPtr cachedHandle;
    private Interval[] occupied = [];
    private DateTime nextScan;

    internal readonly record struct Interval(int Start, int End)
    {
        public int Length => End - Start;
    }

    internal readonly record struct Slot(int Start, int Width, Interval Safe);

    public void Rescan()
    {
        scan = null;
        nextScan = default;
    }

    public void Invalidate()
    {
        pendingHandle = cachedHandle = IntPtr.Zero;
        occupied = [];
        Rescan();
    }

    public Slot? Position(IntPtr h, Native.Rect bar, int desired, int preferredMinimum, int badgeMinimum,
        int placement, int offset, int gutter, double? preferred = null)
    {
        if (bar.Right - bar.Left < badgeMinimum) return null;
        if (pendingHandle != IntPtr.Zero && pendingHandle != h) Invalidate();

        if (scan is { IsCompleted: true })
        {
            if (scan.IsCompletedSuccessfully && scan.Result.Length > 0)
            {
                cachedHandle = pendingHandle;
                occupied = scan.Result;
            }
            scan = null;
        }
        if (scan == null && (h != pendingHandle || DateTime.UtcNow >= nextScan))
        {
            pendingHandle = h;
            nextScan = DateTime.UtcNow.AddMilliseconds(750);
            scan = Task.Run(() => ReadButtons(h, bar));
        }
        if (h != cachedHandle || occupied.Length == 0) return null;
        return Pick(bar.Left, bar.Right, occupied, desired, preferredMinimum, placement, offset, gutter, preferred)
            ?? Pick(bar.Left, bar.Right, occupied, desired, badgeMinimum, placement, offset, gutter, preferred);
    }

    internal static Slot? Pick(int start, int end, IEnumerable<Interval> occupied, int desired,
        int minimum, int placement, int offset, int gutter, double? preferred = null)
    {
        var gaps = new List<Interval>();
        var cursor = start + gutter;
        foreach (var input in occupied.OrderBy(x => x.Start))
        {
            var left = Math.Clamp(input.Start - gutter, start, end);
            if (left - cursor >= minimum) gaps.Add(new(cursor, left));
            cursor = Math.Max(cursor, Math.Clamp(input.End + gutter, start, end));
        }
        if (end - gutter - cursor >= minimum) gaps.Add(new(cursor, end - gutter));
        if (gaps.Count == 0) return null;
        if (preferred.HasValue)
        {
            var center = start + (int)Math.Round((end - start) * preferred.Value / 100);
            var nearby = gaps.OrderBy(x => Distance(x, center)).ThenByDescending(x => x.Length).First();
            var room = 2 * Math.Min(center - nearby.Start, nearby.End - center);
            var available = Math.Clamp(room, minimum, Math.Min(desired, nearby.Length));
            var left = Math.Clamp(center - available / 2, nearby.Start, nearby.End - available);
            return new Slot(left, available, nearby);
        }
        var target = placement switch { 0 => start, 1 => (start + end) / 2, _ => end };
        var best = gaps.OrderBy(x => x.Length >= desired * .85 ? 0 : 1)
            .ThenBy(x => Distance(x, target)).ThenByDescending(x => x.Length).First();
        var width = Math.Min(best.Length, desired);
        var ideal = placement switch { 0 => best.Start, 1 => (best.Start + best.End - width) / 2, _ => best.End - width };
        var x = Math.Clamp(ideal + (placement == 2 ? -offset : offset), best.Start, best.End - width);
        return new Slot(x, width, best);
    }

    private static int Distance(Interval x, int target) => Math.Max(0, Math.Max(x.Start - target, target - x.End));

    private static Interval[] ReadButtons(IntPtr shell, Native.Rect bar)
    {
        try
        {
            var hits = new List<Interval>();
            var tray = FindWindowEx(shell, IntPtr.Zero, "TrayNotifyWnd", null);
            if (tray != IntPtr.Zero && Native.GetWindowRect(tray, out var trayBounds) && trayBounds.Right > trayBounds.Left)
                hits.Add(new(trayBounds.Left, trayBounds.Right));

            var root = AutomationElement.FromHandle(shell);
            var queue = new Queue<(AutomationElement Element, int Level)>();
            queue.Enqueue((root, 0));
            var walker = TreeWalker.ControlViewWalker;
            var count = 0;
            var buttons = 0;
            while (queue.Count > 0 && count++ < 260)
            {
                var (item, level) = queue.Dequeue();
                var kind = item.Current.ControlType;
                if (kind == ControlType.Button || kind == ControlType.TabItem || kind == ControlType.MenuItem)
                {
                    var r = item.Current.BoundingRectangle;
                    if (!r.IsEmpty && r.Width > 0 && r.Bottom > bar.Top && r.Top < bar.Bottom)
                    {
                        hits.Add(new((int)Math.Floor(r.Left), (int)Math.Ceiling(r.Right)));
                        buttons++;
                    }
                    continue;
                }
                if (level >= 6) continue;
                for (var child = walker.GetFirstChild(item); child != null && queue.Count < 260; child = walker.GetNextSibling(child))
                    queue.Enqueue((child, level + 1));
            }
            return buttons > 0 ? hits.ToArray() : [];
        }
        catch (Exception ex) when (ex is ElementNotAvailableException or InvalidOperationException or COMException or ArgumentException)
        {
            return [];
        }
    }

    internal static IntPtr ShellBar(Wf.Screen screen)
    {
        if (screen.Primary) return FindWindowEx(IntPtr.Zero, IntPtr.Zero, "Shell_TrayWnd", null);
        var h = IntPtr.Zero;
        while ((h = FindWindowEx(IntPtr.Zero, h, "Shell_SecondaryTrayWnd", null)) != IntPtr.Zero)
            if (Wf.Screen.FromHandle(h).DeviceName == screen.DeviceName) return h;
        return IntPtr.Zero;
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string name, string? title);
}
