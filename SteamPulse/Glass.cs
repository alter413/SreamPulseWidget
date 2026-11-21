using System;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Drawing = System.Drawing;

namespace SteamPulse;

internal readonly record struct GlassOptions(double Depth, double Blur, double Dispersion, double Shine,
    double Light, double Radius);

internal sealed class GlassBackdrop : IDisposable
{
    private readonly Window window;
    private readonly Action<ImageSource?>? present;
    private readonly Action<byte[]?>? presentEncoded;
    private readonly Func<Native.Rect?>? contentBounds;
    private long lastSample;
    private long lastMovement;
    private ImageSource? encodedImage;
    private byte[]? encodedFrame;
    private readonly DispatcherTimer timer = new(DispatcherPriority.Background) { Interval = TimeSpan.FromMilliseconds(140) };
    private readonly VasoLens lens = new();
    private GlassOptions options;
    private bool enabled;
    private bool excluded;
    private bool busy;
    private ImageSource? presented;
    private bool closed;
    private IntPtr hwnd;
    private int revision;

    public GlassBackdrop(Window window, Action<ImageSource?>? present = null,
        Func<Native.Rect?>? contentBounds = null, Action<byte[]?>? presentEncoded = null)
    {
        this.window = window;
        this.present = present;
        this.contentBounds = contentBounds;
        this.presentEncoded = presentEncoded;
        if (presentEncoded != null)
        {
            timer.Interval = TimeSpan.FromMilliseconds(95);
            window.LocationChanged += Moved;
            window.SizeChanged += Resized;
        }
        window.IsVisibleChanged += Visibility;
        window.StateChanged += StateChange;
        timer.Tick += (_, _) => _ = Paint();
    }

    public void Configure(bool enabled, GlassOptions options)
    {
        if (this.enabled == enabled && this.options == options) return;
        this.enabled = enabled;
        this.options = options;
        revision++;
        Update();
    }

    private void Visibility(object sender, DependencyPropertyChangedEventArgs e) => Update();
    private void StateChange(object? sender, EventArgs e) => Update();

    private void Moved(object? sender, EventArgs e) => PositionChanged();
    private void Resized(object sender, SizeChangedEventArgs e) => PositionChanged();

    private void PositionChanged()
    {
        if (closed || !enabled || !excluded || !window.IsVisible) return;
        lastMovement = Stopwatch.GetTimestamp();
        _ = Paint();
    }

    private void Clear()
    {
        present?.Invoke(null);
        presentEncoded?.Invoke(null);
    }

    private void Update()
    {
        if (closed) return;
        var active = enabled && window.IsVisible && window.WindowState != WindowState.Minimized;
        hwnd = new WindowInteropHelper(window).Handle;
        if (active && !excluded && hwnd != IntPtr.Zero) excluded = SetWindowDisplayAffinity(hwnd, 0x11);
        if (!active || !excluded)
        {
            timer.Stop();
            revision++;
            lastSample = 0;
            if (excluded && hwnd != IntPtr.Zero && SetWindowDisplayAffinity(hwnd, 0)) excluded = false;
            presented = null;
            Clear();
            return;
        }
        timer.Start();
        _ = Paint();
    }

    private async Task Paint()
    {
        // Sample only our window's rectangle and only when visible. Windows excludes our own
        // window from that sample. The pixels stay in memory for local rendering.
        if (busy || !enabled || !excluded || closed || !window.IsVisible || window.WindowState == WindowState.Minimized) return;
        if (presentEncoded != null)
        {
            var moving = lastMovement != 0 && Stopwatch.GetElapsedTime(lastMovement).TotalMilliseconds < 170;
            var delay = moving ? 42 : 95;
            if (timer.Interval.TotalMilliseconds != delay) timer.Interval = TimeSpan.FromMilliseconds(delay);
            if (lastSample != 0 && Stopwatch.GetElapsedTime(lastSample).TotalMilliseconds < delay - 4) return;
        }
        Native.Rect bounds;
        if (contentBounds != null)
        {
            if (contentBounds() is not { } content) return;
            bounds = content;
        }
        else if (!Native.GetWindowRect(hwnd, out bounds)) return;
        if (bounds.Right - bounds.Left < 3 || bounds.Bottom - bounds.Top < 3) return;
        lastSample = Stopwatch.GetTimestamp();
        var request = options;
        var generation = revision;
        busy = true;
        try
        {
            var frame = await Task.Run(() =>
            {
                var bitmap = lens.Capture(bounds, request);
                return (Bitmap: bitmap, Jpeg: presentEncoded == null ? null : EncodeFrame(bitmap));
            });
            if (!closed && enabled && window.IsVisible && window.WindowState != WindowState.Minimized &&
                generation == revision && !ReferenceEquals(presented, frame.Bitmap))
            {
                presented = frame.Bitmap;
                present?.Invoke(frame.Bitmap);
                presentEncoded?.Invoke(frame.Jpeg);
            }
        }
        catch (Exception ex) when (ex is Win32Exception or ExternalException or ArgumentException)
        {
            if (!closed && generation == revision) Clear();
        }
        finally { busy = false; }
    }

    private byte[]? EncodeFrame(ImageSource? image)
    {
        if (image is not BitmapSource bitmap) return null;
        if (ReferenceEquals(encodedImage, bitmap)) return encodedFrame;
        using var bytes = new MemoryStream();
        var encoder = new JpegBitmapEncoder { QualityLevel = 92 };
        encoder.Frames.Add(BitmapFrame.Create(bitmap));
        encoder.Save(bytes);
        encodedFrame = bytes.ToArray();
        encodedImage = bitmap;
        return encodedFrame;
    }

    public void Dispose()
    {
        closed = true;
        timer.Stop();
        if (excluded && hwnd != IntPtr.Zero) SetWindowDisplayAffinity(hwnd, 0);
        window.IsVisibleChanged -= Visibility;
        window.StateChanged -= StateChange;
        window.LocationChanged -= Moved;
        window.SizeChanged -= Resized;
    }

    [DllImport("user32.dll")] private static extern bool SetWindowDisplayAffinity(IntPtr hwnd, uint affinity);
}

// Native adaptation of vaso's src/core/optics.ts, filter.ts and light.ts.
// Upstream huozhi/vaso, MIT, revision af5ba08. See the embedded Vaso-LICENSE.txt.
internal sealed class VasoLens
{
    private const int ProfileSamples = 256;
    private static readonly double[] Profile = BuildProfile();
    private byte[] backdrop = [];
    private byte[] previous = [];
    private (int W, int H, GlassOptions Options) lastCapture;
    private ImageSource? cached;
    private byte[] horizontal = [];
    private byte[] softened = [];
    private byte[] result = [];
    private Ray[] rays = [];
    private (int Width, int Height, double Radius, double Light) lensKey;
    private readonly record struct Ray(float X, float Y, float Specular);

    public ImageSource? Capture(Native.Rect bounds, GlassOptions p)
    {
        var nativeWidth = bounds.Right - bounds.Left;
        var nativeHeight = bounds.Bottom - bounds.Top;
        var scale = Math.Min(1, Math.Sqrt(160_000d / ((double)nativeWidth * nativeHeight)));
        var width = Math.Max(3, (int)Math.Round(nativeWidth * scale));
        var height = Math.Max(3, (int)Math.Round(nativeHeight * scale));
        var bytes = width * height * 4;
        if (backdrop.Length != bytes)
        {
            backdrop = new byte[bytes];
            previous = new byte[bytes];
            cached = null;
            horizontal = new byte[bytes];
            softened = new byte[bytes];
            result = new byte[bytes];
        }
        if (!DesktopPixels(bounds, width, height, backdrop)) return null;
        var requested = (width, height, p);
        if (cached != null && lastCapture == requested && backdrop.AsSpan().SequenceEqual(previous)) return cached;
        lastCapture = requested;
        backdrop.CopyTo(previous, 0);
        Filter(width, height, p with { Radius = p.Radius * scale, Blur = p.Blur * scale }, backdrop, result);
        var image = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgr32, null, result, width * 4);
        image.Freeze();
        return cached = image;
    }

    public static ImageSource Preview(int width, int height, GlassOptions options)
    {
        var pixels = new byte[width * height * 4];
        var filtered = new byte[pixels.Length];
        for (var y = 0; y < height; y++)
        for (var x = 0; x < width; x++)
        {
            var i = (y * width + x) * 4;
            var band = .5 + .5 * Math.Sin((x * .045 + y * .07) + 1.2 * Math.Cos(x * .022));
            var line = (x + 2 * y) % 64 < 2 ? 28d : 0;
            pixels[i] = (byte)(51 + 59 * band + line);
            pixels[i + 1] = (byte)(31 + 53 * band + line);
            pixels[i + 2] = (byte)(33 + 22 * (1 - band) + line);
        }
        new VasoLens().Filter(width, height, options, pixels, filtered);
        var result = BitmapSource.Create(width, height, 96, 96, PixelFormats.Bgr32, null, filtered, width * 4);
        result.Freeze();
        return result;
    }

    internal void Filter(int width, int height, GlassOptions p, byte[] input, byte[] output)
    {
        var bytes = width * height * 4;
        if (horizontal.Length != bytes)
        {
            horizontal = new byte[bytes];
            softened = new byte[bytes];
        }
        var source = input;
        var radius = (int)Math.Round(p.Blur);
        if (radius > 0)
        {
            // Two separable box passes approximate a Gaussian without sampling an entire kernel per pixel.
            BlurLine(input, horizontal, width, height, radius, false);
            BlurLine(horizontal, softened, width, height, radius, true);
            BlurLine(softened, horizontal, width, height, radius, false);
            BlurLine(horizontal, softened, width, height, radius, true);
            source = softened;
        }

        var bevel = Math.Min(width, height) * .5 * .6;
        var baseOffset = .35 * bevel * p.Depth;
        var spread = Math.Min(Math.Min(width, height) < 80 ? 5 : 10,
            Math.Max(0, p.Dispersion) * bevel * .5 * Math.Min(1, Math.Abs(p.Depth) * 4));
        var spectral = spread <= .5 ? 0 : spread * .27;
        if (lensKey != (width, height, p.Radius, p.Light))
        {
            lensKey = (width, height, p.Radius, p.Light);
            rays = BuildRays(width, height, bevel, p.Radius, p.Light);
        }

        for (var y = 0; y < height; y++)
        for (var x = 0; x < width; x++)
        {
            var pixel = y * width + x;
            var ray = rays[pixel];
            var o = pixel * 4;
            var glare = p.Shine * ray.Specular;
            if (ray.X == 0 && ray.Y == 0)
            {
                for (var channel = 0; channel < 3; channel++)
                    output[o + channel] = Lit(source[o + channel], glare);
                continue;
            }
            // The red, green and blue samples follow Vaso's least-to-most refracted wavelength order.
            for (var channel = 0; channel < 3; channel++)
            {
                var offset = baseOffset + spectral * (1 - channel);
                var value = Sample(source, width, height, x + ray.X * offset, y + ray.Y * offset, channel);
                output[o + channel] = Lit(value, glare);
            }
        }
    }

    private static byte Lit(double channel, double glare) => (byte)Math.Clamp(channel + (255 - channel) * glare, 0, 255);

    private static double Sample(byte[] src, int width, int height, double x, double y, int channel)
    {
        x = Math.Clamp(x, 0, width - 1);
        y = Math.Clamp(y, 0, height - 1);
        var px = (int)x;
        var py = (int)y;
        var sx = Math.Min(px + 1, width - 1);
        var sy = Math.Min(py + 1, height - 1);
        var upper = Blend(src[(py * width + px) * 4 + channel], src[(py * width + sx) * 4 + channel], x - px);
        var lower = Blend(src[(sy * width + px) * 4 + channel], src[(sy * width + sx) * 4 + channel], x - px);
        return Blend(upper, lower, y - py);
    }

    private static double Blend(double left, double right, double t) => left + (right - left) * t;

    private static Ray[] BuildRays(int w, int h, double bevel, double radius, double angle)
    {
        var map = new Ray[w * h];
        var halfW = w / 2d;
        var halfH = h / 2d;
        var r = Math.Min(Math.Max(radius, bevel), Math.Min(halfW, halfH));
        var lightX = Math.Sin(angle * Math.PI / 180);
        var lightY = -Math.Cos(angle * Math.PI / 180);
        for (var y = 0; y < h; y++)
        for (var x = 0; x < w; x++)
        {
            var cx = x + .5 - halfW;
            var cy = y + .5 - halfH;
            var qx = Math.Abs(cx) - (halfW - r);
            var qy = Math.Abs(cy) - (halfH - r);
            double distance, nx, ny;
            if (qx > 0 && qy > 0)
            {
                var length = Math.Sqrt(qx * qx + qy * qy);
                distance = r - length;
                nx = qx / length;
                ny = qy / length;
            }
            else
            {
                distance = r - Math.Max(qx, qy);
                nx = qx > qy ? 1 : 0;
                ny = qx > qy ? 0 : 1;
            }
            if (distance >= bevel) continue;
            nx *= Math.Sign(cx);
            ny *= Math.Sign(cy);
            var cell = Math.Clamp(distance / bevel, 0, 1) * (ProfileSamples - 1);
            var i = (int)cell;
            var power = Blend(Profile[i], Profile[Math.Min(i + 1, ProfileSamples - 1)], cell - i);
            var facing = nx * lightX + ny * lightY;
            var edgeFade = Math.Max(0, 1 - distance / Math.Max(2, bevel * .26));
            var specular = Math.Clamp(edgeFade, 0, 1) * (.11 + .41 * Math.Max(0, facing) + .23 * Math.Max(0, -facing));
            map[y * w + x] = new((float)(-nx * power), (float)(-ny * power), (float)specular);
        }
        return map;
    }

    private static double[] BuildProfile()
    {
        var profile = new double[ProfileSamples];
        var max = 0d;
        for (var i = 0; i < ProfileSamples; i++)
        {
            var u = 1 - i / (ProfileSamples - 1d);
            var slope = u * u * u / Math.Pow(Math.Max(1 - u * u * u * u, 1e-9), .75);
            var incident = Math.Atan(slope);
            var refracted = Math.Asin(Math.Sin(incident) / 1.5);
            profile[i] = Math.Tan(incident - refracted);
            max = Math.Max(max, profile[i]);
        }
        for (var i = 0; i < ProfileSamples; i++) profile[i] /= max;
        return profile;
    }

    private static void BlurLine(byte[] src, byte[] dest, int width, int height, int radius, bool vertical)
    {
        var lines = vertical ? width : height;
        var length = vertical ? height : width;
        var step = (vertical ? width : 1) * 4;
        var normalizer = 2 * radius + 1;
        for (var line = 0; line < lines; line++)
        {
            var start = line * (vertical ? 1 : width) * 4;
            var b = 0;
            var g = 0;
            var r = 0;
            for (var i = -radius; i <= radius; i++)
            {
                var k = start + Math.Clamp(i, 0, length - 1) * step;
                b += src[k]; g += src[k + 1]; r += src[k + 2];
            }
            for (var i = 0; i < length; i++)
            {
                var o = start + i * step;
                dest[o] = (byte)(b / normalizer);
                dest[o + 1] = (byte)(g / normalizer);
                dest[o + 2] = (byte)(r / normalizer);
                var add = start + Math.Min(i + radius + 1, length - 1) * step;
                var remove = start + Math.Max(0, i - radius) * step;
                b += src[add] - src[remove]; g += src[add + 1] - src[remove + 1]; r += src[add + 2] - src[remove + 2];
            }
        }
    }

    private static bool DesktopPixels(Native.Rect rect, int width, int height, byte[] destination)
    {
        using var bitmap = new Drawing.Bitmap(width, height, System.Drawing.Imaging.PixelFormat.Format32bppRgb);
        using (var g = Drawing.Graphics.FromImage(bitmap))
        {
            var screen = GetDC(IntPtr.Zero);
            if (screen == IntPtr.Zero) return false;
            try
            {
                var result = g.GetHdc();
                try
                {
                    SetStretchBltMode(result, 4);
                    SetBrushOrgEx(result, 0, 0, IntPtr.Zero);
                    if (!StretchBlt(result, 0, 0, width, height, screen, rect.Left, rect.Top,
                        rect.Right - rect.Left, rect.Bottom - rect.Top, 0x00CC0020)) return false;
                }
                finally { g.ReleaseHdc(result); }
            }
            finally { ReleaseDC(IntPtr.Zero, screen); }
        }
        var data = bitmap.LockBits(new Drawing.Rectangle(0, 0, width, height), ImageLockMode.ReadOnly, System.Drawing.Imaging.PixelFormat.Format32bppRgb);
        try { Marshal.Copy(data.Scan0, destination, 0, destination.Length); }
        finally { bitmap.UnlockBits(data); }
        return true;
    }

    [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
    [DllImport("gdi32.dll")] private static extern int SetStretchBltMode(IntPtr dc, int mode);
    [DllImport("gdi32.dll")] private static extern bool SetBrushOrgEx(IntPtr dc, int x, int y, IntPtr previous);
    [DllImport("gdi32.dll")] private static extern bool StretchBlt(IntPtr target, int x, int y, int w, int h,
        IntPtr source, int sx, int sy, int sw, int sh, uint operation);
}
