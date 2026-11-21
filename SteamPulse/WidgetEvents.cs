using System;
using System.Runtime.InteropServices;
using System.Windows.Threading;

namespace SteamPulse;

internal sealed class WidgetEvents : IDisposable
{
    private readonly Dispatcher dispatcher;
    private readonly Action refresh;
    private readonly Func<IntPtr> shell;
    private readonly Callback callback;
    private readonly IntPtr foregroundHook;
    private readonly IntPtr shellHook;
    private bool queued;
    private bool disposed;

    public WidgetEvents(Dispatcher dispatcher, Func<IntPtr> shell, Action refresh)
    {
        this.dispatcher = dispatcher;
        this.refresh = refresh;
        this.shell = shell;
        callback = OnEvent;
        // The callback receives window events out of process. Own window changes are excluded.
        foregroundHook = SetWinEventHook(3, 3, IntPtr.Zero, callback, 0, 0, 2);
        shellHook = SetWinEventHook(0x8002, 0x8004, IntPtr.Zero, callback, 0, 0, 2);
    }

    private void OnEvent(IntPtr hook, uint id, IntPtr hwnd, int obj, int child, uint thread, uint time)
    {
        if (disposed || queued || hwnd == IntPtr.Zero || obj != 0 || child != 0) return;
        if (id != 3 && hwnd != shell()) return;
        queued = true;
        dispatcher.BeginInvoke(DispatcherPriority.Input, new Action(() =>
        {
            queued = false;
            if (!disposed) refresh();
        }));
    }

    public void Dispose()
    {
        disposed = true;
        if (foregroundHook != IntPtr.Zero) UnhookWinEvent(foregroundHook);
        if (shellHook != IntPtr.Zero) UnhookWinEvent(shellHook);
        GC.KeepAlive(callback);
    }

    private delegate void Callback(IntPtr hook, uint id, IntPtr hwnd, int obj, int child, uint thread, uint time);
    [DllImport("user32.dll")] private static extern IntPtr SetWinEventHook(uint first, uint last, IntPtr module, Callback callback, uint process, uint thread, uint flags);
    [DllImport("user32.dll")] private static extern bool UnhookWinEvent(IntPtr handle);
}
