# tun2proxy GUI

**English** | [中文](README.md)

A cross-platform desktop front end for [tun2proxy](https://github.com/tun2proxy/tun2proxy),
written in Rust, with a UI built on [ZSUI](https://github.com/qiu7824/zsui)
(native Rust desktop UI — no WebView).

The layout and interaction model were inspired by
[Paper-Dragon/tun2proxy-gui](https://github.com/Paper-Dragon/tun2proxy-gui), then
rebuilt around "multiple proxy profiles + one-click start/stop".

## Features

- **Multiple proxy profiles**: keep as many as you like (home / office / a
  throwaway node), switch between them from a list, and create, duplicate,
  delete or rename them. The selected profile is what the tunnel starts with.
- **One-click start/stop**: the sidebar always shows the service state plus a
  start/stop button, so you can tell at a glance whether the tunnel is up.
- **Managed tun2proxy**: on first run, hit "Download and install" and the app
  fetches the official binary for your platform from GitHub Releases, verifies
  its SHA-256 and unpacks it into its own data directory. Later you can "Check
  for updates" and swap it in place — no need to ship a new GUI release.
- **Full option coverage**: upstream proxy (`socks5://` / `socks4://` /
  `http://`), DNS strategy (direct / virtual IP / over TCP), DNS servers,
  virtual address pool, bypass ranges, TUN name, MTU, TCP/UDP timeouts, max
  sessions, IPv6, auto route setup, UDPGW and log level.
- **Live log**: the child process's stdout/stderr is streamed straight into the
  window, and can be cleared with one click.
- **Proxy reachability check**: fill in the upstream URL, hit "Test", and the
  app opens a TCP connection and performs one handshake for the given protocol
  (SOCKS5 greeting / HTTP `CONNECT`). "Nothing is listening on that port", "the
  proxy wants authentication" or "connected, but the protocol is wrong" is
  written right on that row — no need to start the tunnel and guess from logs.
- **Pre-flight before start**: pressing "Start" runs the same handshake probe
  first. If the proxy is unreachable the app explains why in a dialog and does
  **not** launch the core, instead of spawning a tun2proxy that is bound to fail.
- **Chinese and English UI**: pick "Follow system / 中文 / English" in Settings,
  applied immediately. Every string lives in code as
  `t("中文", "English")`, so there is no separate translation file to maintain.
- **Live traffic chart**: the dashboard hero card shows downstream/upstream
  throughput for the last minute.
- **System tray**: a tray icon is always present, with a tooltip that follows
  the service state. Its context menu can start, stop, show the window or quit;
  double-clicking the icon brings the window back.
- **Automatic elevation**: on Windows, if the app is not running as
  administrator it asks and then relaunches itself through UAC. On macOS/Linux
  it explains that root is required (on Linux you can enable "no root" mode to
  run inside a user namespace instead).

## Project layout

```
src/
├─ main.rs         Startup: privileges → DPI → config → engine → window
├─ core/           UI-independent core (no zsui dependency, testable on its own)
│  ├─ mod.rs       Module docs and re-exports
│  ├─ paths.rs     Where config / downloads / logs live
│  ├─ config.rs    Read, validate and convert proxy profiles into CLI args
│  ├─ binary.rs    Download / verify / unpack the official tun2proxy binary
│  ├─ engine.rs    Spawn the child process, collect logs, track state and progress
│  ├─ elevate.rs   Administrator / root detection and Windows auto-elevation
│  ├─ probe.rs     Upstream proxy reachability check (TCP + SOCKS5 / HTTP handshake)
│  ├─ traffic.rs   Sample the child process's byte counters (Windows IO counters / /proc)
│  ├─ tray.rs      Tray icon (Windows: a dedicated message window + Shell_NotifyIcon)
│  ├─ window.rs    Find and show/hide the main window (tray restore, minimize to tray)
│  ├─ icon.rs      Multi-size bitmaps of the app icon (one copy for window and tray)
│  ├─ i18n.rs      Chinese/English strings and the language setting
│  └─ platform.rs  Process-level init (Windows DPI awareness)
└─ ui/             Presentation layer (zsui)
   ├─ state.rs     App state, message enum, numeric input drafts
   ├─ view.rs      The whole view tree: sidebar + content area
   ├─ sidebar.rs   Sidebar (title, service state, navigation, start/stop)
   ├─ pages.rs     Content area: thin top bar + the grouped composition of each page
   ├─ cards.rs     Groups and rows for each page
   ├─ widgets.rs   Reusable text styles and widgets (hero card, group, setting row, pill)
   ├─ chart.rs     Live traffic chart (zsui canvas)
   └─ update.rs    Message handling (the only place that mutates state)
assets/
├─ icon.ico       App icon (multi-size, embedded into the exe resource section)
└─ icon.png       Icon source image (512×512, with alpha)
build.rs          Compiles app.rc (icon + version info) into the Windows exe
```

### App icon

Two independent paths, each covering a different surface:

- **Explorer / taskbar**: `app.rc` + `build.rs` (`embed-resource`) compile
  `assets/icon.ico` into the exe's resource section. That is the only source
  Windows reads the icon from.
- **Window title bar**: `core::paths::install_icon()` writes the embedded ico
  into the data directory and hands the absolute path to
  `native_window(..).icon_path(..)`. On non-Windows platforms only this path
  exists.

The icon is a multi-size ico (16/24/32/48/64/128/256); the system picks the
size it needs, so the small taskbar icon never turns to mush. To change the
icon, replace `assets/icon.ico` and run `cargo build` again.

### UI structure

```
┌──────────────┬──────────────────────────────────────┐
│ Tun2Proxy    │ Dashboard          Service state, …  │
│ [v0.0.1]     │──────────────────────────────────────│
│              │  ╭──────────────────────────────────╮│
│ ▸ Dashboard  │  │ [Stopped]            [  Start ]  ││
│ ▸ Profiles   │  │ Tun2Proxy is not running         ││
│ ▸ Network    │  │ One line of explanation…         ││
│ ▸ App & logs │  ╰──────────────────────────────────╯│
│              │  Active profile                      │
│              │  ╭──────────────────────────────────╮│
│              │  │ [▤] Profile         [ Default ⌄] ││
│              │  │     The tunnel starts with…      ││
│              │  ├──────────────────────────────────┤│
│              │  │ [⚙] Upstream  socks5://…   [✎]   ││
│              │  ╰──────────────────────────────────╯│
│ ● Service    │                                      │
│ [  Start  ]  │  Scrollable list of groups           │
└──────────────┴──────────────────────────────────────┘
```

Three visual layers: white sidebar, light-grey content area (`Surface`), and
white cards with a 1dp outline and 12dp radius. White cards float naturally on
the grey background, and neighbouring groups are separated by spacing and
outlines rather than divider lines.

Setting rows follow Windows 11 Task Manager: **the group title sits outside the
card** (small bold text) and the card holds one setting per row — icon on the
left, "title + grey description" in the middle, control or value on the right,
with a hairline inset 14dp on both sides between rows. Grouping related
settings into one card is far denser than one card per setting, and removes the
wall of boxes.

The top bar is a single line (page name, one explanatory sentence on the right,
hairline at the bottom) instead of taking two rows. The dashboard hero card
(state pill + large title + primary action) is the first item in the content
flow. Only the group list scrolls; the top bar stays visible.

On the profile detail page (Profiles → pencil), the right side of the top bar
becomes a primary "Save to disk" button: when a form is long enough to scroll,
Save must not scroll out of view. The back button sits left of the title and
returns to the profile list.

Each of the four pages owns one group of settings, and switching pages never
loses anything you typed (all state lives in `App`). The old "Advanced options"
page is gone: log level moved to "App & logs", UDP forwarding (UDPGW) to
"Network" — a page with three switches only adds a click.

The core notifies the UI exclusively through a repaint callback
(`Engine::set_rebuild_hook`), so `src/core` can be reused as-is if the UI is
ever ported to another framework.

## Building

```bash
cargo build --release
```

The binary lands in `target/release/tun2proxy-gui` (`tun2proxy-gui.exe` on
Windows).

Dependencies:

- Windows: nothing extra.
- Linux: ZSUI uses the `linux-direct` backend (Cairo/Pango drawing, not GTK
  widgets), so you need `libcairo2-dev libpango1.0-dev libglib2.0-dev
  pkg-config`.
- macOS: the Xcode command line tools.

### Releasing

Pushing a tag builds every supported target and publishes them to GitHub
Releases (see `.github/workflows/release.yml`):

```bash
# Bump `version` in Cargo.toml, commit it, then tag — CI enforces that they match
git tag v0.1.0 && git push origin v0.1.0
```

Artifacts: `tun2proxy-gui-<tag>-<target>.zip` (Windows) and `.tar.gz` (Linux /
macOS, containing `Tun2Proxy GUI.app` on macOS), plus a `SHA256SUMS.txt`.
You can also run the workflow manually from the Actions page with the name of
an existing tag to republish.

## Network and proxies

Downloading/updating tun2proxy goes through the system `curl` (falling back to
`wget`, then PowerShell), so it **honours the standard `http_proxy` /
`https_proxy` environment variables**:

```bash
# Windows (cmd)
set http_proxy=http://127.0.0.1:7890
set https_proxy=http://127.0.0.1:7890
# Windows (PowerShell)
$env:http_proxy="http://127.0.0.1:7890"; $env:https_proxy="http://127.0.0.1:7890"
```

Then start the app from that same terminal. On failure the UI shows the actual
`curl` reason (`28` means timeout, `403` usually means a proxy blocked GitHub).
You can also download the release archive yourself and unpack its contents into
`bin/` inside the data directory.

## Data directory

| Platform | Path |
|----------|------|
| Windows | `%APPDATA%\tun2proxy-gui\` |
| macOS | `~/Library/Application Support/tun2proxy-gui/` |
| Linux | `~/.config/tun2proxy-gui/` (follows `$XDG_CONFIG_HOME`) |

Contents: `config.json` (all settings) and `bin/` (the downloaded tun2proxy,
`wintun.dll` and friends). Setting the `TUN2PROXY_GUI_HOME` environment
variable moves the whole directory elsewhere (portable mode).

`config.json` looks like this:

```json
{
  "profiles": [
    { "name": "Home SOCKS5", "proxy_url": "socks5://127.0.0.1:1080" },
    { "name": "Office HTTP", "proxy_url": "http://10.0.0.5:8080", "dns": "virtual" }
  ],
  "active": 0,
  "auto_check_update": true,
  "release_repo": "tun2proxy/tun2proxy"
}
```

Missing fields fall back to defaults, so a hand-written config only needs the
keys you care about.

## Things to know before running

- tun2proxy creates a virtual network adapter and rewrites system routes, so it
  **requires administrator / root privileges**.
- The GUI neither bundles tun2proxy nor links against its libraries: the tunnel
  is entirely driven by the official binary, so GUI releases don't have to
  follow tun2proxy's release cadence.
- Stop the service before updating (on Windows a running exe cannot be replaced;
  the app blocks the attempt for you).
- "Start" probes the upstream proxy first (the same handshake as the "Test"
  button). If it can't connect, the core is never started and the dialog states
  the reason.

## Tray and closing

- Closing the window opens a confirmation dialog whose **default button is
  "Minimize to tray"** (Enter accepts it): the tunnel keeps running and
  double-clicking the tray icon restores the window. To really quit, use the
  close button or press `Esc`.
- With "Ask on close" disabled in Settings, closing the window hides it into the
  tray without asking.
- Tray context menu: show window / start / stop / quit; the tooltip follows the
  service state.
- Windows only: tray support on other platforms needs their own system APIs and
  is not implemented yet. There, "minimize to tray" degrades to a plain exit, so
  the window can never be hidden where you can't get it back.

## Language

Settings offers "Follow system / 中文 / English"; the choice applies immediately
and is persisted (the `language` field). Strings live in code
(`t("中文", "English")`) — there is no separate translation resource file.

## Development

```bash
cargo check        # type check
cargo test         # unit tests for config management
cargo build        # debug build
```

A screenshot helper for debugging lives at `.probe/shot.ps1` (Windows only):
`-Resize` changes the window size, `-Click` clicks once, `-Wheel` scrolls, and
`PrintWindow` captures the window to a PNG so you can inspect the layout. Set
`TUN2PROXY_GUI_NO_ELEVATE=1` to skip elevation.

> Don't use screen capture (`CopyFromScreen`): while debugging the window is not
> necessarily composited onto a visible desktop, so you end up capturing another
> app. `PrintWindow(hwnd, hdc, 2)` asks the window for its pixels directly and is
> reliable.

### Three layout traps (zsui 0.3)

1. **Wrapping is a style, not a method call**: `ViewNode::text_wrap()` only
   affects multi-line text inputs and is a no-op on ordinary text. To wrap, set
   `SemanticTextStyle { wrap: TextWrap::Word, .. }` — see `wrapped()` in
   `src/ui/widgets.rs`. Otherwise long descriptions stay on one line and get
   clipped by the window edge once they leave the card.
2. **A container's width is the natural width of its widest child**: there is no
   `width: 100%`. To make a card follow the window, give the parent
   `align(ViewAlign::Stretch)` and the child `flex(1.0)` (see `card()` in
   `src/ui/widgets.rs`). Conversely, wrapping text inside a `row` gets squeezed
   into a vertical strip, so inline labels use `caption_nowrap()`.
3. **When it doesn't fit, it clips — it does not wrap**: widgets and text are
   never squeezed below platform metrics, and a container narrower than its
   intrinsic minimum width simply gets cut off on the right (the hard-size
   contract in `src/view/layout.rs`). The minimum window width
   `ui::MIN_WINDOW_WIDTH` must therefore exceed the intrinsic minimum width of
   the widest page — `ui::layout_probe::no_page_overflows_the_minimum_window`
   watches this, so dropping a wider widget into a card fails the test instead of
   showing the user a cut-off edge.

## License

MIT — see [LICENSE](LICENSE) for the full text.
