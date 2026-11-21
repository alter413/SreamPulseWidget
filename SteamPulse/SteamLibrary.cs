using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using Microsoft.Win32;

namespace SteamPulse;

internal sealed record LibraryGame(uint Id, string Name, bool Installed);

internal static class SteamLibrary
{
    public static LibraryGame[] Read(string? root)
    {
        if (root == null) return [];
        var installed = new Dictionary<uint, string>();
        var libraries = Vdf.Parse(Shared(Path.Combine(root, "steamapps", "libraryfolders.vdf")))
            .Child("libraryfolders")?.Libraries().Prepend(root).Distinct(StringComparer.OrdinalIgnoreCase) ?? [root];
        foreach (var path in libraries)
        {
            var directory = Path.Combine(path, "steamapps");
            try
            {
                if (!Directory.Exists(directory)) continue;
                foreach (var file in Directory.EnumerateFiles(directory, "appmanifest_*.acf"))
                {
                    var state = Vdf.Parse(Shared(file)).Child("AppState");
                    if (uint.TryParse(state?.Value("appid"), out var id) && state?.Value("name") is { Length: > 0 } name)
                        installed[id] = name;
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { }
        }

        const string key = @"Software\Valve\Steam";
        var known = new Dictionary<uint, string>(installed);
        using (var apps = Registry.CurrentUser.OpenSubKey(key + @"\Apps"))
        {
            foreach (var item in apps?.GetSubKeyNames() ?? [])
            {
                if (!uint.TryParse(item, out var id)) continue;
                using var app = apps?.OpenSubKey(item);
                known.TryAdd(id, app?.GetValue("Name") as string ?? "");
            }
        }
        using (var active = Registry.CurrentUser.OpenSubKey(key + @"\ActiveProcess"))
        {
            if (active?.GetValue("ActiveUser") is int account && account != 0)
            {
                var source = Shared(Path.Combine(root, "userdata", ((uint)account).ToString(), "config", "localconfig.vdf"));
                var apps = Vdf.Parse(source).Child("UserLocalConfigStore")?.Child("Software")?.Child("Valve")?.Child("Steam")?.Child("apps");
                foreach (var id in apps?.Keys() ?? [])
                    if (uint.TryParse(id, out var n)) known.TryAdd(n, "");
            }
        }
        var metadata = CachedNames(Path.Combine(root, "appcache", "appinfo.vdf"), known.Keys.ToHashSet());
        return known.Where(x => x.Key != 228980)
            .Select(x =>
            {
                metadata.TryGetValue(x.Key, out var info);
                var title = info.Name ?? x.Value;
                return (Title: title, info.Type, x.Key, Installed: installed.ContainsKey(x.Key));
            })
            .Where(x => x.Title.Length > 0 && (x.Type == null || x.Type is "game" or "application" or "demo" or "mod" or "beta"))
            .Select(x => new LibraryGame(x.Key, x.Title, x.Installed))
            .OrderBy(x => x.Name, StringComparer.CurrentCultureIgnoreCase).ToArray();
    }

    private static Dictionary<uint, (string? Name, string? Type)> CachedNames(string path, HashSet<uint> wanted)
    {
        var result = new Dictionary<uint, (string?, string?)>();
        try
        {
            using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            using var r = new BinaryReader(stream, Encoding.UTF8);
            var magic = r.ReadUInt32();
            if (magic is not (0x07564428 or 0x07564429)) return result;
            r.ReadUInt32();
            string[]? keys = null;
            var endOfApps = stream.Length;
            if (magic == 0x07564429)
            {
                endOfApps = r.ReadInt64();
                if (endOfApps < 16 || endOfApps > stream.Length - 4) return result;
                stream.Position = endOfApps;
                var count = r.ReadUInt32();
                if (count > 250000) return result;
                keys = new string[count];
                for (var i = 0; i < keys.Length; i++)
                    keys[i] = NullString(r, stream.Length);
                stream.Position = 16;
            }
            while (stream.Position + 8 <= endOfApps)
            {
                var id = r.ReadUInt32();
                if (id == 0) break;
                var length = r.ReadUInt32();
                var end = stream.Position + length;
                if (length < 60 || end > endOfApps) break;
                if (wanted.Contains(id))
                {
                    stream.Position += 60;
                    try
                    {
                        string? name = null;
                        string? type = null;
                        ReadNode(r, end, keys, "", 0, ref name, ref type);
                        result[id] = (name, type?.ToLowerInvariant());
                    }
                    catch (InvalidDataException) { }
                }
                stream.Position = end;
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidDataException) { }
        return result;
    }

    private static void ReadNode(BinaryReader reader, long end, string[]? keys, string parent,
        int depth, ref string? name, ref string? type)
    {
        if (depth > 24) throw new InvalidDataException();
        var stream = reader.BaseStream;
        while (stream.Position < end)
        {
            var token = reader.ReadByte();
            if (token is 8 or 11) return;
            var key = keys == null ? NullString(reader, end) : Key(reader.ReadUInt32(), keys);
            if (token == 0)
                ReadNode(reader, end, keys, key, depth + 1, ref name, ref type);
            else if (token == 1)
            {
                var value = NullString(reader, end);
                if (parent == "common" && key == "name") name = value;
                if (parent == "common" && key == "type") type = value;
            }
            else if (token is 2 or 3 or 4 or 6) stream.Position += 4;
            else if (token is 7 or 10) stream.Position += 8;
            else throw new InvalidDataException();
            if (stream.Position > end) throw new InvalidDataException();
            if (name != null && type != null) return;
        }
    }

    private static string Key(uint index, string[] keys) => index < keys.Length ? keys[index] : throw new InvalidDataException();

    private static string NullString(BinaryReader r, long end)
    {
        var bytes = new List<byte>();
        while (r.BaseStream.Position < end && bytes.Count < 65_536)
        {
            var value = r.ReadByte();
            if (value == 0) return Encoding.UTF8.GetString(bytes.ToArray());
            bytes.Add(value);
        }
        throw new InvalidDataException();
    }

    private static string Shared(string path)
    {
        try
        {
            using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            using var reader = new StreamReader(file);
            return reader.ReadToEnd();
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException) { return ""; }
    }
}
