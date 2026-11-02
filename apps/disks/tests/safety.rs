#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use accesskit::{Action, ActionRequest, Role, TreeId};
use bande_disks::{
    Abilities, DisksView, EJECT, ERASE, Erase, Format, Job, Model, RESTORE, Request,
};
use bande_toolkit::Appearance;
use bande_toolkit::ui::Size;
use bande_volumes::udisks::Objects;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const UD: &str = "/org/freedesktop/UDisks2";
const APFS: &str = "7c3457ef-0000-11aa-aa11-00306543ecac";
const OBJECTS: &str = "
drives/APPLE Drive:Model=APPLE_SSD_AP0256Z NVMe.Controller:SmartCriticalWarning=
drives/USB Drive:Vendor=SanDisk,Model=Ultra,Ejectable=true,ConnectionBus=usb
block_devices/nvme0n1 Block:PreferredDevice=/dev/nvme0n1,Drive=APPLE,Size=250 PartitionTable:Type=gpt
block_devices/nvme0n1p2 Block:PreferredDevice=/dev/nvme0n1p2,Drive=APPLE,Size=114,IdType=apfs Partition:Table=nvme0n1,Offset=1,Size=114,Type=APFS
block_devices/nvme0n1p6 Block:PreferredDevice=/dev/nvme0n1p6,Drive=APPLE,Size=100,IdType=ext4 Partition:Table=nvme0n1,Offset=115,Size=100 Filesystem:MountPoints=/
block_devices/nvme0n1p7 Block:PreferredDevice=/dev/nvme0n1p7,Drive=APPLE,Size=20,IdType=ext4 Partition:Table=nvme0n1,Offset=215,Size=20 Filesystem:MountPoints=
block_devices/sda Block:PreferredDevice=/dev/sda,Drive=USB,Size=32 PartitionTable:Type=gpt
block_devices/sda1 Block:PreferredDevice=/dev/sda1,Drive=USB,Size=31,IdType=vfat,IdLabel=STICK Partition:Table=sda,Offset=1,Size=31 Filesystem:MountPoints=/media/anna/STICK";

fn value(key: &str, text: &str) -> OwnedValue {
    let text = text.replace('_', " ").replace("APFS", APFS);
    let value = match key {
        "Size" | "Offset" => Value::from(text.parse::<u64>().unwrap() * 1_000_000_000),
        "Ejectable" => Value::from(text == "true"),
        "Drive" => Value::from(OwnedObjectPath::try_from(format!("{UD}/drives/{text}")).unwrap()),
        "Table" => {
            Value::from(OwnedObjectPath::try_from(format!("{UD}/block_devices/{text}")).unwrap())
        }
        "PreferredDevice" => Value::from(format!("{text}\0").into_bytes()),
        "MountPoints" => Value::from(
            (!text.is_empty())
                .then(|| format!("{text}\0").into_bytes())
                .into_iter()
                .collect::<Vec<_>>(),
        ),
        "SmartCriticalWarning" => Value::from(Vec::<String>::new()),
        _ => Value::from(text),
    };
    value.try_to_owned().unwrap()
}

fn objects() -> Objects {
    let mut tree = HashMap::new();
    for line in OBJECTS.lines().filter(|line| !line.is_empty()) {
        let mut words = line.split(' ');
        let path = OwnedObjectPath::try_from(format!("{UD}/{}", words.next().unwrap())).unwrap();
        let interfaces = words.map(|word| {
            let (interface, properties) = word.split_once(':').unwrap();
            let properties = properties
                .split(',')
                .map(|pair| pair.split_once('=').unwrap());
            let properties = properties.map(|(key, text)| (key.to_owned(), value(key, text)));
            (
                format!("org.freedesktop.UDisks2.{interface}"),
                properties.collect(),
            )
        });
        tree.insert(path, interfaces.collect());
    }
    Objects::from(tree)
}

fn shown(view: &mut DisksView) -> String {
    view.ui().layout(Size::new(920.0, 600.0));
    format!("{:?}", view.ui_ref().accessibility())
}

#[test]
fn the_startup_disk_and_macos_stay_untouchable_while_a_usb_stick_is_erased_and_ejected() {
    let mut view = DisksView::new(Appearance::default());
    view.set_abilities(Abilities {
        formats: vec![Format::Ext4, Format::ExFat],
        ..Abilities::default()
    });
    view.set_model(Model::from_objects(&objects()));
    let listed = shown(&mut view);
    for expected in [
        "Internal",
        "External",
        "macOS",
        "SanDisk Ultra",
        "Apple Fabric",
        "Verified",
    ] {
        assert!(listed.contains(expected), "{expected} is not shown");
    }

    let key = |kind: &str, name: &str| format!("{kind}:{UD}/block_devices/{name}");
    for (kind, name) in [
        ("disk", "nvme0n1"),
        ("volume", "nvme0n1p2"),
        ("volume", "nvme0n1p6"),
    ] {
        view.select(&key(kind, name));
        assert!(
            !view.can(ERASE) && !view.can(RESTORE) && !view.can(EJECT),
            "{name} can be changed"
        );
        view.command(ERASE);
        assert!(!view.ui_ref().has_sheet() && view.take_requests().is_empty());
    }
    view.select(&key("volume", "nvme0n1p7"));
    view.command(ERASE);
    let asked = shown(&mut view);
    assert!(asked.contains("(/dev/nvme0n1p7, 20 GB)") && asked.contains("started up from"));

    view.select(&key("disk", "sda"));
    view.command(ERASE);
    assert!(shown(&mut view).contains("“SanDisk Ultra” (/dev/sda, 32 GB) will permanently delete"));
    let tree = view.ui_ref().accessibility();
    let erase = tree
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == Role::Button && node.label() == Some("Erase"));
    let clicks: Vec<_> = erase
        .map(|(id, _)| ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: *id,
            data: None,
        })
        .collect();
    assert!(clicks.iter().any(|click| view.ui().perform(click)));
    view.pump();
    view.command(EJECT);
    let requests = view.take_requests();
    let sda = key("disk", "sda").replace("disk:", "");
    let erased = |disk: &str, erase: &Erase| {
        disk == sda && erase.name == "Untitled" && erase.format == Format::Ext4
    };
    assert!(
        matches!(&requests[..], [Request::Run(Job::EraseDisk(disk, _, erase))] if erased(disk, erase)),
        "{requests:?}"
    );
}
