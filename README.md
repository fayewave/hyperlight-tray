# Hyperlight Tray

An unofficial, open-source alternative to the Hitscan Utility, for the original **Hitscan Hyperlight** mouse.

---

<p>
  <img src="docs/tray-menu-dark.png" width="244" alt="The tray menu: battery status with a refresh button, Open Configurator, Start with Windows and Exit">
</p>

- **Tray app.** The battery % sits in the taskbar. It also gives
  low-battery and fully-charged alerts, and estimates the time remaining.

<img src="docs/configurator-settings.png" width="720" alt="The configurator's Settings page in demo mode">

- **Web Configurator.** A browser page for DPI, polling rate, lift-off distance, sensor options, sleep, debounce and
  buttons. Open it from the tray.
  
## Install

1. Download `hyperlight-tray.exe` from [Releases](https://github.com/fayewave/hyperlight-tray/releases/latest).
2. Put it somewhere permanent, e.g. `%LOCALAPPDATA%\Programs\HyperlightTray\`.
3. Run it, click the battery number in the tray, and tick **Start with Windows**.

To uninstall: untick **Start with Windows**, exit, then delete the exe and `%APPDATA%\HyperlightTray`.

## Web Configurator

Choose **Open Configurator...** in the tray menu. Requires Chrome, Edge or Opera.

## Stream Deck

Download `com.fayewave.hyperlight.streamDeckPlugin` from
[Releases](https://github.com/fayewave/hyperlight-tray/releases/latest) and double-click it. Then drag
**Hyperlight Tray > Mouse Battery** onto a key. It needs Stream Deck 6.5 or later, and Hyperlight Tray running.

The key shows the battery percent over a bar, orange or red when it's low and blue while charging, with charging or
the time remaining under the name. In the key's settings you can turn those lines off, or add an asleep line or a
blue dot when there's an update. Press the key to re-read the battery, or hold it to open the configurator. If the tray isn't
running, pressing the key starts it. Its text is drawn in Optician Sans (the number) and Bai Jamjuree Bold (the
rest), both under the SIL Open Font License 1.1 and bundled in the plugin, so there are no fonts to install.

## Privacy and safety

No driver, no admin rights and no hooks. It uses the same user-mode HID calls as the official utility. The tray's
small web server (for the configurator and the Stream Deck plugin) listens on `localhost` only, and only the
configurator page can change settings through it.

It makes two internet requests. The configurator loads its fonts from Google Fonts. Once a day, the tray asks
GitHub's public API for the latest release. When there's a newer version, a blue dot appears on the tray number,
and the tray menu and configurator link to the release page. Nothing is downloaded or installed automatically.
Turn **Check for updates** off in the configurator to stop this request.

## License

MIT. This is an unofficial community project, not affiliated with or endorsed by Hitscan.
