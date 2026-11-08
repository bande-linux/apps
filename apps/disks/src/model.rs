use std::collections::HashSet;
use std::path::Path;

use bande_l10n::{text, tr};
use bande_volumes::udisks::{
    ATA, BLOCK, DRIVE, ENCRYPTED, FILESYSTEM, LOOP, NVME, Objects, PARTITION, PARTITION_TABLE,
    SWAPSPACE,
};

const MIB: u64 = 1 << 20;
const SMALLEST_FREE: u64 = 4 * MIB;
const KELVIN: f64 = 273.15;
const USER_MOUNTS: [&str; 3] = ["/media/", "/run/media/", "/mnt/"];
const APPLE_DRIVE: &str = "APPLE SSD";
const MACOS_TYPES: [&str; 3] = [
    "7c3457ef-0000-11aa-aa11-00306543ecac",
    "48465300-0000-11aa-aa11-00306543ecac",
    "53746f72-6167-11aa-aa11-00306543ecac",
];
const APPLE_SYSTEM_TYPES: [&str; 4] = [
    "69646961-6700-11aa-aa11-00306543ecac",
    "52637672-7900-11aa-aa11-00306543ecac",
    "426f6f74-0000-11aa-aa11-00306543ecac",
    "5265636f-7665-11aa-aa11-00306543ecac",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Internal,
    External,
    Images,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Gpt,
    Mbr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apple {
    MacOs,
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Smart {
    pub failing: bool,
    pub celsius: Option<f64>,
    pub hours: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Volume {
    pub path: String,
    pub filesystem: Option<String>,
    pub name: String,
    pub device: String,
    pub size: u64,
    pub offset: u64,
    pub fs_type: String,
    pub uuid: String,
    pub encrypted: bool,
    pub locked: bool,
    pub mount_points: Vec<String>,
    pub apple: Option<Apple>,
    pub in_use: bool,
    pub partition: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Disk {
    pub path: String,
    pub name: String,
    pub device: String,
    pub size: u64,
    pub group: Group,
    pub model: String,
    pub serial: String,
    pub connection: String,
    pub scheme: Option<Scheme>,
    pub smart: Option<Smart>,
    pub system: bool,
    pub apple: bool,
    pub solid_state: bool,
    pub ejectable: bool,
    pub image: Option<String>,
    pub volumes: Vec<Volume>,
    pub free: Vec<(u64, u64)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub disks: Vec<Disk>,
}

pub const DISK_KEY: &str = "disk:";
pub const VOLUME_KEY: &str = "volume:";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Selected<'a> {
    Disk(&'a Disk),
    Volume(&'a Disk, &'a Volume),
}

fn bytes_text(bytes: Vec<u8>) -> String {
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(&bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

fn system_mount(point: &str) -> bool {
    !USER_MOUNTS.iter().any(|root| point.starts_with(root))
}

fn apple_kind(objects: &Objects, block: &str, apple_drive: bool) -> Option<Apple> {
    if !apple_drive {
        return None;
    }
    let kind = objects.text(block, PARTITION, "Type").to_lowercase();
    if APPLE_SYSTEM_TYPES.contains(&kind.as_str()) || !objects.has(block, PARTITION) {
        return Some(Apple::System);
    }
    let macos =
        MACOS_TYPES.contains(&kind.as_str()) || objects.text(block, BLOCK, "IdType") == "apfs";
    macos.then_some(Apple::MacOs)
}

fn cleartext(objects: &Objects, block: &str) -> Option<String> {
    objects.path(block, ENCRYPTED, "CleartextDevice")
}

fn volume(objects: &Objects, block: &str, apple_drive: bool) -> Volume {
    let encrypted = objects.has(block, ENCRYPTED);
    let opened = cleartext(objects, block);
    let shown = opened.clone().unwrap_or_else(|| block.to_owned());
    let filesystem = Some(shown.clone()).filter(|path| objects.has(path, FILESYSTEM));
    let mount_points = filesystem
        .as_deref()
        .map(|path| objects.mount_points(path))
        .unwrap_or_default();
    let swapping = objects.flag(&shown, SWAPSPACE, "Active");
    let apple = apple_kind(objects, block, apple_drive);
    let name = [
        objects.text(&shown, BLOCK, "IdLabel"),
        objects.text(block, PARTITION, "Name"),
    ]
    .into_iter()
    .find(|name| !name.is_empty());
    let name = match apple {
        Some(Apple::MacOs) => tr!("macOS"),
        Some(Apple::System) => name.unwrap_or_else(|| tr!("Apple system")),
        None => name.unwrap_or_else(|| tr!("Untitled")),
    };
    Volume {
        path: block.to_owned(),
        filesystem,
        name,
        device: objects.device(block),
        size: objects.get(block, BLOCK, "Size").unwrap_or(0),
        offset: objects.get(block, PARTITION, "Offset").unwrap_or(0),
        fs_type: objects.text(&shown, BLOCK, "IdType"),
        uuid: objects.text(&shown, BLOCK, "IdUUID"),
        encrypted,
        locked: encrypted && opened.is_none(),
        in_use: swapping || mount_points.iter().any(|point| system_mount(point)),
        mount_points,
        apple,
        partition: objects.has(block, PARTITION),
    }
}

fn smart(objects: &Objects, drive: &str) -> Option<Smart> {
    if objects.has(drive, NVME) {
        let warnings: Vec<String> = objects
            .get(drive, NVME, "SmartCriticalWarning")
            .unwrap_or_default();
        let kelvin: u16 = objects.get(drive, NVME, "SmartTemperature").unwrap_or(0);
        return Some(Smart {
            failing: !warnings.is_empty(),
            celsius: (kelvin > 0).then(|| f64::from(kelvin) - KELVIN),
            hours: objects.get(drive, NVME, "SmartPowerOnHours"),
        });
    }
    if !objects.flag(drive, ATA, "SmartSupported") || !objects.flag(drive, ATA, "SmartEnabled") {
        return None;
    }
    let kelvin: f64 = objects.get(drive, ATA, "SmartTemperature").unwrap_or(0.0);
    let seconds: Option<u64> = objects.get(drive, ATA, "SmartPowerOnSeconds");
    Some(Smart {
        failing: objects.flag(drive, ATA, "SmartFailing"),
        celsius: (kelvin > 0.0).then_some(kelvin - KELVIN),
        hours: seconds
            .filter(|seconds| *seconds > 0)
            .map(|seconds| seconds / 3600),
    })
}

fn connection(objects: &Objects, drive: &str, apple_drive: bool) -> String {
    let key = match objects.text(drive, DRIVE, "ConnectionBus").as_str() {
        "usb" => text!("USB"),
        "sdio" => text!("Secure Digital"),
        "ieee1394" => text!("FireWire"),
        _ if apple_drive => text!("Apple Fabric"),
        _ if objects.has(drive, NVME) => text!("PCI Express"),
        _ if objects.flag(drive, DRIVE, "Optical") => text!("ATAPI"),
        _ if objects.has(drive, ATA) => text!("SATA"),
        _ => text!("Unknown"),
    };
    key.tr()
}

fn free_regions(size: u64, volumes: &[Volume], objects: &Objects) -> Vec<(u64, u64)> {
    let mut taken: Vec<(u64, u64)> = volumes
        .iter()
        .filter(|volume| volume.partition && !objects.flag(&volume.path, PARTITION, "IsContained"))
        .map(|volume| (volume.offset, volume.offset + volume.size))
        .collect();
    taken.sort_unstable();
    let mut free = Vec::new();
    let mut start = MIB;
    for (from, to) in taken {
        if from >= start + SMALLEST_FREE {
            free.push((start, from - start));
        }
        start = start.max(to);
    }
    let end = size.saturating_sub(MIB);
    if end >= start + SMALLEST_FREE {
        free.push((start, end - start));
    }
    free
}

fn disk(objects: &Objects, block: &str) -> Option<Disk> {
    let size: u64 = objects.get(block, BLOCK, "Size").unwrap_or(0);
    let drive = objects.path(block, BLOCK, "Drive");
    let image = objects
        .get::<Vec<u8>>(block, LOOP, "BackingFile")
        .map(bytes_text)
        .filter(|file| !file.is_empty());
    if size == 0 || (drive.is_none() && image.is_none()) {
        return None;
    }
    let drive = drive.unwrap_or_default();
    let model = objects.text(&drive, DRIVE, "Model");
    let apple = model.starts_with(APPLE_DRIVE);
    let mut partitions: Vec<String> = objects
        .paths()
        .map(|path| path.to_string())
        .filter(|path| objects.path(path, PARTITION, "Table").as_deref() == Some(block))
        .collect();
    partitions.sort_by_key(|path| objects.get::<u64>(path, PARTITION, "Offset"));
    let scheme = match objects.text(block, PARTITION_TABLE, "Type").as_str() {
        "gpt" => Some(Scheme::Gpt),
        "dos" => Some(Scheme::Mbr),
        _ => None,
    };
    let whole = scheme.is_none()
        && (objects.has(block, FILESYSTEM)
            || objects.has(block, ENCRYPTED)
            || !objects.text(block, BLOCK, "IdUsage").is_empty());
    let volumes: Vec<Volume> = match whole {
        true => vec![volume(objects, block, apple)],
        false => partitions
            .iter()
            .filter(|path| !objects.flag(path, PARTITION, "IsContainer"))
            .map(|path| volume(objects, path, apple))
            .collect(),
    };
    let free = match scheme {
        Some(_) => free_regions(size, &volumes, objects),
        None => Vec::new(),
    };
    let vendor = objects.text(&drive, DRIVE, "Vendor");
    let name = match &image {
        Some(file) => Path::new(file)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.clone()),
        None => [vendor.as_str(), model.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    };
    let external = objects.flag(&drive, DRIVE, "Removable")
        || objects.flag(&drive, DRIVE, "MediaRemovable")
        || matches!(
            objects.text(&drive, DRIVE, "ConnectionBus").as_str(),
            "usb" | "sdio" | "ieee1394"
        );
    let group = match (&image, external) {
        (Some(_), _) => Group::Images,
        (None, true) => Group::External,
        (None, false) => Group::Internal,
    };
    let ejectable = image.is_some()
        || objects.flag(&drive, DRIVE, "Ejectable")
        || objects.flag(&drive, DRIVE, "CanPowerOff");
    Some(Disk {
        path: block.to_owned(),
        name: match name.is_empty() {
            true => objects.device(block),
            false => name,
        },
        device: objects.device(block),
        size,
        group,
        serial: objects.text(&drive, DRIVE, "Serial"),
        connection: match &image {
            Some(_) => tr!("Disk Image"),
            None => connection(objects, &drive, apple),
        },
        scheme,
        smart: smart(objects, &drive),
        system: volumes.iter().any(|volume| volume.in_use),
        apple,
        solid_state: objects.get::<i32>(&drive, DRIVE, "RotationRate") == Some(0),
        ejectable,
        image,
        volumes,
        free,
        model,
    })
}

impl Model {
    pub fn from_objects(objects: &Objects) -> Self {
        let mut disks: Vec<Disk> = objects
            .paths()
            .filter(|path| {
                objects.has(path, BLOCK)
                    && !objects.has(path, PARTITION)
                    && objects.path(path, BLOCK, "CryptoBackingDevice").is_none()
            })
            .filter_map(|path| disk(objects, path))
            .collect();
        disks.sort_by(|a, b| (a.group as u8, &a.device).cmp(&(b.group as u8, &b.device)));
        let mut seen = HashSet::new();
        for disk in &mut disks {
            if !seen.insert(disk.name.clone()) {
                let device = disk.device.trim_start_matches("/dev/");
                disk.name = format!("{} ({device})", disk.name);
            }
        }
        Self { disks }
    }

    pub fn find(&self, key: &str) -> Option<Selected<'_>> {
        if let Some(path) = key.strip_prefix(DISK_KEY) {
            return self
                .disks
                .iter()
                .find(|disk| disk.path == path)
                .map(Selected::Disk);
        }
        let path = key.strip_prefix(VOLUME_KEY)?;
        self.disks.iter().find_map(|disk| {
            disk.volumes
                .iter()
                .find(|volume| volume.path == path)
                .map(|volume| Selected::Volume(disk, volume))
        })
    }
}

impl Disk {
    pub fn key(&self) -> String {
        format!("{DISK_KEY}{}", self.path)
    }

    pub fn protected(&self) -> bool {
        self.system || self.apple
    }

    pub fn whole_volume(&self) -> Option<&Volume> {
        self.volumes.iter().find(|volume| volume.path == self.path)
    }

    pub fn largest_free(&self) -> Option<(u64, u64)> {
        self.free.iter().copied().max_by_key(|(_, size)| *size)
    }

    pub fn free_after(&self, volume: &Volume) -> u64 {
        let end = volume.offset + volume.size;
        self.free
            .iter()
            .find(|(start, _)| start.abs_diff(end) < MIB)
            .map_or(0, |(start, size)| start + size - end)
    }

    pub fn mounted(&self) -> Vec<&Volume> {
        self.volumes
            .iter()
            .filter(|volume| !volume.mount_points.is_empty())
            .collect()
    }
}

impl Volume {
    pub fn key(&self) -> String {
        format!("{VOLUME_KEY}{}", self.path)
    }

    pub fn protected(&self) -> bool {
        self.in_use || self.apple.is_some()
    }

    pub fn mounted(&self) -> bool {
        !self.mount_points.is_empty()
    }
}
