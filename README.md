# Hyperlight Tray

A tiny, open-source tray app for **Hitscan wireless mice** (built and tested on the
Hitscan Hyperlight). It shows the battery percentage as a number in the Windows
notification area and pops up a Windows notification when the battery runs low.

It replaces the battery part of the official Hitscan Utility, which has no low-battery
alert and uses far more resources:

|                      | Hyperlight Tray | Hitscan Utility |
|----------------------|-------------:|----------------:|
| Download size        | ~190 KB, one exe | ~86 MB install |
| Private memory       | ~2 MB        | ~30 MB          |
| CPU when idle        | wakes every 30 s for a few ms | polls continuously |
| Low-battery alert    | yes          | no              |

It doesn't replace the official utility's DPI, button, or lighting settings. Those are
saved on the mouse itself, so set them once in the official utility and then close it.

## Features

- Battery % in the tray. White or black to match your taskbar, red when low, green
  while charging, grey while the mouse is asleep (showing the last reading).
- A notification at your chosen level (off, 5, 10, 15, 20, 25 or 30%; default 10%),
  plus a second "critical" alert at 5%. Each alert fires once and re-arms after
  the battery climbs back up or you charge it.
- An optional "fully charged" notification.
- Checks every 30 seconds, and again when the receiver is plugged in or the PC
  wakes from sleep. Each check is two 17-byte messages, so it costs next to nothing.
- Start with Windows, set per user with no admin rights.
- No installer, no background service, no driver, no network access.

## Install

1. Download `hyperlight-tray.exe` from Releases, or build it (see below).
2. Put it somewhere permanent, e.g. `%LOCALAPPDATA%\Programs\HyperlightTray\`.
3. Run it. Click the battery number in the tray and tick **Start with Windows**.

If the number is hidden in the tray overflow (^), drag it onto the taskbar.

Settings live in `%APPDATA%\HyperlightTray\settings.ini`, and the last battery value in
`battery.ini` next to it. To uninstall, untick Start with Windows, exit, and delete the
exe and that folder.

## Web configurator (Hyperlight Configurator)

`web/` is the **Hyperlight Configurator**, a browser-based replacement for the rest of the official utility.
Its look follows Hitscan's own web app (Inter and IBM Plex Mono, Carbon dark theme, with a light theme too). It covers DPI stages and colours,
polling rate, lift-off distance, Motion Sync, Ripple Control, Angle Snapping, High Performance, Long Distance
Mode, Auto Sleep, debounce, button remapping and settings backups. It talks to the receiver directly with WebHID,
so it works in Chrome, Edge or Opera on desktop, with nothing to install. It's a static site with no build step
(plain ES modules), so it can be hosted on GitHub Pages.

To run it locally:

```
py -m http.server 8765 --directory web
```

Then open <http://localhost:8765> in Chrome or Edge. WebHID needs `localhost` or HTTPS, so opening `index.html`
directly won't work. **Try the demo** works without a mouse.

Every change is written straight to the mouse and then read back to confirm it. The mouse has to be awake to
accept changes, and the page says so when it isn't. Options that are decoded but not yet confirmed on hardware are
marked *untested*.

Tests for the protocol code: `node --test web/test`.

## How it works

The official utility talks to the receiver through its `HIDUsb.dll`. Disassembling that
DLL shows a simple request/reply protocol over a vendor-defined HID collection on the
receiver (usage page `0xFF02`, 17-byte input and output reports):

```
byte  0      report ID, always 0x08
byte  1      command
bytes 2..15  arguments / reply data
byte  16     checksum = 0x55 - (sum of bytes 0..15), mod 256
```

| Command | Purpose        | Reply                                                                 |
|---------|----------------|-----------------------------------------------------------------------|
| `0x03`  | mouse online?  | byte 6 > 0 when the receiver has a link to the mouse                 |
| `0x04`  | battery        | byte 6 = percent, byte 7 = charging flag, bytes 8–9 = millivolts (big-endian) |

Example battery reply at 25%, 3729 mV:

```
08 04 00 00 00 02 19 00 0e 91 00 00 00 00 00 00 8f
                  ^^ ^^ ^^^^^
                  %  chg  mV
```

### The percentage you see

The firmware's own percent (byte 6) jumps around, especially while charging, when the
voltage rises. The official utility doesn't show it. Its `HIDUsb.dll` ("BatteryOptimize")
computes its own figure, and Hyperlight Tray does the same thing so both apps agree:

1. Voltage is converted to a percent with a fixed curve: 3050 mV = 0%, then
   3420, 3480, 3540, 3600, 3660, 3720, 3760, 3800, 3840, 3880, 3920, 3940, 3960,
   3980, 4000, 4020, 4040, 4060, 4080, up to 4110 mV = 100%, with linear steps within each 5% band.
2. The displayed value moves toward that by at most 1% every 10 seconds. While
   discharging it only goes down. While charging it only goes up, stays about 10
   points behind the voltage estimate, slows to one step per 5 minutes near full,
   and reaches 100% once the firmware reports full. If the firmware reports 15% or
   less, the display drops straight to 15%.
3. On startup, the last displayed value is restored from `battery.ini` or from the
   official utility's `HKCU\Software\DeviceBatteryOptimize\BatteryValue_<mouse id>`,
   whichever is newer. It is moved toward the new reading by at most 0.028%/s up or
   0.014%/s down of the time since it was saved. A value more than 30 minutes old is ignored.

`src/estimate.rs` is a port of that logic. Between checks, the tray replays the
10-second steps it missed.

Known IDs (VID `0x3770`): `0x0200` 1K receiver, `0x0300` 8K receiver, `0x0100`
mouse on a cable. The firmware is a Compx-style design (nRF52840 mouse, CX52650N
receiver), so other mice built on the same platform may work with only a VID/PID change.

The app opens the receiver with ordinary user-mode `CreateFile` / `WriteFile` /
`ReadFile` calls, the same way the official utility does. It installs no driver and
sets no hooks, injects nothing, and never needs admin rights.

## Diagnostics

`hyperlight-probe.exe` is a console tool that lists every Hitscan HID collection and dumps
the raw replies:

```
> hyperlight-probe
Querying pid=0200:
  online  : 08 03 00 00 00 01 01 99 da 13 00 00 00 00 00 00 c2  -> mouse connected
  battery : 08 04 00 00 00 02 19 00 0e 8c 00 00 00 00 00 00 94  -> firmware 25%, not charging, 3724 mV (voltage curve: 31%)
```

`hyperlight-probe --settings` also dumps and decodes the mouse's configuration (DPI stages, polling rate, LOD,
debounce, toggles, sleep, buttons). It only reads and never changes anything. The full byte map is in
[PROTOCOL.md](PROTOCOL.md).

If you have a different Hitscan model, please open an issue with this output.

## Build

Needs Rust (MSVC toolchain) and the Windows SDK (for `rc.exe`, which comes with Visual
Studio Build Tools).

```
cargo build --release
```

The binaries end up in `target\release\`. If the project folder is inside Dropbox or
OneDrive, build somewhere else, since the sync client locks files mid-build:

```
$env:CARGO_TARGET_DIR = "$env:LOCALAPPDATA\cargo-target\hyperlight-tray"; cargo build --release
```

`tools/make_icon.py` regenerates `res/app.ico`. `cargo test --bin hyperlight-tray -- --ignored`
writes sample tray icons to `%TEMP%\hyperlight-tray-icons` for checking how they look.

## Disclaimer

This is an unofficial community project. It isn't affiliated with or endorsed by Hitscan.
It only reads status from the mouse and never writes settings to it.

## License

MIT
