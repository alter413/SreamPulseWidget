using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;
using System.Windows.Media;

namespace SteamPulse;

internal sealed record Preferences
{
    public bool Widget { get; set; } = true;
    public bool Downloads { get; set; } = true;
    public bool Games { get; set; } = true;
    public bool AirMode { get; set; }
    public bool WidgetBorder { get; set; } = true;
    public bool Icon { get; set; } = true;
    public bool Name { get; set; } = true;
    public bool Speed { get; set; } = true;
    public bool RemainingTime { get; set; } = true;
    public bool Percent { get; set; } = true;
    public bool Bar { get; set; } = true;
    public bool Network { get; set; }
    public bool Discounts { get; set; } = true;
    public bool DiscountCard { get; set; } = true;
    public bool Gradient { get; set; }
    public bool Glass { get; set; }
    public bool ClickThrough { get; set; }
    public bool WidgetFrozen { get; set; }
    public bool Bits { get; set; }
    public bool SettingsGlass { get; set; } = true;
    public int Placement { get; set; } = 2;
    public string MonitorName { get; set; } = "";
    public int MinDiscount { get; set; } = 25;
    public double Opacity { get; set; } = 0.90;
    public double Width { get; set; } = 315;
    public double Radius { get; set; } = 11;
    public double Offset { get; set; } = 14;
    public double GlassTint { get; set; } = 1;
    public double SettingsTint { get; set; } = .05;
    public double SettingsGlassDepth { get; set; } = -5;
    public double SettingsGlassBlur { get; set; } = 6.8;
    public double SettingsGlassDispersion { get; set; } = 3;
    public double SettingsGlassLight { get; set; } = 360;
    public double SettingsShine { get; set; }
    public double GlassDepth { get; set; } = -5;
    public double GlassBlur { get; set; } = 6.8;
    public double GlassDispersion { get; set; } = 3;
    public double GlassLight { get; set; } = 360;
    public double Angle { get; set; } = 20;
    public double Shine { get; set; }
    public string Background { get; set; } = "#101929";
    public string BackgroundEnd { get; set; } = "#1F3851";
    public string Accent { get; set; } = "#54DAFE";
    public string AccentEnd { get; set; } = "#81A5FF";
    public string Text { get; set; } = "#F1F7FF";
    public string Country { get; set; } = "";
    public bool LightTheme { get; set; }
    public double? HorizontalPosition { get; set; }
    public string Language { get; set; } = "en";
    public LocalGame[] LocalGames { get; set; } = [];
    public bool MonitorSystem { get; set; } = true;
    public bool MonitorCpu { get; set; } = true;
    public bool MonitorRam { get; set; } = true;
    public bool RamGiB { get; set; }
    public int MetricsInterval { get; set; } = 2;
    public bool Friends { get; set; } = true;
    public int FriendCycle { get; set; } = 6;
    public uint[] IgnoredGames { get; set; } = [];
    public int FormatVersion { get; set; } = 5;
    public Dictionary<uint, int> SeenOffers { get; set; } = new();

    public bool Ignores(uint id) => Array.IndexOf(IgnoredGames, id) >= 0;

    public GlassOptions Optics(bool settings, double dpi = 1) => settings
        ? new(SettingsGlassDepth, SettingsGlassBlur * dpi, SettingsGlassDispersion, SettingsShine, SettingsGlassLight, 20 * dpi)
        : new(GlassDepth, GlassBlur * dpi, GlassDispersion, Shine, GlassLight, Math.Min(11, Radius) * dpi);

    public static string Folder { get; set; } = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SteamPulse");
    private static string FileName => Path.Combine(Folder, "settings.json");

    public static Preferences Load()
    {
        try
        {
            var raw = File.ReadAllText(FileName);
            using var document = JsonDocument.Parse(raw);
            var p = JsonSerializer.Deserialize<Preferences>(raw) ?? new Preferences();
            if (!document.RootElement.TryGetProperty("FormatVersion", out var version))
            {
                var defaults = new Preferences();
                p.Glass = p.Gradient = p.ClickThrough = false;
                p.SettingsGlass = defaults.SettingsGlass;
                p.GlassTint = defaults.GlassTint;
                p.SettingsTint = defaults.SettingsTint;
                p.GlassDepth = defaults.GlassDepth;
                p.GlassBlur = defaults.GlassBlur;
                p.GlassDispersion = defaults.GlassDispersion;
                p.GlassLight = defaults.GlassLight;
                p.Shine = defaults.Shine;
            }
            if (version.ValueKind == JsonValueKind.Undefined || p.FormatVersion < 5)
            {
                p.SettingsGlassDepth = p.GlassDepth;
                p.SettingsGlassBlur = p.GlassBlur;
                p.SettingsGlassDispersion = p.GlassDispersion;
                p.SettingsGlassLight = p.GlassLight;
                p.SettingsShine = p.Shine;
            }
            p.FormatVersion = Math.Max(p.FormatVersion, 5);
            p.Opacity = Math.Clamp(p.Opacity, .10, 1);
            p.Width = Math.Clamp(p.Width, 200, 550);
            p.Radius = Math.Clamp(p.Radius, 0, 11);
            p.Offset = Math.Clamp(p.Offset, -1000, 1000);
            p.GlassTint = Math.Clamp(p.GlassTint, 0, 1);
            p.SettingsTint = Math.Clamp(p.SettingsTint, .05, 1);
            p.SettingsGlassDepth = Math.Clamp(p.SettingsGlassDepth, -5, 5);
            p.SettingsGlassBlur = Math.Clamp(p.SettingsGlassBlur, 0, 10);
            p.SettingsGlassDispersion = Math.Clamp(p.SettingsGlassDispersion, 0, 3);
            p.SettingsGlassLight = Math.Clamp(p.SettingsGlassLight, 0, 360);
            p.SettingsShine = Math.Clamp(p.SettingsShine, 0, 1);
            p.GlassDepth = Math.Clamp(p.GlassDepth, -5, 5);
            p.GlassBlur = Math.Clamp(p.GlassBlur, 0, 10);
            p.GlassDispersion = Math.Clamp(p.GlassDispersion, 0, 3);
            p.GlassLight = Math.Clamp(p.GlassLight, 0, 360);
            p.Shine = Math.Clamp(p.Shine, 0, 1);
            p.Angle = Math.Clamp(p.Angle, 0, 360);
            p.MinDiscount = Math.Clamp(p.MinDiscount, 5, 95);
            p.MonitorName ??= "";
            p.Placement = Math.Clamp(p.Placement, 0, 2);
            p.SeenOffers ??= new();
            p.IgnoredGames ??= [];
            p.LocalGames ??= [];
            p.MetricsInterval = Math.Clamp(p.MetricsInterval, 1, 5);
            p.FriendCycle = Math.Clamp(p.FriendCycle, 3, 20);
            p.Language ??= "en";
            if (p.HorizontalPosition is double x) p.HorizontalPosition = Math.Clamp(x, 0, 100);
            return p;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException)
        {
            return new Preferences();
        }
    }

    public void Save()
    {
        Directory.CreateDirectory(Folder);
        var temp = FileName + ".tmp";
        File.WriteAllText(temp, JsonSerializer.Serialize(this, new JsonSerializerOptions { WriteIndented = true }));
        File.Move(temp, FileName, true);
    }

    public static Color Color(string? hex, Color fallback)
    {
        try { return ColorConverter.ConvertFromString(hex ?? "") is Color c ? c : fallback; }
        catch (FormatException) { return fallback; }
    }
}