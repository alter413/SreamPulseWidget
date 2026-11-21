using System;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Runtime.InteropServices;
using SystemParameters = System.Windows.SystemParameters;
using Wf = System.Windows.Forms;

namespace SteamPulse;

internal sealed class TraySubmenuItem : Wf.ToolStripMenuItem
{
    public TraySubmenuItem()
    {
        DropDown = new Wf.ToolStripDropDownMenu { ShowImageMargin = false };
        TrayPopups.Attach(DropDown);
    }

    protected override void OnMouseDown(Wf.MouseEventArgs e)
    {
        if (e.Button != Wf.MouseButtons.Left) base.OnMouseDown(e);
    }

    protected override void OnMouseUp(Wf.MouseEventArgs e)
    {
        if (e.Button == Wf.MouseButtons.Left && Enabled && HasDropDownItems)
        {
            if (DropDown.Visible) HideDropDown();
            else ShowDropDown();
        }
        else base.OnMouseUp(e);
    }
}

internal static class TrayPopups
{
    public static void Attach(Wf.ToolStripDropDown menu)
    {
        var timer = new Wf.Timer { Interval = 15 };
        var watch = new Stopwatch();
        var expected = false;
        var lastLocation = Point.Empty;
        menu.Opening += (_, _) =>
        {
            if (!Wf.SystemInformation.HighContrast && SystemParameters.ClientAreaAnimation)
            {
                menu.AllowTransparency = true;
                menu.Opacity = .72;
            }
        };
        menu.Opened += (_, _) =>
        {
            expected = true;
            lastLocation = menu.Location;
            Round(menu);
            watch.Restart();
            timer.Interval = menu.AllowTransparency && menu.Opacity < 1 ? 15 : 150;
            timer.Start();
        };
        menu.SizeChanged += (_, _) => { if (menu.Visible) Round(menu); };
        timer.Tick += (_, _) =>
        {
            if (!expected || !menu.Visible || !menu.IsHandleCreated) { timer.Stop(); return; }
            if (!IsWindowVisible(menu.Handle)) ShowWindow(menu.Handle, 4);
            if (timer.Interval != 15) return;
            var t = Math.Clamp(watch.Elapsed.TotalMilliseconds / 135, 0, 1);
            menu.Opacity = 1 - .28 * (1 - t) * (1 - t);
            if (t == 1) timer.Interval = 150;
        };
        menu.Closing += (_, _) => expected = false;
        menu.VisibleChanged += (_, _) =>
        {
            if (menu.Visible || !expected || menu.IsDisposed || menu.Disposing) return;
            expected = false;
            if (!menu.IsHandleCreated) return;
            menu.BeginInvoke((Action)(() =>
            {
                if (menu.IsDisposed || menu.Visible || (menu.OwnerItem is { } owner && !owner.IsOnDropDown)) return;
                if (menu.OwnerItem is Wf.ToolStripDropDownItem item)
                {
                    if (item.GetCurrentParent()?.Visible != true) return;
                    item.ShowDropDown();
                }
                else menu.Show(lastLocation);
            }));
        };
        menu.Closed += (_, _) =>
        {
            timer.Stop();
            if (menu.AllowTransparency) menu.Opacity = 1;
        };
        menu.Disposed += (_, _) => timer.Dispose();
    }

    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr handle);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr handle, int command);

    private static void Round(Wf.ToolStripDropDown menu)
    {
        var radius = Math.Min(menu.DeviceDpi * 12f / 96, Math.Min(menu.Width, menu.Height) / 2f);
        var oldRegion = menu.Region;
        if (Wf.SystemInformation.HighContrast || radius < 2)
        {
            menu.Region = null;
        }
        else
        {
            using var path = new GraphicsPath();
            path.AddRoundedRectangle(new RectangleF(0, 0, menu.Width, menu.Height), new SizeF(radius, radius));
            menu.Region = new Region(path);
        }
        oldRegion?.Dispose();
    }
}
