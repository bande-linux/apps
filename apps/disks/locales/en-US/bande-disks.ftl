app-name = Disk Utility
menu-file = File
menu-images = Images
no-disks = No disks found.
working = This may take a while. Don’t disconnect the disk.

location-internal = Internal
location-external = External
location-image = Disk Images

volume-untitled = Untitled
volume-macos = macOS
volume-apple-system = Apple system

connection-usb = USB
connection-sd = Secure Digital
connection-firewire = FireWire
connection-apple-fabric = Apple Fabric
connection-pci = PCI Express
connection-optical = ATAPI
connection-sata = SATA
connection-unknown = Unknown
connection-image = Disk Image

scheme-gpt = GUID Partition Map
scheme-mbr = Master Boot Record
scheme-none = No Partition Map

format-ext4 = Linux (ext4)
format-ext4-encrypted = Linux (ext4, Encrypted)
format-exfat = ExFAT
format-fat32 = MS-DOS (FAT32)
format-ntfs = Windows NT (NTFS)

fs-ext = Linux ({ $kind })
fs-fat = MS-DOS (FAT)
fs-exfat = ExFAT
fs-ntfs = Windows NT (NTFS)
fs-apfs = APFS
fs-hfs = Mac OS Extended
fs-luks = Encrypted (LUKS)
fs-swap = Linux Swap
fs-iso = ISO 9660
fs-none = Not formatted
fs-encrypted = { $kind }, Encrypted

subtitle-disk = { $connection } { $location } Physical Disk • { $scheme }
subtitle-volume = { $location } Physical Volume • { $format }
subtitle-image = Disk Image • { $file }

free-space = Free
used-space = Used
locked = Locked
not-mounted = Not Mounted
device-described = { $name } ({ $device }, { $size })

info-location = Location:
info-capacity = Capacity:
info-connection = Connection:
info-children = Child count:
info-scheme = Partition Map:
info-smart = S.M.A.R.T. status:
info-device = Device:
info-type = Type:
info-model = Model:
info-serial = Serial:
info-temperature = Temperature:
info-hours = Power-on hours:
info-access = Access:
info-mount-point = Mount Point:
info-format = Format:
info-available = Available:
info-uuid = UUID:

type-image = Disk Image
type-ssd = Solid state
type-disk = Disk
smart-verified = Verified
smart-failing = Failing
smart-unsupported = Not Supported
access-apple = Apple system (read-only)
access-macos = macOS (read-only)
access-startup = Startup disk (read-only)
access-in-use = In use (read-only)
startup-disk-warning = This partition is on the disk this computer started up from.

tool-add = +
tool-delete = −
action-first-aid = First Aid
action-partition = Partition
action-erase = Erase
action-restore = Restore
action-mount = Mount
action-unmount = Unmount
action-eject = Eject

menu-first-aid = First Aid…
menu-add-partition = Add Partition…
menu-delete-partition = Delete Partition…
menu-resize = Resize Partition…
menu-erase = Erase…
menu-restore = Restore…
menu-rename = Rename…
menu-image-from = Image from Selection…
menu-open-image = Open Disk Image…

button-cancel = Cancel
button-done = Done
button-stop = Stop
button-erase = Erase
button-add = Add
button-delete = Delete
button-resize = Resize
button-run = Run
button-rename = Rename
button-unlock = Unlock
button-restore = Restore
button-choose = Choose
button-ok = OK

field-name = Name
field-format = Format
field-scheme = Scheme
field-size = Size
field-password = Password
field-verify = Verify

erase-title = Erase { $name }?
erase-message = Erasing { $target } will permanently delete all data stored on it. You can’t undo this action.
erasing = Erasing { $name }…
erase-complete = Erase process is complete.
erase-complete-message = Click Done to continue.

passphrase-title = Choose a Password
passphrase-message = Enter a password to encrypt the volume. You need it every time you open the volume, and nobody can read its data without it.
passphrase-empty = The password can’t be empty.
passphrase-mismatch = The passwords don’t match. Enter the same password in both fields.

add-title = Add a Partition to { $name }
add-message = { $free } of free space is available.
adding = Adding { $name }…
delete-title = Delete { $name }?
delete-message = Deleting the partition { $target } deletes all data stored on it. You can’t undo this action.
deleting = Deleting the partition…
resize-title = Resize { $name }
resize-message = Choose a new size for { $target }, up to { $largest }.
resizing = Resizing { $name }…
resize-cannot-grow = This format can’t grow.
resize-cannot-shrink = This format can’t shrink.
size-invalid = Enter a size of at least 16 MB and at most { $largest }.
partition-complete = Partition process is complete.
partition-complete-message = Click Done to continue.

first-aid-title = Run First Aid on { $name }?
first-aid-message = First Aid checks the volume for errors and repairs them.
first-aid-unmounts = First Aid checks the volume for errors and repairs them. The volume is unmounted while it runs and mounted again afterwards.
checking = Checking the file system…
first-aid-complete = First Aid process has completed.
first-aid-ok = The volume appears to be OK.
first-aid-repaired = The volume had errors that were repaired.
first-aid-failed = The volume has errors that First Aid couldn’t repair. Back up its data and erase it.

rename-title = Rename { $name }
unlock-title = Unlock { $name }
unlock-message = Enter the password to unlock this volume.

restore-title = Restore { $name }?
restore-message = Restoring { $target } from { $image } ({ $image_size }) replaces all data stored on it. You can’t undo this action.
restore-too-big = The image is too big.
restore-too-big-message = { $image } doesn’t fit on { $target }.
restoring = Restoring { $name }…
restore-complete = Restore process is complete.
restore-complete-message = Click Done to continue.
imaging = Creating an image of { $name }…
image-complete = Image created.
image-saved = The image was saved to { $file }.
copy-stopped = The operation was stopped.

save-image = Save Image
choose-image = Choose an Image
open-image = Open Disk Image
disk-images = Disk Images

failed-mount = Couldn’t mount the volume.
failed-unmount = Couldn’t unmount the volume.
failed-eject = Couldn’t eject the disk.
failed-erase = Erase process has failed.
failed-partition = Couldn’t change the partitions.
failed-first-aid = First Aid process has failed.
failed-rename = Couldn’t rename the volume.
failed-unlock = Couldn’t unlock the volume.
failed-restore = Restore process has failed.
failed-image = Couldn’t create the image.
failed-open-image = Couldn’t open the disk image.
quit-busy = Disk Utility is still working.
quit-busy-message = Wait until the operation finishes before quitting.
