app-name = Skivverktyg
menu-file = Arkiv
menu-images = Avbildningar
no-disks = Inga skivor hittades.
working = Det här kan ta en stund. Koppla inte från skivan.

location-internal = Intern
location-external = Extern
location-image = Skivavbildningar

volume-untitled = Namnlös
volume-macos = macOS
volume-apple-system = Apple-system

connection-usb = USB
connection-sd = Secure Digital
connection-firewire = FireWire
connection-apple-fabric = Apple Fabric
connection-pci = PCI Express
connection-optical = ATAPI
connection-sata = SATA
connection-unknown = Okänd
connection-image = Skivavbildning

scheme-gpt = GUID-partitionsschema
scheme-mbr = Master Boot Record
scheme-none = Inget partitionsschema

format-ext4 = Linux (ext4)
format-ext4-encrypted = Linux (ext4, krypterad)
format-exfat = ExFAT
format-fat32 = MS-DOS (FAT32)
format-ntfs = Windows NT (NTFS)

fs-ext = Linux ({ $kind })
fs-fat = MS-DOS (FAT)
fs-exfat = ExFAT
fs-ntfs = Windows NT (NTFS)
fs-apfs = APFS
fs-hfs = Mac OS Extended
fs-luks = Krypterad (LUKS)
fs-swap = Linux-växlingsutrymme
fs-iso = ISO 9660
fs-none = Inte formaterad
fs-encrypted = { $kind }, krypterad

subtitle-disk = Fysisk skiva, { $connection } { $location } • { $scheme }
subtitle-volume = Fysisk volym, { $location } • { $format }
subtitle-image = Skivavbildning • { $file }

free-space = Ledigt
used-space = Använt
locked = Låst
not-mounted = Inte monterad
device-described = { $name } ({ $device }, { $size })

info-location = Plats:
info-capacity = Kapacitet:
info-connection = Anslutning:
info-children = Antal underordnade:
info-scheme = Partitionsschema:
info-smart = S.M.A.R.T.-status:
info-device = Enhet:
info-type = Typ:
info-model = Modell:
info-serial = Serienummer:
info-temperature = Temperatur:
info-hours = Drifttimmar:
info-access = Åtkomst:
info-mount-point = Monteringspunkt:
info-format = Format:
info-available = Tillgängligt:
info-uuid = UUID:

type-image = Skivavbildning
type-ssd = Halvledarlagring
type-disk = Skiva
smart-verified = Verifierad
smart-failing = Felar
smart-unsupported = Stöds inte
access-apple = Apple-system (skrivskyddad)
access-macos = macOS (skrivskyddad)
access-startup = Startskiva (skrivskyddad)
access-in-use = Används (skrivskyddad)
startup-disk-warning = Den här partitionen finns på skivan som datorn startade från.

tool-add = +
tool-delete = −
action-first-aid = Första hjälpen
action-partition = Partitionera
action-erase = Radera
action-restore = Återställ
action-mount = Montera
action-unmount = Avmontera
action-eject = Mata ut

menu-first-aid = Första hjälpen …
menu-add-partition = Lägg till partition …
menu-delete-partition = Radera partition …
menu-resize = Ändra storlek på partition …
menu-erase = Radera …
menu-restore = Återställ …
menu-rename = Byt namn …
menu-image-from = Avbildning från markering …
menu-open-image = Öppna skivavbildning …

button-cancel = Avbryt
button-done = Klar
button-stop = Stoppa
button-erase = Radera
button-add = Lägg till
button-delete = Radera
button-resize = Ändra storlek
button-run = Kör
button-rename = Byt namn
button-unlock = Lås upp
button-restore = Återställ
button-choose = Välj
button-ok = OK

field-name = Namn
field-format = Format
field-scheme = Schema
field-size = Storlek
field-password = Lösenord
field-verify = Bekräfta

erase-title = Radera { $name }?
erase-message = Om du raderar { $target } raderas alla data på den permanent. Du kan inte ångra det här.
erasing = Raderar { $name } …
erase-complete = Raderingen är klar.
erase-complete-message = Klicka på Klar för att fortsätta.

passphrase-title = Välj ett lösenord
passphrase-message = Ange ett lösenord för att kryptera volymen. Du behöver det varje gång du öppnar volymen, och ingen kan läsa dess data utan det.
passphrase-empty = Lösenordet får inte vara tomt.
passphrase-mismatch = Lösenorden stämmer inte överens. Ange samma lösenord i båda fälten.

add-title = Lägg till en partition på { $name }
add-message = { $free } ledigt utrymme finns tillgängligt.
adding = Lägger till { $name } …
delete-title = Radera { $name }?
delete-message = Om du raderar partitionen { $target } raderas alla data på den. Du kan inte ångra det här.
deleting = Raderar partitionen …
resize-title = Ändra storlek på { $name }
resize-message = Välj en ny storlek för { $target }, upp till { $largest }.
resizing = Ändrar storlek på { $name } …
resize-cannot-grow = Det här formatet kan inte bli större.
resize-cannot-shrink = Det här formatet kan inte bli mindre.
size-invalid = Ange en storlek på minst 16 MB och högst { $largest }.
partition-complete = Partitioneringen är klar.
partition-complete-message = Klicka på Klar för att fortsätta.

first-aid-title = Köra Första hjälpen på { $name }?
first-aid-message = Första hjälpen söker efter fel på volymen och reparerar dem.
first-aid-unmounts = Första hjälpen söker efter fel på volymen och reparerar dem. Volymen avmonteras medan det pågår och monteras igen efteråt.
checking = Kontrollerar filsystemet …
first-aid-complete = Första hjälpen är klar.
first-aid-ok = Volymen verkar vara i ordning.
first-aid-repaired = Volymen hade fel som har reparerats.
first-aid-failed = Volymen har fel som Första hjälpen inte kunde reparera. Säkerhetskopiera dess data och radera den.

rename-title = Byt namn på { $name }
unlock-title = Lås upp { $name }
unlock-message = Ange lösenordet för att låsa upp den här volymen.

restore-title = Återställa { $name }?
restore-message = Om du återställer { $target } från { $image } ({ $image_size }) ersätts alla data på den. Du kan inte ångra det här.
restore-too-big = Avbildningen är för stor.
restore-too-big-message = { $image } ryms inte på { $target }.
restoring = Återställer { $name } …
restore-complete = Återställningen är klar.
restore-complete-message = Klicka på Klar för att fortsätta.
imaging = Skapar en avbildning av { $name } …
image-complete = Avbildningen har skapats.
image-saved = Avbildningen sparades i { $file }.
copy-stopped = Åtgärden stoppades.

save-image = Spara avbildning
choose-image = Välj en avbildning
open-image = Öppna skivavbildning
disk-images = Skivavbildningar

failed-mount = Det gick inte att montera volymen.
failed-unmount = Det gick inte att avmontera volymen.
failed-eject = Det gick inte att mata ut skivan.
failed-erase = Raderingen misslyckades.
failed-partition = Det gick inte att ändra partitionerna.
failed-first-aid = Första hjälpen misslyckades.
failed-rename = Det gick inte att byta namn på volymen.
failed-unlock = Det gick inte att låsa upp volymen.
failed-restore = Återställningen misslyckades.
failed-image = Det gick inte att skapa avbildningen.
failed-open-image = Det gick inte att öppna skivavbildningen.
quit-busy = Skivverktyg arbetar fortfarande.
quit-busy-message = Vänta tills åtgärden är klar innan du avslutar.
