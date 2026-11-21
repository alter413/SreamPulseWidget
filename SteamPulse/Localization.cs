using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Reflection;
using System.Text.Json;

namespace SteamPulse;

internal static class Localization
{
    internal static readonly Dictionary<string, Dictionary<string, string>> All = Load();
    public static string Language { get; private set; } = "en";
    public static CultureInfo Culture => CultureInfo.GetCultureInfo(Language);

    public static void Set(string? preference)
    {
        var code = string.IsNullOrEmpty(preference) ? CultureInfo.CurrentUICulture.TwoLetterISOLanguageName : preference;
        Language = All.ContainsKey(code) ? code : "en";
    }

    public static string T(string key) => All[Language].GetValueOrDefault(key) ?? All["en"].GetValueOrDefault(key) ?? key;

    private static Dictionary<string, Dictionary<string, string>> Load()
    {
        using var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream("SteamPulse.Locales.json")
            ?? throw new InvalidDataException("Locales.json");
        return JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(stream)
            ?? throw new InvalidDataException("Locales.json");
    }
}
