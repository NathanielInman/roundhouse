//! Disks and the filesystems on them, found from `/proc/self/mountinfo` with
//! no config.
//!
//! - btrfs subvolumes and bind mounts fold into one filesystem, keyed by its
//!   source device, and filesystems fold into their physical disk.
//! - Network shares are sized on a helper thread with a deadline, so a NAS
//!   that went away never stalls a sample.
//! - Automount points that have nothing mounted show as "on demand". They are
//!   never touched, so opening the drawer does not wake a share.

use super::{read, read_trim};
use crate::model::{Disk, DiskKind, Filesystem};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::{Duration, Instant};

const LOCAL: &[&str] = &[
    "ext2", "ext3", "ext4", "btrfs", "xfs", "f2fs", "vfat", "exfat", "ntfs", "ntfs3", "fuseblk",
    "zfs", "bcachefs", "jfs", "reiserfs", "hfsplus", "apfs",
];
const NETWORK: &[&str] = &[
    "cifs",
    "smb3",
    "nfs",
    "nfs4",
    "fuse.sshfs",
    "fuse.rclone",
    "9p",
    "ceph",
    "glusterfs",
];
const SKIP_UNDER: &[&str] = &[
    "/proc",
    "/sys",
    "/dev",
    "/run/user",
    "/run/credentials",
    "/snap",
    "/var/lib/docker",
    "/var/lib/containers",
];

/// How long a share gets to report its size before the sample moves on.
const NET_DEADLINE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// The directory of the filesystem that is mounted (`/` unless it is a
    /// bind mount or a btrfs subvolume).
    pub root: String,
    pub mount: String,
    pub opts: String,
    pub fstype: String,
    pub source: String,
}

/// Decodes the octal escapes mountinfo uses for spaces and friends.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            let v = (b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0');
            out.push(v);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn parse_mountinfo(text: &str) -> Vec<MountEntry> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let l: Vec<&str> = left.split(' ').collect();
            let r: Vec<&str> = right.split(' ').collect();
            if l.len() < 6 || r.len() < 2 {
                return None;
            }
            Some(MountEntry {
                root: unescape(l[3]),
                mount: unescape(l[4]),
                opts: l[5].to_string(),
                fstype: r[0].to_string(),
                source: unescape(r[1]),
            })
        })
        .collect()
}

fn skipped(mount: &str) -> bool {
    SKIP_UNDER
        .iter()
        .any(|p| mount == *p || mount.starts_with(&format!("{p}/")))
}

/// The disk a block device lives on: through device-mapper slaves (LUKS,
/// LVM) and from partition to whole disk.
fn disk_of(name: &str, depth: usize) -> String {
    let base = Path::new("/sys/class/block").join(name);
    if depth < 4 {
        if let Ok(rd) = std::fs::read_dir(base.join("slaves")) {
            if let Some(first) = rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .min()
            {
                return disk_of(&first, depth + 1);
            }
        }
    }
    if base.join("partition").exists() {
        if let Ok(real) = std::fs::canonicalize(&base) {
            if let Some(parent) = real.parent().and_then(|p| p.file_name()) {
                return parent.to_string_lossy().to_string();
            }
        }
    }
    name.to_string()
}

fn model_of(disk: &str) -> Option<String> {
    let dev = Path::new("/sys/class/block").join(disk).join("device");
    read_trim(dev.join("model")).map(|m| m.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// `(total, used, avail)` in bytes.
pub type Sizes = (u64, u64, u64);

#[cfg(unix)]
pub fn statvfs(path: &str) -> Option<Sizes> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let f = s.f_frsize as u64;
    let total = s.f_blocks as u64 * f;
    let free = s.f_bfree as u64 * f;
    let avail = s.f_bavail as u64 * f;
    Some((total, total.saturating_sub(free), avail))
}

#[cfg(not(unix))]
pub fn statvfs(_path: &str) -> Option<Sizes> {
    None
}

struct Probe {
    rx: Receiver<Option<Sizes>>,
    since: Instant,
}

#[derive(Default)]
pub struct Storage {
    probes: HashMap<String, Probe>,
    last: HashMap<String, Sizes>,
    models: HashMap<String, Option<String>>,
}

impl Storage {
    /// Sizes a share on a helper thread. Returns the freshest answer and
    /// whether the share has stopped answering.
    fn net_size(&mut self, mount: &str) -> (Option<Sizes>, bool) {
        if let Some(p) = self.probes.get(mount) {
            match p.rx.try_recv() {
                Ok(v) => {
                    self.probes.remove(mount);
                    if let Some(v) = v {
                        self.last.insert(mount.to_string(), v);
                    }
                    return (self.last.get(mount).copied(), false);
                }
                Err(TryRecvError::Empty) => {
                    let stale = p.since.elapsed() > Duration::from_secs(2);
                    return (self.last.get(mount).copied(), stale);
                }
                Err(TryRecvError::Disconnected) => {
                    self.probes.remove(mount);
                }
            }
        }
        let (tx, rx) = mpsc::channel();
        let m = mount.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(statvfs(&m));
        });
        match rx.recv_timeout(NET_DEADLINE) {
            Ok(v) => {
                if let Some(v) = v {
                    self.last.insert(mount.to_string(), v);
                }
                (self.last.get(mount).copied(), false)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.probes.insert(
                    mount.to_string(),
                    Probe {
                        rx,
                        since: Instant::now(),
                    },
                );
                (self.last.get(mount).copied(), false)
            }
            Err(RecvTimeoutError::Disconnected) => (None, true),
        }
    }

    pub fn sample(&mut self, hide: &[String], drive_temps: &[(String, f32)]) -> Vec<Disk> {
        let Some(text) = read("/proc/self/mountinfo") else {
            return vec![];
        };
        let entries = parse_mountinfo(&text);
        let real_mounts: Vec<&str> = entries
            .iter()
            .filter(|e| e.fstype != "autofs")
            .map(|e| e.mount.as_str())
            .collect();

        // source -> (fstype, mounts, readonly)
        let mut by_source: BTreeMap<String, (String, Vec<String>, bool)> = BTreeMap::new();
        // Mount points that show a filesystem's own root rather than a bind
        // of one of its directories.
        let top: Vec<&str> = entries
            .iter()
            .filter(|e| e.root == "/")
            .map(|e| e.mount.as_str())
            .collect();
        let mut on_demand = vec![];
        for e in &entries {
            if skipped(&e.mount) {
                continue;
            }
            if e.fstype == "autofs" {
                if !real_mounts.contains(&e.mount.as_str()) && !hide.contains(&e.mount) {
                    on_demand.push(e.mount.clone());
                }
                continue;
            }
            if !LOCAL.contains(&e.fstype.as_str()) && !NETWORK.contains(&e.fstype.as_str()) {
                continue;
            }
            let ro = e.opts.split(',').any(|o| o == "ro");
            let slot = by_source
                .entry(e.source.clone())
                .or_insert_with(|| (e.fstype.clone(), vec![], true));
            if !slot.1.contains(&e.mount) {
                slot.1.push(e.mount.clone());
            }
            slot.2 &= ro;
        }

        let mut disks: BTreeMap<String, Disk> = BTreeMap::new();
        for (source, (fstype, mut mounts, readonly)) in by_source {
            mounts.retain(|m| !hide.contains(m));
            if mounts.is_empty() {
                continue;
            }
            // The main mount: "/", else one showing the filesystem's root
            // (`/scratch`, not its `/tmp` bind), else the shortest path.
            mounts.sort_by_key(|m| (m != "/", !top.contains(&m.as_str()), m.len(), m.clone()));
            let network = NETWORK.contains(&fstype.as_str());
            let (sizes, stale) = if network {
                self.net_size(&mounts[0])
            } else {
                (statvfs(&mounts[0]), false)
            };
            let Some((total, used, avail)) = sizes else {
                continue;
            };
            if total == 0 {
                continue;
            }
            let (key, kind) = if network {
                (source.clone(), DiskKind::Network)
            } else {
                let dev = std::fs::canonicalize(&source)
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));
                match dev {
                    Some(d) => (disk_of(&d, 0), DiskKind::Local),
                    None => (source.clone(), DiskKind::Local),
                }
            };
            let model = if kind == DiskKind::Local {
                self.models
                    .entry(key.clone())
                    .or_insert_with(|| model_of(&key))
                    .clone()
            } else {
                None
            };
            let temp = drive_temps.iter().find(|(n, _)| *n == key).map(|(_, t)| *t);
            let disk = disks.entry(key.clone()).or_insert_with(|| Disk {
                name: key.clone(),
                model,
                kind,
                temp,
                filesystems: vec![],
            });
            disk.filesystems.push(Filesystem {
                source,
                fstype,
                mounts,
                total,
                used,
                avail,
                readonly,
                stale,
            });
        }

        let mut out: Vec<Disk> = disks.into_values().collect();
        for d in &mut out {
            d.filesystems.sort_by(|a, b| {
                let ka = a.mounts.first().map(|m| m != "/").unwrap_or(true);
                let kb = b.mounts.first().map(|m| m != "/").unwrap_or(true);
                ka.cmp(&kb).then(a.mounts.cmp(&b.mounts))
            });
        }
        let has_root = |d: &Disk| {
            d.filesystems
                .iter()
                .any(|f| f.mounts.iter().any(|m| m == "/"))
        };
        out.sort_by(|a, b| {
            (!has_root(a))
                .cmp(&!has_root(b))
                .then((a.kind as u8).cmp(&(b.kind as u8)))
                .then(a.name.cmp(&b.name))
        });
        for m in on_demand {
            out.push(Disk {
                name: Path::new(&m)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| m.clone()),
                model: None,
                kind: DiskKind::OnDemand,
                temp: None,
                filesystems: vec![Filesystem {
                    source: "automount".into(),
                    fstype: "autofs".into(),
                    mounts: vec![m],
                    ..Default::default()
                }],
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MI: &str = "\
22 1 0:21 /@ / rw,noatime shared:1 - btrfs /dev/nvme0n1p2 rw,compress=zstd:3,ssd,subvol=/@
45 22 0:21 /@home /home rw,noatime shared:2 - btrfs /dev/nvme0n1p2 rw,subvol=/@home
50 22 259:5 / /data rw,relatime shared:3 - ext4 /dev/nvme1n1p1 rw
51 22 259:5 /sites /home/nate/Sites rw,relatime shared:3 - ext4 /dev/nvme1n1p1 rw
26 49 0:34 / /proc/sys/fs/binfmt_misc rw,relatime shared:13 - autofs systemd-1 rw,fd=43
256 67 0:62 / /home/nate/Rime rw,relatime shared:223 - autofs systemd-1 rw,fd=113
499 256 0:87 / /home/nate/Rime rw,relatime shared:614 - cifs //192.168.1.51/private rw,vers=3.1.1
60 22 0:70 / /mnt/My\\040Disk rw - vfat /dev/sda1 rw
";

    #[test]
    fn parses_mountinfo() {
        let e = parse_mountinfo(MI);
        assert_eq!(e.len(), 8);
        assert_eq!(
            e[0],
            MountEntry {
                root: "/@".into(),
                mount: "/".into(),
                opts: "rw,noatime".into(),
                fstype: "btrfs".into(),
                source: "/dev/nvme0n1p2".into()
            }
        );
        assert_eq!(e[6].fstype, "cifs");
        assert_eq!(e[6].source, "//192.168.1.51/private");
        assert_eq!(e[7].mount, "/mnt/My Disk");
    }

    #[test]
    fn skips_system_trees() {
        assert!(skipped("/proc/sys/fs/binfmt_misc"));
        assert!(skipped("/run/user/1000/doc"));
        assert!(!skipped("/run/media/nate/usb"));
        assert!(!skipped("/home/nate/Rime"));
    }
}
