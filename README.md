# Hyperlight Tray

An unofficial, open-source replacement for the Hitscan Utility, for the **Hitscan Hyperlight** mouse.

- **Hyperlight Tray** shows the battery level in the system tray and warns you when it's low.
- **Hyperlight Configurator** is a browser page for the mouse's settings, opened from the tray.

|                   | Hyperlight Tray    | Hitscan Utility    |
|-------------------|--------------------|--------------------|
| Size              | ~280 KB, one exe   | ~86 MB install     |
| Memory            | ~2 MB              | ~30 MB             |
| CPU when idle     | a few ms every 30 s | polls constantly  |
| Low-battery alert | yes                | no                 |

## Install

1. Download `hyperlight-tray.exe` from Releases, or [build it](#development).
2. Put it somewhere permanent, e.g. `%LOCALAPPDATA%\Programs\HyperlightTray\`.
3. Run it, click the battery number in the tray, and tick **Start with Windows**.

If the number is hidden under the tray's ^ arrow, drag it onto the taskbar.

To uninstall: untick **Start with Windows**, exit, then delete the exe and `%APPDATA%\HyperlightTray`.

## Tray app

- Battery % as a number in the tray: white or black to match the taskbar, red when low, green while charging,
  grey while the mouse sleeps.
- Shows the same percentage as the official utility, using the same voltage-based smoothing.
- Alerts when the battery hits your chosen level (default 10%), again at 5%, and optionally when fully charged.
  Set these at the top of the configurator's **Performance** page.
- Checks every 30 seconds, and straight away when the receiver is plugged in or the PC wakes.
- The tray menu has **Open Configurator…**, **Start with Windows** and **Exit**.
  `hyperlight-tray.exe --configurator` opens the configurator too, for shortcuts.

## Configurator

Change DPI stages and colours, polling rate, lift-off distance, motion sync, ripple control, angle snapping,
performance mode, long distance mode, auto sleep, debounce and button assignments, and back up or restore your
settings. When opened from the tray, a **Hyperlight Tray** card at the top of the Performance page holds the tray
app's own settings: battery alerts, Start with Windows and a test notification.

- Needs Chrome, Edge or Opera on a computer (it uses WebHID).
- Each change is saved to the mouse straight away and checked by reading it back. The mouse has to be awake.
- The tray serves the page at `http://localhost:47820`, which only your PC can reach.

Still needs the official utility: keyboard keys, media keys or macros on buttons; onboard profiles; DPI above
12,800 or separate X/Y DPI; lighting effects; receiver pairing and firmware updates.

## Privacy and safety

- No driver, no admin rights, no hooks. It uses the same user-mode HID calls as the official utility.
- Nothing is sent over the internet, apart from the configurator page loading its fonts from Google Fonts.
- Firmware updates and pairing are left out on purpose.

## Diagnostics

`hyperlight-probe` lists the Hitscan devices it finds and shows the raw battery reply. `hyperlight-probe
--settings` also decodes the mouse's settings, read-only. Got a different Hitscan model? Open an issue with its
output.

## How it works

The protocol was recovered by disassembling the official utility's `HIDUsb.dll` and checking each setting on a
real mouse. Everything goes through 17-byte HID reports on the receiver's vendor collection (usage page
`0xFF02`). See [PROTOCOL.md](PROTOCOL.md) for the commands, the settings map and the battery calculation.

## Development

Needs Rust (MSVC toolchain) and the Windows SDK, for `rc.exe`.

```
cargo build --release
```

Binaries go to `target\release\`. If the folder is synced by Dropbox or OneDrive, set `CARGO_TARGET_DIR` to a
folder outside it, because sync clients lock files mid-build.

- Tests: `cargo test` and `node --test web/test`
- Configurator on its own: `py -m http.server 8765 --directory web`, then open <http://localhost:8765>.
  **Start demo** works without a mouse.
- `tools/make_icon.py` regenerates `res/app.ico`.

## Disclaimer

Unofficial community project, not affiliated with or endorsed by Hitscan. The configurator writes settings to
your mouse, so export a backup first if you want an easy way back.

## License

MIT
