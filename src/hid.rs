//! Talks to a Hitscan receiver (or a mouse plugged in by cable) over its vendor HID
//! collection, using the same plain user-mode Win32 calls the official utility makes
//! (CreateFile / WriteFile / ReadFile on the HID device). No drivers, no hooks.
//!
//! Protocol, recovered from the official utility's `HIDUsb.dll`:
//!
//! * Every message is a 17-byte HID report, sent as an output report and answered
//!   with an input report on the same collection.
//! * Byte 0 is the report ID (`0x08`), byte 1 is the command, bytes 2..15 are
//!   arguments or reply data, and byte 16 is a checksum: `0x55 - sum(bytes 0..16)`.
//! * A reply echoes the command in byte 1 and carries its own checksum.
//!
//! Commands used here:
//!
//! | cmd    | meaning          | reply                                                        |
//! |--------|------------------|--------------------------------------------------------------|
//! | `0x03` | is mouse online  | byte 6 > 0 when the receiver has a live link to the mouse    |
//! | `0x04` | battery          | byte 6 = percent, byte 7 = charging, bytes 8..9 = mV (BE)    |

use std::{mem, ptr};

use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Devices::HumanInterfaceDevice::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::System::IO::*;

/// USB vendor ID used by Hitscan devices.
pub const VID: u16 = 0x3770;
/// Product IDs from the official utility's Config.ini.
pub const PID_WIRED: u16 = 0x0100;
pub const PID_DONGLE_1K: u16 = 0x0200;
pub const PID_DONGLE_8K: u16 = 0x0300;
/// Cable first: when the mouse is plugged in, ask it directly.
pub const PIDS: [u16; 3] = [PID_WIRED, PID_DONGLE_1K, PID_DONGLE_8K];

pub const REPORT_LEN: usize = 17;
pub const REPORT_ID: u8 = 0x08;
pub const CMD_ONLINE: u8 = 0x03;
pub const CMD_BATTERY: u8 = 0x04;

const REPLY_TIMEOUT_MS: u32 = 250;
const ATTEMPTS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Battery {
    /// The firmware's own percent. The official utility shows a smoothed,
    /// voltage-based figure instead; see `estimate`.
    pub percent: u8,
    pub charging: bool,
    pub millivolts: u16,
    /// Mouse ID from the online reply (bytes 7..9); the official utility keys its
    /// saved battery value by it. Zero when unknown.
    pub device_id: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// No Hitscan receiver or cabled mouse found.
    NoReceiver,
    /// Receiver present but the mouse is off, asleep, or out of range.
    MouseOffline,
    Battery(Battery),
}

/// Information about one matching HID collection, for the probe tool.
#[derive(Clone, Debug)]
pub struct Collection {
    pub path: String,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
}

impl Collection {
    pub fn is_command_channel(&self) -> bool {
        self.usage_page >= 0xFF00
            && self.input_len as usize == REPORT_LEN
            && self.output_len as usize == REPORT_LEN
    }
}

pub fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0x55u8, |acc, b| acc.wrapping_sub(*b))
}

pub fn build_request(cmd: u8, args: &[u8]) -> [u8; REPORT_LEN] {
    let mut buf = [0u8; REPORT_LEN];
    buf[0] = REPORT_ID;
    buf[1] = cmd;
    let n = args.len().min(REPORT_LEN - 3);
    buf[2..2 + n].copy_from_slice(&args[..n]);
    buf[REPORT_LEN - 1] = checksum(&buf[..REPORT_LEN - 1]);
    buf
}

pub fn is_valid_reply(cmd: u8, r: &[u8; REPORT_LEN]) -> bool {
    r[0] == REPORT_ID && r[1] == cmd && r[REPORT_LEN - 1] == checksum(&r[..REPORT_LEN - 1])
}

pub fn parse_battery(r: &[u8; REPORT_LEN]) -> Option<Battery> {
    let percent = r[6];
    if percent > 100 {
        return None;
    }
    Some(Battery {
        percent,
        charging: r[7] != 0,
        millivolts: u16::from_be_bytes([r[8], r[9]]),
        device_id: [0; 3],
    })
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// An open HID collection. Closed on drop.
pub struct Device {
    handle: HANDLE,
    pub info: Collection,
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

/// Lists every HID collection that belongs to a known Hitscan VID/PID.
pub fn enumerate() -> Vec<Collection> {
    let mut out = Vec::new();
    for path in hid_paths() {
        let lower = path.to_ascii_lowercase();
        if !lower.contains(&format!("vid_{VID:04x}")) {
            continue;
        }
        // Zero access rights is enough to read attributes and caps, and works even on
        // collections Windows holds exclusively (the mouse and keyboard ones).
        if let Some(h) = open(&path, 0) {
            if let Some(info) = describe(h, &path) {
                if PIDS.contains(&info.pid) {
                    out.push(info);
                }
            }
            unsafe { CloseHandle(h) };
        }
    }
    out.sort_by_key(|c| PIDS.iter().position(|p| *p == c.pid).unwrap_or(99));
    out
}

/// Opens every command-capable Hitscan collection, wired mouse first.
pub fn open_command_channels() -> Vec<Device> {
    enumerate()
        .into_iter()
        .filter(Collection::is_command_channel)
        .filter_map(|info| {
            open(&info.path, GENERIC_READ | GENERIC_WRITE).map(|handle| Device { handle, info })
        })
        .collect()
}

/// One-shot status read. Opens the device fresh each time so no stale reports
/// linger in the HID driver's queue between polls, and hot-plugging just works.
pub fn query() -> Status {
    let devices = open_command_channels();
    if devices.is_empty() {
        return Status::NoReceiver;
    }
    for dev in &devices {
        let online = dev.transact(CMD_ONLINE, &[]);
        // A cabled mouse is always there; through a receiver, ask first.
        if dev.info.pid != PID_WIRED && !online.is_some_and(|r| r[6] > 0) {
            continue;
        }
        if let Some(mut b) = dev.transact(CMD_BATTERY, &[]).as_ref().and_then(parse_battery) {
            if let Some(r) = online {
                b.device_id = [r[7], r[8], r[9]];
            }
            return Status::Battery(b);
        }
    }
    Status::MouseOffline
}

impl Device {
    /// Sends one command and waits for its matching reply.
    pub fn transact(&self, cmd: u8, args: &[u8]) -> Option<[u8; REPORT_LEN]> {
        let req = build_request(cmd, args);
        for _ in 0..ATTEMPTS {
            if let Some(r) = unsafe { self.exchange(&req) } {
                return Some(r);
            }
        }
        None
    }

    /// Posts a read, writes the request, then reads until a reply with a matching
    /// command byte arrives or the timeout passes. Other programs (like the official
    /// utility) may be talking to the receiver too; their replies are skipped.
    unsafe fn exchange(&self, req: &[u8; REPORT_LEN]) -> Option<[u8; REPORT_LEN]> {
        let read_ev = CreateEventW(ptr::null(), 1, 0, ptr::null());
        let write_ev = CreateEventW(ptr::null(), 1, 0, ptr::null());
        if read_ev.is_null() || write_ev.is_null() {
            close_all(&[read_ev, write_ev]);
            return None;
        }
        let mut buf = [0u8; REPORT_LEN];
        let mut read_ov: OVERLAPPED = mem::zeroed();
        read_ov.hEvent = read_ev;
        let mut result = None;

        let mut read_pending = self.start_read(&mut buf, &mut read_ov);
        if read_pending && self.write(req, write_ev) {
            let start = std::time::Instant::now();
            loop {
                let elapsed = start.elapsed().as_millis() as u32;
                if elapsed >= REPLY_TIMEOUT_MS {
                    break;
                }
                if WaitForSingleObject(read_ev, REPLY_TIMEOUT_MS - elapsed) != WAIT_OBJECT_0 {
                    break;
                }
                let mut got = 0u32;
                let ok = GetOverlappedResult(self.handle, &read_ov, &mut got, 0) != 0;
                read_pending = false;
                if ok && got as usize >= REPORT_LEN && is_valid_reply(req[1], &buf) {
                    result = Some(buf);
                    break;
                }
                read_pending = self.start_read(&mut buf, &mut read_ov);
                if !read_pending {
                    break;
                }
            }
        }
        if read_pending {
            // The buffer must not be written after we return: cancel and wait it out.
            CancelIoEx(self.handle, &read_ov);
            let mut got = 0u32;
            GetOverlappedResult(self.handle, &read_ov, &mut got, 1);
        }
        close_all(&[read_ev, write_ev]);
        result
    }

    unsafe fn start_read(&self, buf: &mut [u8; REPORT_LEN], ov: &mut OVERLAPPED) -> bool {
        ResetEvent(ov.hEvent);
        let ok = ReadFile(self.handle, buf.as_mut_ptr(), REPORT_LEN as u32, ptr::null_mut(), ov);
        ok != 0 || GetLastError() == ERROR_IO_PENDING
    }

    unsafe fn write(&self, req: &[u8; REPORT_LEN], ev: HANDLE) -> bool {
        let mut ov: OVERLAPPED = mem::zeroed();
        ov.hEvent = ev;
        let ok = WriteFile(self.handle, req.as_ptr(), REPORT_LEN as u32, ptr::null_mut(), &mut ov);
        if ok == 0 && GetLastError() != ERROR_IO_PENDING {
            return false;
        }
        let mut sent = 0u32;
        if WaitForSingleObject(ev, 1000) != WAIT_OBJECT_0 {
            CancelIoEx(self.handle, &ov);
            GetOverlappedResult(self.handle, &ov, &mut sent, 1);
            return false;
        }
        GetOverlappedResult(self.handle, &ov, &mut sent, 0) != 0 && sent as usize == REPORT_LEN
    }
}

unsafe fn close_all(handles: &[HANDLE]) {
    for h in handles {
        if !h.is_null() {
            CloseHandle(*h);
        }
    }
}

fn open(path: &str, access: u32) -> Option<HANDLE> {
    let p = wide(path);
    let h = unsafe {
        CreateFileW(
            p.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            ptr::null_mut(),
        )
    };
    (h != INVALID_HANDLE_VALUE).then_some(h)
}

fn describe(h: HANDLE, path: &str) -> Option<Collection> {
    unsafe {
        let mut attrs: HIDD_ATTRIBUTES = mem::zeroed();
        attrs.Size = mem::size_of::<HIDD_ATTRIBUTES>() as u32;
        if HidD_GetAttributes(h, &mut attrs) == 0 || attrs.VendorID != VID {
            return None;
        }
        let mut pp: PHIDP_PREPARSED_DATA = mem::zeroed();
        if HidD_GetPreparsedData(h, &mut pp) == 0 {
            return None;
        }
        let mut caps: HIDP_CAPS = mem::zeroed();
        let st = HidP_GetCaps(pp, &mut caps);
        HidD_FreePreparsedData(pp);
        if st != HIDP_STATUS_SUCCESS {
            return None;
        }
        Some(Collection {
            path: path.to_string(),
            pid: attrs.ProductID,
            usage_page: caps.UsagePage,
            usage: caps.Usage,
            input_len: caps.InputReportByteLength,
            output_len: caps.OutputReportByteLength,
            feature_len: caps.FeatureReportByteLength,
        })
    }
}

/// Device paths of every present HID interface.
fn hid_paths() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut guid: GUID = mem::zeroed();
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(
            &guid,
            ptr::null(),
            ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        );
        if set as isize == -1 {
            return out;
        }
        let mut index = 0;
        loop {
            let mut iface: SP_DEVICE_INTERFACE_DATA = mem::zeroed();
            iface.cbSize = mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if SetupDiEnumDeviceInterfaces(set, ptr::null(), &guid, index, &mut iface) == 0 {
                break;
            }
            index += 1;
            let mut needed = 0u32;
            SetupDiGetDeviceInterfaceDetailW(
                set,
                &iface,
                ptr::null_mut(),
                0,
                &mut needed,
                ptr::null_mut(),
            );
            if needed == 0 {
                continue;
            }
            // u32 storage keeps the struct correctly aligned.
            let mut storage = vec![0u32; (needed as usize + 3) / 4];
            let detail = storage.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(
                set,
                &iface,
                detail,
                needed,
                ptr::null_mut(),
                ptr::null_mut(),
            ) == 0
            {
                continue;
            }
            let start = ptr::addr_of!((*detail).DevicePath) as *const u16;
            let max = (needed as usize - 4) / 2;
            let len = (0..max).take_while(|&i| *start.add(i) != 0).count();
            out.push(String::from_utf16_lossy(std::slice::from_raw_parts(start, len)));
        }
        SetupDiDestroyDeviceInfoList(set);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_captured_reply() {
        // Real reply from a Hyperlight on a 1K receiver: 25%, not charging, 3729 mV.
        let r: [u8; 17] = [
            0x08, 0x04, 0x00, 0x00, 0x00, 0x02, 0x19, 0x00, 0x0e, 0x91, 0, 0, 0, 0, 0, 0, 0x8f,
        ];
        assert!(is_valid_reply(CMD_BATTERY, &r));
        assert_eq!(
            parse_battery(&r),
            Some(Battery { percent: 25, charging: false, millivolts: 3729, device_id: [0; 3] })
        );
    }

    #[test]
    fn request_layout() {
        let q = build_request(CMD_BATTERY, &[]);
        assert_eq!(&q[..2], &[0x08, 0x04]);
        assert_eq!(q[16], 0x55u8.wrapping_sub(0x0c));
    }
}
