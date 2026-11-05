# Hyperlight Tray

An unofficial, open-source alternative to the Hitscan Utility, for the original **Hitscan Hyperlight** mouse.

---

<p>
  <img src="docs/tray-menu-dark.png" width="244" alt="The tray menu: battery status with a refresh button, Open Configurator, Start with Windows and Exit">
</p>

- **Tray app.** The battery % sits in the taskbar, coloured green, orange or red as it runs down. It also gives
  low-battery and fully-charged alerts, and estimates the time remaining. One ~320 KB exe using 2–5 MB of
  memory, where the official utility takes about 30 MB.

<img src="docs/configurator-settings.png" width="720" alt="The configurator's Settings page in demo mode">

- **Web Configurator.** A browser page for DPI, polling rate, lift-off distance, sensor options, sleep, debounce and
  buttons. Open it from the tray.

## Install

1. Download `hyperlight-tray.exe` from [Releases](https://github.com/fayewave/hyperlight-tray/releases/latest), or
   [build it](#build).
2. Put it somewhere permanent, e.g. `%LOCALAPPDATA%\Programs\HyperlightTray\`.
3. Run it, click the battery number in the tray, and tick **Start with Windows**.

To uninstall: untick **Start with Windows**, exit, then delete the exe and `%APPDATA%\HyperlightTray`.

## Web Configurator

Choose **Open Configurator...** in the tray menu. It needs Chrome, Edge or Opera, because it uses WebHID. Changes
are saved to the mouse straight away, and the mouse has to be awake. Add `?demo` to the address to try it without a
mouse.

Not supported yet: keyboard keys, media keys or macros on buttons, onboard profiles, DPI above 12,800, and lighting
effects. Firmware updates and receiver pairing are left out on purpose.

## Privacy and safety

No driver, no admin rights and no hooks. It uses the same user-mode HID calls as the official utility. The
configurator is served on `localhost` only.

It makes two internet requests. The configurator loads its fonts from Google Fonts. Once a day, the tray asks
GitHub's public API for the latest release. When there's a newer version, a blue dot appears on the tray number,
and the tray menu and configurator link to the release page. Nothing is downloaded or installed automatically.
Turn **Check for updates** off in the configurator to stop this request.

## Build

Needs Rust (MSVC toolchain) and the Windows SDK.

```
cargo build --release
```

Tests: `cargo test` and `node --test web/test`. `hyperlight-probe --settings` prints the mouse's settings
(read-only), which is handy for bug reports. The reverse-engineered protocol is documented in
[PROTOCOL.md](PROTOCOL.md).

## License

MIT. This is an unofficial community project, not affiliated with or endorsed by Hitscan.
