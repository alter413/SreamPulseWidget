using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Net.NetworkInformation;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Microsoft.Win32;

namespace SteamPulse;

public sealed class Vdf
{
    private readonly Dictionary<string, Vdf> children = new(StringComparer.OrdinalIgnoreCase);
    private readonly Dictionary<string, string> values = new(StringComparer.OrdinalIgnoreCase);

    public IEnumerable<string> Keys() => children.Keys;
    public Vdf? Child(string key) => children.GetValueOrDefault(key);
    public string? Value(string key) => values.GetValueOrDefault(key);
    public long Number(string key) => long.TryParse(Value(key), out var value) ? Math.Max(0, value) : 0;

    public IEnumerable<string> Libraries() => children.Values.Select(x => x.Value("path"))
        .OfType<string>().Concat(values.Where(x => uint.TryParse(x.Key, out _)).Select(x => x.Value));

    private static readonly Regex Token = new("""//[^\r\n]*|"((?:\\.|[^"\\])*)"|[{}]|[^\s{}"]+""");
    private static readonly Regex Escape = new("""\\([\\"])""");

    public static Vdf Parse(string raw)
    {
        var root = new Vdf();
        var stack = new Stack<Vdf>();
        stack.Push(root);
        string? key = null;
        foreach (Match m in Token.Matches(raw))
        {
            if (m.Value.StartsWith("//", StringComparison.Ordinal)) continue;
            if (m.Value == "}" && stack.Count > 1) { stack.Pop(); key = null; continue; }
            if (m.Value == "{" && key != null)
            {
                var child = new Vdf();
                stack.Peek().children[key] = child;
                stack.Push(child);
                key = null;
                continue;
            }
            var token = m.Groups[1].Success ? Escape.Replace(m.Groups[1].Value, "$1") : m.Value;
            if (key == null) key = token;
            else { stack.Peek().values[key] = token; key = null; }
        }
        return root;
    }
}

public record ContentSample(uint Id = 0, long Done = 0, long Total = 0, DateTime Anchor = default,
    double Integrated = 0, double Speed = 0, DateTime LastRate = default, double Estimated = 0)
{
    private static readonly Regex Line = new(@"^\[(.{19})\]\s+(?:AppID (\d+)\s+(.*)|Current download rate:\s+([\d.]+)\s+([KMG]?bps))", RegexOptions.IgnoreCase);
    private static readonly Regex Bytes = new(@"(?:update started|update progress)\s*:\s*download (\d+)/(\d+)", RegexOptions.IgnoreCase);
    private static readonly Regex AdjustedRate = new(@"rate was [\d.]+, now ([\d.]+)", RegexOptions.IgnoreCase);

    public double CurrentSpeed(DateTime now) => Id != 0 && (now - LastRate).TotalSeconds is >= 0 and < 95 ? Speed : 0;
    public double Estimate(DateTime now) => Math.Clamp(Integrated +
        Speed * Math.Clamp((now - LastRate).TotalSeconds, 0, 68), 0, Math.Max(0, (double)Total - Done));

    public static ContentSample Parse(string raw, DateTime now, ContentSample? initial = null)
    {
        var s = initial ?? new ContentSample();
        foreach (var line in raw.Split('\n'))
        {
            var adjusted = AdjustedRate.Match(line);
            var m = Line.Match(line);
            if (!m.Success && !adjusted.Success) continue;
            if (line.Length < 21 || !DateTime.TryParseExact(line.AsSpan(1, 19), "yyyy-MM-dd HH:mm:ss",
                CultureInfo.InvariantCulture, DateTimeStyles.None, out var at)) continue;
            if (adjusted.Success && !m.Success)
            {
                if (s.Id != 0 && double.TryParse(adjusted.Groups[1].Value, NumberStyles.Float,
                    CultureInfo.InvariantCulture, out var adjustedSpeed))
                    s = UpdateRate(s, at, adjustedSpeed * 125000);
                continue;
            }
            if (uint.TryParse(m.Groups[2].Value, out var id))
            {
                var message = m.Groups[3].Value.Trim();
                if (message.Contains("update changed", StringComparison.OrdinalIgnoreCase))
                {
                    var flags = message[(message.IndexOf(':') + 1)..].Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries);
                    var active = flags.Any(x => x is "Running Update" or "Running" or "Downloading" or "Staging" or "Committing");
                    var stopped = flags.Any(x => x is "Stopping" or "None" or "Paused");
                    if (active && !stopped && id != s.Id) s = new ContentSample(id, Anchor: at);
                    if ((!active || stopped) && id == s.Id) s = new ContentSample();
                }
                var bytes = Bytes.Match(message);
                if (bytes.Success && long.TryParse(bytes.Groups[1].Value, out var done)
                    && long.TryParse(bytes.Groups[2].Value, out var total))
                    s = new ContentSample(id, done, total, at);
                if (id == s.Id && (message.StartsWith("scheduler finished") || message.StartsWith("update canceled") ||
                    message.StartsWith("finished update"))) s = new ContentSample();
            }
            else if (s.Id != 0 && double.TryParse(m.Groups[4].Value, NumberStyles.Float, CultureInfo.InvariantCulture, out var rate))
            {
                var unit = m.Groups[5].Value.ToLowerInvariant();
                var scale = unit switch { "kbps" => 125d, "mbps" => 125000d, "gbps" => 125000000d, _ => 0.125d };
                s = UpdateRate(s, at, rate * scale);
            }
        }
        return s with { Estimated = s.Done + s.Estimate(now) };
    }

    private static ContentSample UpdateRate(ContentSample sample, DateTime at, double speed)
    {
        var previous = sample.LastRate == default ? sample.Anchor : sample.LastRate;
        var elapsed = Math.Clamp((at - previous).TotalSeconds, 0, 90);
        return sample with { Speed = speed, LastRate = at, Integrated = sample.Integrated + speed * elapsed };
    }
}

public record GameInfo(uint Id, string Name, string? Art, string? LocalId = null);
public record DownloadInfo(GameInfo App, double? Percent, bool Estimated, double? Speed, bool Installing = false, TimeSpan? Remaining = null)
{
    public string Name => App.Name;
}

public record SteamStatus(string? Root, bool Active, GameInfo? Game, DownloadInfo? Download, double? Network)
{
    public SystemLoad? Load { get; init; }
    public FriendsSnapshot? Friends { get; init; }
    public int FriendOffset { get; init; }
    public static readonly SteamStatus Empty = new(null, false, null, null, null);
}

public sealed class SteamReader
{
    private const string SteamKey = @"Software\Valve\Steam";
    private string? root;
    private DateTime nextDiscovery;
    private string[] libraries = [];
    private DateTime nextLibraries;
    private string? logPath;
    private long logLength = -1;
    private DateTime logWrite;
    private ContentSample sample = new();
    private readonly Dictionary<uint, (string? Art, DateTime Until)> artCache = new();
    private readonly Dictionary<string, (DateTime Write, Vdf State)> manifestCache = new();
    private long previousBytes;
    private DateTime networkAt;
    private uint previousId;
    private long lastDone;
    private DateTime exactAt;
    private DateTime sampleAnchor;
    private double exactSpeed;

    public SteamStatus Read(bool network = false)
    {
        var now = DateTime.Now;
        var rate = network ? ReadNetwork(now) : null;
        if (now >= nextDiscovery)
        {
            nextDiscovery = now.AddSeconds(8);
            var discovered = Discover();
            if (!string.Equals(discovered, root, StringComparison.OrdinalIgnoreCase))
            {
                root = discovered;
                nextLibraries = default;
                artCache.Clear();
                manifestCache.Clear();
            }
        }
        if (root == null) return SteamStatus.Empty with { Network = rate };

        using var steam = Registry.CurrentUser.OpenSubKey(SteamKey);
        using var activeKey = steam?.OpenSubKey("ActiveProcess");
        var active = Running(Convert.ToInt32(activeKey?.GetValue("pid") ?? 0));
        if (!active)
        {
            using var processes = new ProcessList(Process.GetProcessesByName("steam"));
            active = processes.Items.Any(p => p.SessionId == ProcessSession.Id);
        }
        if (!active) return new(root, false, null, null, rate);

        if (now >= nextLibraries)
        {
            nextLibraries = now.AddSeconds(30);
            libraries = Vdf.Parse(ReadShared(Path.Combine(root, "steamapps", "libraryfolders.vdf")))
                .Child("libraryfolders")?.Libraries().Prepend(root).Distinct(StringComparer.OrdinalIgnoreCase).ToArray() ?? [root];
        }
        var gameId = ToId(steam?.GetValue("RunningAppID"));
        using (var gameKey = steam?.OpenSubKey($"Apps\\{gameId}"))
        {
            if (gameKey?.GetValue("Running") is int running && running == 0) gameId = 0;
        }
        if (gameId == 0)
        {
            using var apps = steam?.OpenSubKey("Apps");
            foreach (var key in apps?.GetSubKeyNames() ?? [])
            {
                using var app = apps?.OpenSubKey(key);
                if (app?.GetValue("Running") is int visible && visible == 1 && uint.TryParse(key, out gameId)) break;
            }
        }

        ReadContent(Path.Combine(root, "logs", "content_log.txt"));
        var game = gameId == 0 ? null : Game(gameId);
        var download = sample.Id == 0 ? null : ReadDownload(now);
        if (download == null) previousId = 0;
        return new(root, true, game, download, rate);
    }

    private DownloadInfo ReadDownload(DateTime now)
    {
        var manifest = Manifest(sample.Id);
        var counter = manifest?.Number("BytesDownloaded") ?? 0;
        var manifestTotal = manifest?.Number("BytesToDownload") ?? 0;
        var total = sample.Total > 0 ? sample.Total : manifestTotal;
        var done = Math.Max(sample.Done, counter);
        var speed = sample.CurrentSpeed(now);
        if (previousId != sample.Id || sampleAnchor != sample.Anchor || done < lastDone)
        {
            previousId = sample.Id;
            sampleAnchor = sample.Anchor;
            lastDone = done;
            exactAt = now;
            exactSpeed = 0;
        }
        else if (done > lastDone)
        {
            exactSpeed = (done - lastDone) / Math.Max(1, (now - exactAt).TotalSeconds);
            lastDone = done;
            exactAt = now;
        }
        if (speed == 0 && (now - exactAt).TotalSeconds < 6) speed = exactSpeed;

        double? progress = null;
        var estimated = false;
        if (total > 0)
        {
            var extrapolation = sample.Done + sample.Estimate(now);
            if (done < total && extrapolation > done)
            {
                progress = Math.Min(99.8, extrapolation * 100 / total);
                estimated = true;
            }
            else progress = Math.Clamp(done * 100.0 / total, 0, 100);
        }
        var installing = total > 0 && done >= total;
        var received = progress.HasValue ? total * progress.Value / 100 : done;
        var remaining = EstimateRemaining(total, received, speed, installing);
        return new DownloadInfo(Game(sample.Id, manifest), progress, estimated, speed > 0 && !installing ? speed : null, installing, remaining);
    }

    internal static TimeSpan? EstimateRemaining(long total, double received, double speed, bool installing)
    {
        if (installing || total <= 0 || received >= total || speed <= 0) return null;
        var seconds = (total - received) / speed;
        return double.IsFinite(seconds) && seconds <= TimeSpan.FromDays(365).TotalSeconds
            ? TimeSpan.FromSeconds(Math.Ceiling(seconds)) : null;
    }

    private void ReadContent(string path)
    {
        try
        {
            var info = new FileInfo(path);
            if (!info.Exists) { sample = new(); logLength = -1; return; }
            if (path == logPath && info.Length == logLength && info.LastWriteTimeUtc == logWrite) return;
            var incremental = path == logPath && logLength >= 0 && info.Length > logLength &&
                info.Length - logLength <= 524288;
            var start = incremental ? logLength : Math.Max(0, info.Length - 524288);
            var text = ReadShared(path, start, info.Length - start);
            if (text.Length == 0) return;
            if (!incremental && start > 0)
            {
                var firstNewline = text.IndexOf('\n');
                if (firstNewline >= 0) text = text[(firstNewline + 1)..];
            }
            // Steam writes complete records. Retry a partially written last line on the next poll.
            if (!text.EndsWith('\n')) return;
            sample = ContentSample.Parse(text, DateTime.Now, incremental ? sample : null);
            logPath = path;
            logLength = info.Length;
            logWrite = info.LastWriteTimeUtc;
        }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
    }

    private GameInfo Game(uint id, Vdf? manifest = null)
    {
        manifest ??= Manifest(id);
        using var key = Registry.CurrentUser.OpenSubKey($"{SteamKey}\\Apps\\{id}");
        var name = manifest?.Value("name") ?? key?.GetValue("Name") as string ?? $"Steam App {id}";
        if (artCache.TryGetValue(id, out var entry) && entry.Until > DateTime.Now)
            return new(id, name, entry.Art);

        var cache = Path.Combine(root!, "appcache", "librarycache");
        var directory = Path.Combine(cache, id.ToString());
        string? art = FirstFile(directory, "*.jpg", SearchOption.TopDirectoryOnly)
            ?? FirstFile(directory, "library_600x900.jpg", SearchOption.AllDirectories)
            ?? FirstFile(directory, "library_header.jpg", SearchOption.AllDirectories);
        foreach (var suffix in new[] { "_icon.jpg", "_library_600x900.jpg", "_header.jpg" })
        {
            if (art != null) break;
            var candidate = Path.Combine(cache, id + suffix);
            if (File.Exists(candidate)) art = candidate;
        }
        artCache[id] = (art, DateTime.Now.AddMinutes(1));
        return new(id, name, art);
    }

    private Vdf? Manifest(uint id)
    {
        foreach (var library in libraries)
        {
            var path = Path.Combine(library, "steamapps", $"appmanifest_{id}.acf");
            if (!File.Exists(path)) continue;
            var write = File.GetLastWriteTimeUtc(path);
            if (manifestCache.TryGetValue(path, out var cache) && cache.Write == write) return cache.State;
            var state = Vdf.Parse(ReadShared(path)).Child("AppState");
            if (state?.Value("name") != null)
            {
                manifestCache[path] = (write, state);
                return state;
            }
            return cache.State;
        }
        return null;
    }

    private static string? FirstFile(string path, string pattern, SearchOption search)
    {
        try { return Directory.Exists(path) ? Directory.EnumerateFiles(path, pattern, search).FirstOrDefault() : null; }
        catch (IOException) { return null; }
        catch (UnauthorizedAccessException) { return null; }
    }

    private double? ReadNetwork(DateTime now)
    {
        var bytes = NetworkInterface.GetAllNetworkInterfaces().Where(x => x.OperationalStatus == OperationalStatus.Up
            && x.NetworkInterfaceType is not (NetworkInterfaceType.Loopback or NetworkInterfaceType.Tunnel))
            .Sum(x => x.GetIPStatistics().BytesReceived);
        var elapsed = (now - networkAt).TotalSeconds;
        double? rate = elapsed is > 0.1 and < 15 && bytes >= previousBytes ? (bytes - previousBytes) / elapsed : null;
        previousBytes = bytes;
        networkAt = now;
        return rate;
    }

    private static uint ToId(object? value) => uint.TryParse(Convert.ToString(value), out var id) ? id : 0;

    private static bool Running(int pid)
    {
        if (pid <= 0) return false;
        try { using var p = Process.GetProcessById(pid); return !p.HasExited && p.ProcessName.Equals("steam", StringComparison.OrdinalIgnoreCase); }
        catch (ArgumentException) { return false; }
        catch (InvalidOperationException) { return false; }
        catch (System.ComponentModel.Win32Exception) { return false; }
    }

    private static string? Discover()
    {
        var candidates = new[]
        {
            Registry.GetValue($@"HKEY_CURRENT_USER\{SteamKey}", "SteamPath", null) as string,
            Registry.GetValue(@"HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath", null) as string,
            Registry.GetValue(@"HKEY_LOCAL_MACHINE\SOFTWARE\Valve\Steam", "InstallPath", null) as string,
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86), "Steam")
        };
        foreach (var candidate in candidates.OfType<string>())
            if (File.Exists(Path.Combine(candidate, "steam.exe"))) return Path.GetFullPath(candidate);

        using var processes = new ProcessList(Process.GetProcessesByName("steam"));
        foreach (var p in processes.Items)
        {
            try { if (p.MainModule is { } module) return Path.GetDirectoryName(module.FileName); }
            catch (System.ComponentModel.Win32Exception) { }
            catch (InvalidOperationException) { }
        }
        return null;
    }

    private static string ReadShared(string path, long start = 0, long count = -1)
    {
        try
        {
            using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            if (count >= 0)
            {
                stream.Position = start;
                var buffer = new byte[(int)count];
                var read = stream.ReadAtLeast(buffer, buffer.Length, false);
                return Encoding.UTF8.GetString(buffer, 0, read);
            }
            using var reader = new StreamReader(stream);
            return reader.ReadToEnd();
        }
        catch (IOException) { return ""; }
        catch (UnauthorizedAccessException) { return ""; }
    }

    private sealed class ProcessList(Process[] processes) : IDisposable
    {
        public Process[] Items => processes;
        public void Dispose() { foreach (var p in processes) p.Dispose(); }
    }
}

public record Offer(uint Id, string Name, int Discount, string? Image);

public static class StoreClient
{
    internal static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(10) };

    private static string StoreLanguage() => Localization.Language switch
    {
        "ru" => "russian", "es" => "spanish", "de" => "german", "fr" => "french", "pt" => "brazilian",
        "ja" => "japanese", "ko" => "koreana", "zh" => "schinese", "id" => "indonesian", _ => "english"
    };

    public static async System.Threading.Tasks.Task<Offer[]> Offers(string? country = null)
    {
        var cc = string.IsNullOrWhiteSpace(country) ? RegionInfo.CurrentRegion.TwoLetterISORegionName : country;
        using var stream = await Http.GetStreamAsync($"https://store.steampowered.com/api/featuredcategories?cc={Uri.EscapeDataString(cc)}&l={StoreLanguage()}");
        using var json = await JsonDocument.ParseAsync(stream);
        if (!json.RootElement.TryGetProperty("specials", out var specials) || specials.ValueKind != JsonValueKind.Object
            || !specials.TryGetProperty("items", out var items) || items.ValueKind != JsonValueKind.Array) return [];
        return items.EnumerateArray()
            .Where(x => x.TryGetProperty("discount_percent", out var d) && d.TryGetInt32(out var n) && n > 0)
            .Select(x => new Offer(x.GetProperty("id").GetUInt32(), x.GetProperty("name").GetString() ?? "Steam",
                x.GetProperty("discount_percent").GetInt32(), x.TryGetProperty("small_capsule_image", out var art) ? art.GetString() : null))
            .ToArray();
    }
}

internal static class ProcessSession
{
    public static readonly int Id = Process.GetCurrentProcess().SessionId;
}

