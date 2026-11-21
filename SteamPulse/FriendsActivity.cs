using System;
using System.Collections.Generic;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Threading.Tasks;
using HtmlAgilityPack;
using Microsoft.Win32;

namespace SteamPulse;

public record FriendGame(string Name, string Game);
public record FriendsSnapshot(bool Available, FriendGame[] Playing)
{
    public static readonly FriendsSnapshot Unavailable = new(false, []);
}

internal static class FriendsActivity
{
    private static readonly HttpClient Http = new(new HttpClientHandler
    {
        UseCookies = false, AutomaticDecompression = DecompressionMethods.All
    }) { Timeout = TimeSpan.FromSeconds(10), MaxResponseContentBufferSize = 5 * 1024 * 1024 };

    public static ulong ActiveAccount()
    {
        using var key = Registry.CurrentUser.OpenSubKey(@"Software\Valve\Steam\ActiveProcess");
        return key?.GetValue("ActiveUser") is int user && user != 0 ? 76561197960265728UL + unchecked((uint)user) : 0;
    }

    public static async Task<FriendsSnapshot> Read(ulong account)
    {
        if (account == 0) return FriendsSnapshot.Unavailable;
        try
        {
            var html = await Http.GetStringAsync($"https://steamcommunity.com/profiles/{account}/friends/?l=english");
            return Parse(html);
        }
        catch (Exception ex) when (ex is HttpRequestException or TaskCanceledException)
        {
            return FriendsSnapshot.Unavailable;
        }
    }

    internal static FriendsSnapshot Parse(string html)
    {
        var doc = new HtmlDocument();
        doc.LoadHtml(html);
        var list = doc.GetElementbyId("friends_list");
        if (list == null) return FriendsSnapshot.Unavailable;
        var entries = list.Descendants("div").Where(x => HasClass(x, "friend_block_v2")).ToArray();
        var found = new List<FriendGame>();
        foreach (var entry in entries)
        {
            if (!HasClass(entry, "in-game") && !HasClass(entry, "ingame")) continue;
            var content = entry.Descendants("div").FirstOrDefault(x => HasClass(x, "friend_block_content"));
            if (content == null) continue;
            var detail = content.Descendants("span").FirstOrDefault(x => HasClass(x, "friend_small_text"));
            var name = Clean(string.Concat(content.ChildNodes.TakeWhile(x => x.Name != "br").Select(x => x.InnerText)));
            var game = detail == null ? "" : string.Join(" · ", Lines(detail).Where(x =>
                !x.Equals("In-Game", StringComparison.OrdinalIgnoreCase) && !x.Equals("In Game", StringComparison.OrdinalIgnoreCase)).Take(1));
            if (game.Length == 0)
                game = string.Join(" · ", Lines(content).Skip(1).Where(x =>
                    !x.Equals("In-Game", StringComparison.OrdinalIgnoreCase) && !x.Equals("In Game", StringComparison.OrdinalIgnoreCase)).Take(1));
            if (name.Length > 0 && game.Length > 0) found.Add(new(name, game));
        }
        return new(true, found.Distinct().OrderBy(x => x.Name, StringComparer.OrdinalIgnoreCase).ToArray());
    }

    private static bool HasClass(HtmlNode node, string token) => node.GetAttributeValue("class", "")
        .Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries).Contains(token, StringComparer.OrdinalIgnoreCase);

    private static IEnumerable<string> Lines(HtmlNode node) => string.Concat(node.ChildNodes.Select(Text))
        .Split('\n').Select(Clean).Where(x => x.Length > 0);

    private static string Text(HtmlNode node) => node.Name == "br" ? "\n" :
        node.NodeType == HtmlNodeType.Text ? node.InnerText.Replace('\n', ' ').Replace('\r', ' ') :
        string.Concat(node.ChildNodes.Select(Text));

    private static string Clean(string text)
    {
        var value = string.Join(" ", WebUtility.HtmlDecode(text).Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries));
        return value.Length > 128 ? value[..128] : value;
    }
}
