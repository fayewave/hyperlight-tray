//! Console diagnostic: lists Hitscan HID collections and dumps the raw replies to the
//! online and battery commands. Handy for checking a new model or filing a bug report.
//! Pass `--settings` to also dump and decode the mouse's configuration (read-only).
//! `--set-polling <hz>` changes the polling rate and verifies it by reading back.
use hyperlight::{estimate, settings};
use hyperlight::hid::{self, CMD_BATTERY, CMD_ONLINE, PID_WIRED};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

fn main() {
    // --settings also dumps and decodes the mouse's configuration (read-only).
    let args: Vec<String> = std::env::args().collect();
    let want_settings = args.iter().any(|a| a == "--settings");
    // --set-polling <hz>: the one write exposed here; all four rates are hardware-tested.
    if let Some(i) = args.iter().position(|a| a == "--set-polling") {
        let Some(code) = args.get(i + 1).and_then(|v| v.parse().ok()).and_then(settings::polling_code)
        else {
            eprintln!("usage: hyperlight-probe --set-polling <1000|500|250|125>");
            std::process::exit(2);
        };
        // Writes are only acknowledged while the mouse is awake; reads work regardless.
        let start = std::time::Instant::now();
        let mut told = false;
        let ok = loop {
            let devs = hid::open_command_channels();
            let awake = devs.iter().find(|d| {
                d.info.pid == PID_WIRED
                    || d.transact(CMD_ONLINE, &[]).is_some_and(|r| r[6] > 0)
            });
            if let Some(d) = awake {
                break d.set_pair(settings::OFF_POLLING, code);
            }
            if start.elapsed().as_secs() >= 180 {
                break false;
            }
            if !told {
                println!("Mouse is asleep. Move it to wake it up...");
                told = true;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        };
        println!("{}", if ok { "polling rate set" } else { "failed (mouse asleep or not found?)" });
        std::process::exit(if ok { 0 } else { 1 });
    }
    let all = hid::enumerate();
    if all.is_empty() {
        println!("No Hitscan device found (VID {:04x}, PIDs {:04x?}).", hid::VID, hid::PIDS);
        std::process::exit(1);
    }
    println!("Hitscan HID collections:");
    for c in &all {
        println!(
            "  pid={:04x} usage={:04x}:{:04x} in={} out={} feat={}{}\n    {}",
            c.pid,
            c.usage_page,
            c.usage,
            c.input_len,
            c.output_len,
            c.feature_len,
            if c.is_command_channel() { "  <- command channel" } else { "" },
            c.path
        );
    }
    for dev in hid::open_command_channels() {
        println!("\nQuerying pid={:04x}:", dev.info.pid);
        match dev.transact(CMD_ONLINE, &[]) {
            Some(r) => println!(
                "  online  : {}  -> {}, id {:02x}{:02x}{:02x}",
                hex(&r),
                if dev.info.pid == PID_WIRED { "cabled" } else if r[6] > 0 { "mouse connected" } else { "mouse offline" },
                r[7], r[8], r[9]
            ),
            None => println!("  online  : no reply"),
        }
        if want_settings {
            match dev.read_flash(0, settings::BLOCK_LEN) {
                Some(raw) => {
                    println!("  settings block:");
                    for (i, row) in raw.chunks(16).enumerate() {
                        println!("    {:04x}: {}", i * 16, hex(row));
                    }
                    for line in settings::describe(&raw) {
                        println!("  {line}");
                    }
                }
                None => println!("  settings: no reply (mouse asleep?)"),
            }
            match dev.long_range_mode() {
                Some(m) => println!("  Long Distance    : {}", if m != 0 { "on" } else { "off" }),
                None => println!("  Long Distance    : no reply"),
            }
        }
        match dev.transact(CMD_BATTERY, &[]) {
            Some(r) => {
                print!("  battery : {}", hex(&r));
                match hid::parse_battery(&r) {
                    Some(b) => println!(
                        "  -> firmware {}%, {}, {} mV (voltage curve: {}%)",
                        b.percent,
                        if b.charging { "charging" } else { "not charging" },
                        b.millivolts,
                        estimate::target(&b)
                    ),
                    None => println!("  -> unparseable"),
                }
            }
            None => println!("  battery : no reply"),
        }
    }
    println!("\nSummary: {:?}", hid::query());
}
