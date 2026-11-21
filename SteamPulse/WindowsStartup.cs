using System;
using System.IO;
using Microsoft.Win32;

namespace SteamPulse;

internal static class WindowsStartup
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string Entry = "SteamPulse";

    public static bool Enabled()
    {
        try
        {
            using var run = Registry.CurrentUser.OpenSubKey(RunKey);
            return run?.GetValue(Entry) is string value &&
                value.Equals(LaunchCommand, StringComparison.OrdinalIgnoreCase);
        }
        catch (Exception ex) when (ex is UnauthorizedAccessException or IOException or System.Security.SecurityException)
        {
            return false;
        }
    }

    public static bool Set(bool enabled)
    {
        try
        {
            using var run = enabled ? Registry.CurrentUser.CreateSubKey(RunKey) : Registry.CurrentUser.OpenSubKey(RunKey, writable: true);
            if (enabled) run!.SetValue(Entry, LaunchCommand, RegistryValueKind.String);
            else run?.DeleteValue(Entry, throwOnMissingValue: false);
            return true;
        }
        catch (Exception ex) when (ex is UnauthorizedAccessException or IOException or System.Security.SecurityException)
        {
            return false;
        }
    }

    private static string LaunchCommand => "\"" + Environment.ProcessPath + "\"";
}
