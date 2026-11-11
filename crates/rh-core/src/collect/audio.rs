//! Audio: which sound cards are streaming, at what rate, for whom, and which
//! MIDI devices are plugged in, all from `/proc/asound`. PipeWire's own view
//! (quantum, DSP load, xruns) comes from `pipewire.rs`.

use super::{read, read_trim};
use crate::model::{Audio, AudioStream};
use std::path::Path;

/// `/proc/asound/cards`: card number to its long name ("Yeti X").
pub fn parse_cards(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|l| {
            let (num, rest) = l.trim_start().split_once(' ')?;
            let num = num.parse().ok()?;
            let name = rest.split_once("]: ")?.1;
            let name = name.split_once(" - ").map_or(name, |(_, n)| n);
            Some((num, name.trim().to_string()))
        })
        .collect()
}

/// A substream's `status` file: whether it runs, and for which process.
pub fn parse_status(text: &str) -> Option<(String, Option<u32>)> {
    let state = text.lines().find_map(|l| l.strip_prefix("state:"))?.trim();
    let pid = text
        .lines()
        .find_map(|l| l.strip_prefix("owner_pid"))
        .and_then(|v| v.trim_start_matches([' ', ':']).trim().parse().ok());
    Some((state.to_string(), pid))
}

/// A substream's `hw_params`: rate, format, channels, period and buffer.
pub fn parse_hw_params(text: &str) -> (Option<u32>, String, Option<u32>, Option<u32>, Option<u32>) {
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix(':')))
            .map(str::trim)
    };
    let num = |k: &str| get(k).and_then(|v| v.split_whitespace().next()?.parse().ok());
    (
        num("rate"),
        get("format").unwrap_or_default().to_string(),
        num("channels"),
        num("period_size"),
        num("buffer_size"),
    )
}

/// `/proc/asound/seq/clients`: hardware MIDI ports (kernel clients), without
/// the system timer and the MIDI Through loopback.
pub fn parse_midi(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("Client")?;
            let (_, rest) = rest.split_once(':')?;
            let name = rest.trim().strip_prefix('"')?;
            let (name, kind) = name.split_once('"')?;
            (kind.contains("Kernel") && name != "System" && name != "Midi Through")
                .then(|| name.to_string())
        })
        .collect()
}

/// Sound servers own the device on behalf of every app, so ALSA's buffer
/// says nothing about the latency an app hears.
fn is_server(name: &str) -> bool {
    matches!(
        name,
        "pipewire" | "pipewire-pulse" | "wireplumber" | "pulseaudio" | "jackd" | "jackdbus"
    )
}

/// Every running PCM substream under `root` (`/proc/asound`).
pub fn read_from(root: &Path) -> Audio {
    let cards = read(root.join("cards"))
        .map(|t| parse_cards(&t))
        .unwrap_or_default();
    let mut streams = vec![];
    for (num, name) in &cards {
        let card = root.join(format!("card{num}"));
        let Ok(rd) = std::fs::read_dir(&card) else {
            continue;
        };
        let mut pcms: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("pcm") && (n.ends_with('p') || n.ends_with('c')))
            .collect();
        pcms.sort();
        for pcm in pcms {
            let Ok(subs) = std::fs::read_dir(card.join(&pcm)) else {
                continue;
            };
            for sub in subs.flatten() {
                if !sub.file_name().to_string_lossy().starts_with("sub") {
                    continue;
                }
                let Some((state, pid)) =
                    read(sub.path().join("status")).and_then(|t| parse_status(&t))
                else {
                    continue;
                };
                if state != "RUNNING" {
                    continue;
                }
                let (rate, format, channels, period, _buffer) = read(sub.path().join("hw_params"))
                    .map(|t| parse_hw_params(&t))
                    .unwrap_or_default();
                let owner = pid.and_then(|p| read_trim(format!("/proc/{p}/comm")));
                let latency_ms = match (&owner, period, rate) {
                    (Some(o), Some(p), Some(r)) if !is_server(o) && r > 0 => {
                        Some(p as f32 / r as f32 * 1000.0)
                    }
                    _ => None,
                };
                streams.push(AudioStream {
                    card: name.clone(),
                    device: format!("card{num}/{pcm}"),
                    capture: pcm.ends_with('c'),
                    rate,
                    format,
                    channels,
                    owner,
                    latency_ms,
                });
            }
        }
    }
    let midi = read(root.join("seq/clients"))
        .map(|t| parse_midi(&t))
        .unwrap_or_default();
    Audio {
        streams,
        midi,
        graph: None,
    }
}

/// The soft "Max realtime priority" from a `/proc/<pid>/limits` file.
pub fn parse_rtprio_limit(limits: &str) -> Option<u32> {
    let rest = limits
        .lines()
        .find_map(|l| l.strip_prefix("Max realtime priority"))?;
    rest.split_whitespace().next()?.parse().ok()
}

/// A thread's name, realtime priority and scheduling policy from its `stat`
/// file (policy 1 is SCHED_FIFO, 2 is SCHED_RR).
pub fn parse_sched(stat: &str) -> Option<(String, u32, u32)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();
    // Fields after the name start at field 3 (state); rt_priority is field
    // 40 and policy 41.
    let f: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    Some((name, f.get(37)?.parse().ok()?, f.get(38)?.parse().ok()?))
}

/// PipeWire's realtime thread, if it has one: (thread name, priority).
/// `None` when PipeWire is not running; `Some(None)` when it runs without
/// one.
pub fn pipewire_rt() -> Option<Option<(String, u32)>> {
    let rd = std::fs::read_dir("/proc").ok()?;
    let pid = rd.flatten().find(|e| {
        e.file_name()
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_digit())
            && read_trim(e.path().join("comm")).as_deref() == Some("pipewire")
    })?;
    let tasks = std::fs::read_dir(pid.path().join("task")).ok()?;
    Some(tasks.flatten().find_map(|t| {
        let (name, prio, policy) = parse_sched(&read(t.path().join("stat"))?)?;
        (policy == 1 || policy == 2).then_some((name, prio))
    }))
}

/// Each USB sound card and its USB power control (`on`, or `auto` when it
/// may autosuspend).
pub fn usb_power() -> Vec<(String, String)> {
    let cards = read("/proc/asound/cards")
        .map(|t| parse_cards(&t))
        .unwrap_or_default();
    cards
        .into_iter()
        .filter_map(|(num, name)| {
            let dev = std::fs::canonicalize(format!("/sys/class/sound/card{num}/device")).ok()?;
            // The card hangs off a USB interface (`5-2.4.2:1.0`); power is on
            // its parent device.
            if !dev.to_string_lossy().contains("/usb") {
                return None;
            }
            let control = read_trim(dev.parent()?.join("power/control"))?;
            Some((name, control))
        })
        .collect()
}

pub fn scan() -> Audio {
    read_from(Path::new("/proc/asound"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // From a desktop with a Cam Link, two HDA codecs, a Yeti X and an Arctis
    // Nova Pro Wireless.
    const CARDS: &str = " 0 [C4K            ]: USB-Audio - Cam Link 4K
                      Elgato Cam Link 4K at usb-0000:74:00.4-1, super speed
 1 [NVidia         ]: HDA-Intel - HDA NVidia
                      HDA NVidia at 0xde080000 irq 152
 4 [X              ]: USB-Audio - Yeti X
                      Blue Microphones Yeti X at usb-0000:74:00.4-2.4.2, full speed
 5 [Wireless       ]: USB-Audio - Arctis Nova Pro Wireless
                      SteelSeries Arctis Nova Pro Wireless at usb-0000:74:00.4-2.4.4.4, full speed
";

    const STATUS: &str = "state: RUNNING
owner_pid   : 2610689
trigger_time: 142502.760580943
tstamp      : 148442.706250663
delay       : 2528
avail       : 31008
avail_max   : 31776
-----
hw_ptr      : 285120288
appl_ptr    : 285122048
";

    const HW_PARAMS: &str = "access: MMAP_INTERLEAVED
format: S24_3LE
subformat: STD
channels: 2
rate: 48000 (48000/1)
period_size: 512
buffer_size: 32768
";

    const CLIENTS: &str = "Client info
  cur  clients : 4
  peak clients : 18
  max  clients : 192

Client   0 : \"System\" [Kernel Legacy]
  Port   0 : \"Timer\" (Rwe-) [In/Out]
Client  14 : \"Midi Through\" [Kernel Legacy]
  Port   0 : \"Midi Through Port-0\" (RWe-) [In/Out]
Client  24 : \"Arturia KeyStep 37\" [Kernel Legacy]
  Port   0 : \"Arturia KeyStep 37 MIDI 1\" (RWeX) [In/Out]
Client 142 : \"PipeWire-System\" [User Legacy]
  Port   0 : \"input\" (rwe-) [In/Out]
";

    #[test]
    fn parses_proc_asound() {
        let cards = parse_cards(CARDS);
        assert_eq!(cards.len(), 4);
        assert_eq!(cards[0], (0, "Cam Link 4K".to_string()));
        assert_eq!(cards[3], (5, "Arctis Nova Pro Wireless".to_string()));
        assert_eq!(
            parse_status(STATUS),
            Some(("RUNNING".to_string(), Some(2610689)))
        );
        assert_eq!(parse_status("closed\n"), None);
        assert_eq!(
            parse_hw_params(HW_PARAMS),
            (
                Some(48000),
                "S24_3LE".to_string(),
                Some(2),
                Some(512),
                Some(32768)
            )
        );
        assert_eq!(parse_midi(CLIENTS), vec!["Arturia KeyStep 37".to_string()]);
    }

    #[test]
    fn reads_a_fake_tree() {
        let root = std::env::temp_dir().join(format!("rh-asound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sub = root.join("card4/pcm0c/sub0");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(root.join("cards"), CARDS).unwrap();
        std::fs::write(sub.join("status"), STATUS.replace("2610689", "999999999")).unwrap();
        std::fs::write(sub.join("hw_params"), HW_PARAMS).unwrap();
        std::fs::create_dir_all(root.join("card5/pcm0p/sub0")).unwrap();
        std::fs::write(root.join("card5/pcm0p/sub0/status"), "closed\n").unwrap();
        let a = read_from(&root);
        assert_eq!(a.streams.len(), 1);
        let s = &a.streams[0];
        assert_eq!(
            (s.card.as_str(), s.capture, s.rate),
            ("Yeti X", true, Some(48000))
        );
        assert_eq!(s.owner, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn realtime_facts() {
        let limits = "Limit                     Soft Limit           Hard Limit           Units
Max realtime priority     0                    0
Max realtime timeout      unlimited            unlimited            us
";
        assert_eq!(parse_rtprio_limit(limits), Some(0));
        // PipeWire's data loop on SCHED_RR at priority 20.
        let stat = "2610696 (data-loop.0) S 768 2610689 2610689 0 -1 4194368 322 0 0 0 212 485 0 0 -21 0 3 0 14107628 123781120 6423 18446744073709551615 94290749247488 94290749249705 140726294410896 0 0 0 16386 4096 0 1 0 0 -1 11 20 2 0 0 0 94290749258736 94290749259792 94291020660736 140726294412289 140726294412307 140726294412307 140726294413286 0";
        assert_eq!(parse_sched(stat), Some(("data-loop.0".to_string(), 20, 2)));
        let normal = "2610689 (pipewire) S 768 2610689 2610689 0 -1 4194304 70005 0 6 0 524 568 0 0 9 -11 3 0 14107627 123781120 6423 18446744073709551615 94290749247488 94290749249705 140726294410896 0 0 0 16386 4096 0 0 0 0 17 1 0 0 0 0 0 94290749258736 94290749259792 94291020660736 140726294412289 140726294412307 140726294412307 140726294413286 0";
        assert_eq!(parse_sched(normal), Some(("pipewire".to_string(), 0, 0)));
    }
}
