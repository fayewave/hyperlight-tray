# Hitscan Hyperlight USB protocol

These notes come from disassembling the official Hitscan Utility (`HIDUsb.dll`, `Hitscan Utility.exe`) and from
watching a real Hyperlight while its settings were changed. Each item is marked with how sure we are:

- **confirmed**: seen on hardware
- **from DLL**: read from the official code, not yet exercised
- **inferred**: an educated guess that still needs a hardware test

## Transport

| | |
|---|---|
| USB VID | `0x3770` |
| PIDs | `0x0200` 1K receiver, `0x0300` 8K receiver, `0x0100` mouse on cable |
| Interface | `MI_01`, HID collection with usage page `0xFF02`, usage `0x0002` |
| Reports | 17-byte output report (host → device) answered by a 17-byte input report |

Use plain output/input reports (WriteFile/ReadFile, or `sendReport` / `inputreport` in WebHID), not feature reports.
Several programs can have the collection open at once, and every one of them sees every reply. Match replies
on the command byte, and on the address too for flash reads.

```
byte  0      report ID, always 0x08
byte  1      command
bytes 2..15  arguments / reply data
byte  16     checksum = (0x55 - sum(bytes 0..15)) & 0xFF
```

## Commands

| Cmd | Arguments (bytes 2..) | Reply | Status |
|---|---|---|---|
| `0x03` | none | byte 6 > 0: receiver has a live link to the mouse. Bytes 7..9: mouse ID (e.g. `99 da 13`) | confirmed |
| `0x04` | none | byte 6: firmware battery %, byte 7: charging, bytes 8..9: millivolts, big-endian | confirmed |
| `0x07` | `00, addr_hi, addr_lo, len (≤ 10), data…` | echoes the request (address, length and data) | confirmed |
| `0x08` | `00, addr_hi, addr_lo, len (≤ 10)` | echoes the arguments in bytes 2..5; data in bytes 6..(6+len) | confirmed |
| `0x0a` | (pushed by device) | unsolicited status update; the DLL refreshes battery from it | from DLL |
| `0x0e` | none | "ReadConfig" | from DLL |
| `0x0f` | `00, 00, 00, 01, n` | "SetCurrentConfig": switch to onboard config/profile `n` | from DLL |
| `0x16` | `00, 00, 00, 0a, mode` | sets Long Distance Mode (1 = on); byte 6 echoes the mode | confirmed |
| `0x17` | none | byte 6: Long Distance Mode (0 = off) | confirmed |

The DLL also has functions for receiver pairing, the dongle's RGB, and firmware updates. Those command bytes
haven't been decoded yet. Firmware update and pairing are out of scope on purpose.

## Writing settings

Writes use `0x07` with the same address/length layout as reads, at most 10 data bytes per request. The data is
the encoded value **including its check byte**. The official DLL compares its copy of the block with the new
one and sends one write per changed field:

- pairs as 2 bytes: `0x00`, `0x02`, `0x04`, `0x06`, `0x08`, `0x0A`, `0x4C`, `0x4E`, `0x50`, `0x52`, `0xA7`,
  `0xA9`, `0xAB`, `0xAD`, `0xAF`, `0xB1`, `0xB3`, `0xB5`, `0xB7`, `0xB9`
- quads (DPI, colours, buttons) as 4 bytes
- `0xA0`..`0xA6` as one 7-byte write

There's no separate save or apply command: the mouse applies a write straight away and it survives power
cycles. Example, polling rate 1000 → 500 Hz:

```
host  : 08 07 00 00 00 02 02 53 00 00 00 00 00 00 00 00 ef
mouse : 08 07 00 00 00 02 02 53 00 00 00 00 00 00 00 00 ef
```

**Writes need the mouse awake.** While it's asleep (`0x03` byte 6 = 0) the receiver still answers reads, apparently
from a copy it keeps, but writes go unanswered and nothing changes. A configurator should check `0x03` first
and ask the user to move the mouse.

Confirmed on hardware: polling rate 1000 → 500 → 1000 Hz, verified by reading back. Writes from the web
configurator, checked by reading back and in the official app: polling 250 and 125 Hz, Ripple Control, Angle
Snapping, and Long Distance Mode (`0x16`).

## Battery percentage

The official app never shows the firmware's percent (byte 6 of the `0x04` reply), which jumps around, especially
while charging. Its `HIDUsb.dll` ("BatteryOptimize") works out its own figure, and Hyperlight Tray and the
configurator copy it so all three agree. Code: [src/estimate.rs](src/estimate.rs) and `Estimator` in
[web/js/protocol.js](web/js/protocol.js).

1. **Voltage curve.** Millivolts map to a percent through a fixed table with one entry per 5%: 3050 mV = 0%, then
   3420, 3480, 3540, 3600, 3660, 3720, 3760, 3800, 3840, 3880, 3920, 3940, 3960, 3980, 4000, 4020, 4040,
   4060, 4080, up to 4110 mV = 100% (99% while charging). Values between entries are interpolated in five steps.
2. **Smoothing.** The shown value moves toward that estimate by at most 1% per tick (every 10 s, or every 5 min
   once charging past 85%):
   - discharging: it only goes down
   - charging: it only goes up, and only while the estimate is more than 10 points ahead; it jumps to 100% once
     the firmware reports 100%
   - firmware at 15% or less: the shown value drops straight to 15%; at 0%, it shows 0%
3. **Restarts.** The last shown value is saved with a timestamp, in the official app's case as
   `HKCU\Software\DeviceBatteryOptimize\BatteryValue_<mouse id> = "<unix time>-<percent>"`. On the next start
   it's reused if it's under 30 minutes old, allowed to drift toward the new estimate by up to 0.028%/s up or
   0.014%/s down.

## Settings block (192 bytes, read with `0x08`)

Most values are stored as a **pair** `[v, 0x55 - v]`. DPI and colours are stored as **quads**
`[a, b, c, 0x55 - (a + b + c)]`. The official parser falls back to a default when a check byte is wrong, and
any writer must keep the check bytes correct.

| Offset | Format | Setting | Values | Status |
|---|---|---|---|---|
| `0x00` | pair | Polling rate | `1` = 1000 Hz, `2` = 500 Hz, `4` = 250 Hz, `8` = 125 Hz | confirmed |
| `0x02` | pair | Number of DPI stages | 1..8 (default 6) | confirmed |
| `0x04` | pair | Current DPI stage | 0-based, less than the stage count | confirmed |
| `0x06` | pair | ? | 0 | unknown |
| `0x08` | pair | ? | 0 | unknown |
| `0x0A` | pair | LOD | `1` = 1 mm, `2` = 2 mm | confirmed |
| `0x0C` | 8 quads | DPI per stage `[x, y, hi, chk]` | DPI = (step + 1) × 50 (1600 → `0x1f`) | confirmed for ≤ 12,800 |
| | | | `hi` presumably holds the high bits above 12,800 | inferred |
| `0x2C` | 8 quads | Stage colour `[r, g, b, chk]` | RGB | confirmed |
| `0x4C`..`0x5F` | pairs/quads | lighting? (`02`, `80`, `03`, `ff 00 ff`, …) | | unknown |
| `0x60` | 16 quads | Button assignments `[type, a, b, chk]` | see below | confirmed for the defaults |
| `0xA0`..`0xA8` | | ? (`01 ff 00 ff 07 09 46`) | | unknown |
| `0xA9` | pair | Debounce | milliseconds | confirmed (0 and 2 seen) |
| `0xAB` | pair | Motion Sync | 0/1 | confirmed |
| `0xAD` | pair | ? | 6 (DLL default 6) | unknown |
| `0xAF` | pair | Angle Snapping | 0/1 | confirmed |
| `0xB1` | pair | Ripple Control | 0/1 | confirmed |
| `0xB3` | pair | ? (a bool in the DLL parser) | 0 | unknown |
| `0xB5` | pair | Auto Sleep enabled | 0/1 | confirmed |
| `0xB7` | pair | Auto Sleep time | units of 10 s: 12 = 2 min, 30 = 5 min | confirmed |
| `0xB9` | pair | High Performance | 0/1 | confirmed |
| `0xBB` | pair | ? | 10 | unknown |
| `0xBD`..`0xBF` | | unused (`ff`) | | |

### Button assignments (`0x60`, 4 bytes per slot)

| Bytes | Meaning | Status |
|---|---|---|
| `01 01 00` | Left click | confirmed |
| `01 02 00` | Right click | confirmed |
| `01 04 00` | Middle click | confirmed |
| `01 08 00` | HID button 4 (Back) | confirmed in slot 4 |
| `01 10 00` | HID button 5 (Forward) | confirmed in slot 5 |
| `02 01 00` | DPI loop | confirmed in slot 6 |
| `02 02 00` / `02 03 00` | DPI up / down | inferred (stored in slots 7 and 8) |
| `07 ..`, `08 ..`, `04 ..` | other functions (keyboard keys, media, macros?) | unknown (stored in slots 9–11) |

The official app labels the Hyperlight's buttons 1–6 as Left, Right, Middle, **Forward**, **Back**, DPI Loop. But
slot 4 holds the HID "Back" code. Which physical button is which slot still needs a test.

## Long Distance Mode

This is a receiver setting, not part of the settings block. Read it with `0x17`; set it with `0x16`.

## Still to do (for a web configurator)

1. Test writes beyond polling rate, one field at a time.
2. DPI above 12,800: the meaning of the third quad byte.
3. The 8K receiver's 2000/4000/8000 Hz codes.
4. Button types `03`+ (keyboard keys, media keys, macros) and where macros are stored.
5. The unknown offsets above.
