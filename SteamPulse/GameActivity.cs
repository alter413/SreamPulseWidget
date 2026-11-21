using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;

namespace SteamPulse;

public record LocalGame(string Id, string Name, string Path, bool Ignored = false);
public record SystemLoad(int? Cpu, int? MemoryPercent, double? MemoryGiB);

internal sealed class GameActivity
{
    private (ulong Total, ulong Idle)? previousCpu;

    public GameInfo? Running(IEnumerable<LocalGame> configured)
    {
        var games = configured.Where(g => !g.Ignored && Path.IsPathFullyQualified(g.Path)).ToArray();
        if (games.Length == 0) return null;
        var byName = games.ToLookup(x => Path.GetFileNameWithoutExtension(x.Path), StringComparer.OrdinalIgnoreCase);
        var active = new List<(LocalGame Game, int Process, string Image)>();
        var processes = Process.GetProcesses();
        try
        {
            foreach (var process in processes)
            {
                try
                {
                    var candidates = byName[process.ProcessName];
                    if (!candidates.Any() || process.HasExited || process.SessionId != ProcessSession.Id) continue;
                    var path = ImagePath(process.Id);
                    var match = path == null ? null : candidates.FirstOrDefault(x => MatchesExecutable(path, x.Path));
                    if (match != null && path != null) active.Add((match, process.Id, path));
                }
                catch (Exception ex) when (ex is InvalidOperationException or System.ComponentModel.Win32Exception) { }
            }
        }
        finally { foreach (var process in processes) process.Dispose(); }
        if (active.Count == 0) return null;
        GetWindowThreadProcessId(GetForegroundWindow(), out var foreground);
        var selected = active.FirstOrDefault(x => x.Process == foreground);
        if (selected.Game == null) selected = active.OrderBy(x => Array.IndexOf(games, x.Game)).First();
        return new GameInfo(0, selected.Game.Name, selected.Image, selected.Game.Id);
    }

    public SystemLoad Sample(bool cpu, bool memory)
    {
        int? load = null;
        if (cpu && GetSystemTimes(out var idle, out var kernel, out var user))
        {
            var total = kernel + user;
            if (previousCpu is { } p && total > p.Total && idle >= p.Idle)
                load = (int)Math.Round(Math.Clamp(100 * (1 - (idle - p.Idle) / (double)(total - p.Total)), 0, 100));
            previousCpu = (total, idle);
        }
        else previousCpu = null;
        var state = new MemoryStatus { Length = (uint)Marshal.SizeOf<MemoryStatus>() };
        if (memory && GlobalMemoryStatusEx(ref state))
            return new(load, (int)state.Load, (state.TotalPhysical - state.AvailablePhysical) / 1073741824d);
        return new(load, null, null);
    }

    internal static bool PathEquals(string a, string b) => string.Equals(Path.GetFullPath(a), Path.GetFullPath(b), StringComparison.OrdinalIgnoreCase);

    internal static bool MatchesExecutable(string running, string saved)
    {
        if (PathEquals(running, saved)) return true;
        if (!Path.GetFileName(running).Equals(Path.GetFileName(saved), StringComparison.OrdinalIgnoreCase)) return false;
        var runningDirectory = Path.GetDirectoryName(running);
        var savedDirectory = Path.GetDirectoryName(saved);
        if (runningDirectory == null || savedDirectory == null) return false;

        // Versioned desktop installs retain a stable launcher in the root and run the same EXE from app-<version>.
        // A versioned directory is the only accepted alias. A same-named EXE in an unrelated location never matches.
        var runningRoot = VersionedRoot(runningDirectory);
        var savedRoot = VersionedRoot(savedDirectory);
        return (runningRoot != null || savedRoot != null) &&
            PathEquals(runningRoot ?? runningDirectory, savedRoot ?? savedDirectory);
    }

    private static string? VersionedRoot(string directory)
    {
        var folder = Path.GetFileName(directory);
        if (!folder.StartsWith("app-", StringComparison.OrdinalIgnoreCase)) return null;
        var version = folder[4..].Split('-', 2)[0];
        return Version.TryParse(version, out _) ? Path.GetDirectoryName(directory) : null;
    }

    private static string? ImagePath(int id)
    {
        var h = OpenProcess(0x1000, false, id);
        if (h == IntPtr.Zero) return null;
        try
        {
            var buffer = new StringBuilder(2048);
            var length = buffer.Capacity;
            return QueryFullProcessImageName(h, 0, buffer, ref length) ? buffer.ToString() : null;
        }
        finally { CloseHandle(h); }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MemoryStatus
    {
        public uint Length, Load;
        public ulong TotalPhysical, AvailablePhysical, TotalPage, AvailablePage, TotalVirtual, AvailableVirtual, AvailableExtended;
    }

    [DllImport("kernel32.dll")] private static extern bool GetSystemTimes(out ulong idle, out ulong kernel, out ulong user);
    [DllImport("kernel32.dll")] private static extern bool GlobalMemoryStatusEx(ref MemoryStatus state);
    [DllImport("kernel32.dll")] private static extern IntPtr OpenProcess(uint access, bool inherit, int id);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] private static extern bool QueryFullProcessImageName(IntPtr process, int flags, StringBuilder path, ref int length);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr h, out int pid);
}
