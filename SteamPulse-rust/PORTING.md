# Rust port

The Windows shell, Steam integration, settings, and rendering are written in Rust. The taskbar widget uses Win32, Direct2D, and DirectWrite. The settings and widget menu use WebView2.

The port retains the shared settings format, executable launch history, Steam download estimates, public friends activity, sale alerts, the watchdog, and the existing GitHub update channel.

In 4.8.4, the settings theme uses a short color transition that respects Windows reduced-motion settings. The CPU/RAM line uses both accent colors. The widget menu no longer highlights Settings when opened with the mouse, and keyboard focus remains visible.
