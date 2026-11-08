# Velox: USB passthrough and bridged networking through polkit helpers

A design for the owner to decide on. Both features need one privileged step, and that conflicts with
decision 4 in `docker-vm.md` ("never runs `sudo`, never configures the system"). The proposal
narrows that rule instead of dropping it. Velox and QEMU stay unprivileged. Each privileged step is
one small method on a system D-Bus service that polkit authorises. The service holds no state and
starts on demand.

## The service

- `bande-velox-helper`: a small Rust binary in its own package, `bande-velox-helper`, which Velox
  suggests. It runs as root, is started by D-Bus on demand (`SystemdService=` with
  `DynamicUser=no` and `ProtectSystem=strict`), and exits when idle.
- It is on the system bus as `org.bande.VeloxHelper1`. Each method checks its own polkit action with
  `CheckAuthorization` for the calling process. No method takes a path, a command or a shell string.
- Debian practice: polkit policy in `/usr/share/polkit-1/actions`, a D-Bus policy in
  `/usr/share/dbus-1/system.d`, a systemd unit, and no setuid bits. It is the same shape as
  udisks2 and fwupd.

## USB passthrough

- QEMU opens a USB device that the user cannot open from a file descriptor:
  `device_add usb-host,hostdevice=/dev/fdset/N`. The descriptor reaches QEMU over QMP `add-fd` on
  the control socket Velox already uses. QEMU wraps it with `libusb_wrap_sys_device`, so QEMU
  itself needs no rights. This needs QEMU 7.1 or later; check it on the mini first.
- `OpenUsbDevice(bus: u8, address: u8) -> fd`, with polkit action
  `org.bande.velox.usb-passthrough`, set to `auth_admin_keep` (allowed only for an active local
  session).
  - The helper reads the device from sysfs, opens `/dev/bus/usb/BBB/DDD` read-write and returns the
    descriptor.
  - It refuses hubs and the internal Apple devices (keyboard, trackpad, T2/SEP, the hub that carries
    them).
  - It refuses any device whose interfaces hold a mounted filesystem.
- For devices the seat ACL already opens (`uaccess`: phones, cameras, security keys), Velox opens
  the node itself with no prompt. The helper is only for the rest.
- UI: a USB menu in the console window, like VMware's Removable Devices. It lists devices from the
  udev monitor (event-driven), and each device has a "Connect to <machine>" check item.
  - Unplugging the device, or choosing the item again, sends `device_del`.
  - Velox keeps the choice per machine by vendor, product and serial number, and connects the
    device again when it reappears (VMware's "remember this device").
- Snapshots that keep memory refuse while a device is connected, because the device state does not
  migrate.

## Bridged and host-only networking

- passt stays the default ("Shared with this computer"). Two more modes:
  - **Bridged:** the guest appears on the local network. This works only on wired Ethernet: a Wi-Fi
    station cannot bridge, and macvtap on Wi-Fi drops the guest's frames. On Wi-Fi the choice is
    disabled with that reason. Most Macs are on Wi-Fi.
  - **Host-only:** a private network between this computer and its machines, with no route out.
- `CreateTap(mode, bridge_or_network) -> name`, with polkit action `org.bande.velox.network`
  (`auth_admin_keep`).
  - The helper creates a persistent tap owned by the caller's uid (`TUNSETOWNER`). QEMU then opens
    it with no privilege (`-netdev tap,ifname=…,script=no`).
  - Bridged mode attaches the tap to the Ethernet bridge, which NetworkManager makes on request
    (a `bridge` connection with the wired port as a slave). This way the helper never edits network
    configuration files.
  - Host-only mode attaches the tap to `velox0`, a NetworkManager bridge in `shared` mode with no
    uplink. NetworkManager runs dnsmasq for it, which is Debian practice.
- `DeleteTap(name)` runs when the machine stops. When the session ends, a `StopWhenUnneeded`
  user unit per machine calls it, so no tap outlives the machine.
- Debian's alternative, `qemu-bridge-helper`, needs setuid or `/etc/qemu/bridge.conf`, and it
  covers bridging only. The polkit service covers both modes and asks the person each time.

## What the owner decides

1. Whether decision 4 may be narrowed to "no root except a polkit-authorised helper the person
   approves".
2. The polkit defaults: `auth_admin_keep` (proposed) or `yes` for the active seat.
3. Whether bridged mode is worth shipping when it works only on wired Ethernet. Host-only covers
   most local testing on Wi-Fi.
