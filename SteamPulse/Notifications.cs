using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Net.Http;
using System.Text.Json;
using System.Threading.Tasks;
using System.Windows;
using static SteamPulse.Localization;
using Wf = System.Windows.Forms;

namespace SteamPulse;

internal sealed partial class PulseHost
{
    private Offer[] offers = [];
    private bool checkingStore;
    private string offerMessage = "";

    private async Task RefreshOffers(bool notify)
    {
        storeTimer.Interval = TimeSpan.FromMinutes(35);
        if (checkingStore || disposed || (notify && !settings.Discounts)) return;
        checkingStore = true;
        var country = settings.Country;
        try
        {
            var available = await StoreClient.Offers(country);
            if (disposed || country != settings.Country) return;
            offers = available.Where(x => x.Discount >= settings.MinDiscount).DistinctBy(x => x.Id)
                .OrderByDescending(x => x.Discount).ToArray();
            PopulateOffers(offers.Length == 0 ? "noOffers" : "");
            if (!notify || !settings.Discounts) return;
            var picks = offers.Where(x => !settings.SeenOffers.TryGetValue(x.Id, out var rate) || rate < x.Discount).ToArray();
            var featured = offers.Select(x => x.Id).ToHashSet();
            var old = settings.SeenOffers.Keys.Where(x => !featured.Contains(x)).ToArray();
            foreach (var id in old) settings.SeenOffers.Remove(id);
            foreach (var item in picks) settings.SeenOffers[item.Id] = item.Discount;
            if (old.Length > 0 || picks.Length > 0) Save();
            if (picks.FirstOrDefault() is not { } pick) return;
            cardOffer = pick;
            expires = DateTime.Now.AddSeconds(25);
            notificationId = pick.Id;
            tray.ShowBalloonTip(5500, T("sales"), pick.Name + $" · −{pick.Discount}%" +
                (picks.Length > 1 ? $"  •  +{picks.Length - 1}" : ""), Wf.ToolTipIcon.Info);
            RefreshWidget();
        }
        catch (Exception e) when (e is HttpRequestException or TaskCanceledException or JsonException or InvalidOperationException or
            KeyNotFoundException or FormatException)
        {
            if (!disposed) PopulateOffers("storeError");
        }
        finally { checkingStore = false; }
    }

    private void PopulateOffers(string message)
    {
        offerMessage = message;
        while (saleList.DropDownItems.Count > 0)
        {
            var old = saleList.DropDownItems[0];
            saleList.DropDownItems.RemoveAt(0);
            old.Dispose();
        }
        if (message.Length > 0) saleList.DropDownItems.Add(new Wf.ToolStripMenuItem(T(message)) { Enabled = false });
        else
        {
            foreach (var item in offers.Take(12))
                saleList.DropDownItems.Add($"{item.Name}  ·  −{item.Discount}%", null, (_, _) => OpenOffer(item.Id));
        }
        saleList.DropDownItems.Add(new Wf.ToolStripSeparator());
        saleList.DropDownItems.Add(T("checkNow"), null, async (_, _) => await RefreshOffers(false));
        saleList.DropDownItems.Add(T("allSales"), null, (_, _) => OpenUri("https://store.steampowered.com/search/?specials=1"));
    }

    private static void OpenOffer(uint id) => OpenUri($"https://store.steampowered.com/app/{id}/");

    private static void OpenUri(string url)
    {
        try { using var opened = Process.Start(new ProcessStartInfo(url) { UseShellExecute = true }); }
        catch (System.ComponentModel.Win32Exception)
        {
            MessageBox.Show(T("storeError"), "Steam Pulse", MessageBoxButton.OK, MessageBoxImage.Information);
        }
    }
}