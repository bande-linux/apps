window-title = Aktivitetskontroll

tab-cpu = CPU
tab-memory = Minne
tab-energy = Energi
tab-disk = Disk
tab-network = Nätverk
tab-gpu = GPU

column-name = Namn
column-cpu = % CPU
column-cpu-time = CPU-tid
column-threads = Trådar
column-pid = PID
column-user = Användare
column-memory = Minne
column-compressed = Komprimerat
column-resident = Residentärt
column-read = Läst/s
column-written = Skrivet/s
column-read-total = Lästa byte
column-written-total = Skrivna byte
column-received = Mottaget/s
column-sent = Skickat/s
column-received-total = Mottagna byte
column-sent-total = Skickade byte
column-gpu = % GPU
column-gpu-time = GPU-tid
column-gpu-memory = GPU-minne
column-energy-impact = Energipåverkan
column-wakeups = Väckningar/s
column-preventing-sleep = Förhindrar vila
preventing-sleep-yes = Ja
preventing-sleep-no = Nej

group-kernel = Kärna

summary-system = System
summary-user = Användare
summary-idle = Inaktiv
summary-processes = Processer
summary-threads = Trådar
summary-physical = Fysiskt minne
summary-used = Minne som används
summary-cached = Cachade filer
summary-swap = Använd växlingsfil
summary-compressed = Komprimerat
summary-app = Programminne
summary-kernel = Fast minne
summary-reads = Läsningar/s
summary-writes = Skrivningar/s
summary-read-rate = Data läst/s
summary-written-rate = Data skriven/s
summary-read = Data läst
summary-written = Data skriven
summary-received-rate = Data mottagen/s
summary-sent-rate = Data skickad/s
summary-received = Data mottagen
summary-sent = Data skickad
summary-gpu = GPU-användning
summary-gpu-memory = GPU-minne
summary-gpu-processes = Processer som använder GPU:n
summary-energy-impact = Energipåverkan
summary-wakeups = Väckningar/s
summary-remaining-charge = Återstående laddning
summary-time-remaining = Återstående tid
summary-time-until-full = Tid till full laddning
summary-time-on-battery = Tid på batteri

graph-span = { $minutes } min
graph-cpu = CPU { $total } %, användare { $user } %, system { $system } %
graph-memory = Minnestryck { $level }, { $used } % används
graph-pressure = Minnestryck
graph-cpu-title = CPU-belastning
graph-energy-title = Energipåverkan
graph-disk-title = Data läst och skriven
graph-network-title = Data mottagen och skickad
graph-gpu-title = GPU-användning
pressure-normal = normalt
pressure-elevated = förhöjt
pressure-critical = kritiskt
graph-disk = Disk läst { $read }, skriven { $written }
graph-network = Nätverk mottaget { $received }, skickat { $sent }
graph-gpu = GPU { $used } %
graph-energy = Energipåverkan { $impact }
graph-battery = Batteri (senaste 12 timmarna)
graph-battery-span = 12 tim
graph-battery-charge = Batteri { $charge } %, energipåverkan { $impact }
cores-performance = Prestanda
cores-efficiency = Effektivitet
core-label = Kärna { $number }

size-bytes = { $count ->
    [one] { $count } byte
   *[other] { $count } byte
}
size-kilobytes = { $value } KB
size-megabytes = { $value } MB
size-gigabytes = { $value } GB
size-terabytes = { $value } TB
per-second = { $size }/s

search-placeholder = Sök
search-filtered = Sök { $filter }
stop-label = Avsluta process
inspect-label = Granska process
tab-label = Visa
button-ok = OK
button-cancel = Avbryt
button-quit = Avsluta
button-force-quit = Tvinga avslut
quit-title = { $count ->
    [one] Vill du avsluta ”{ $name }”?
   *[other] Vill du avsluta de { $count } markerade objekten?
}
quit-message = Avsluta ber processen att stänga sig. Tvinga avslut stoppar den direkt, och osparade ändringar går förlorade.
quit-denied = ”{ $name }” tillhör en annan användare.
quit-denied-message = Bara ägaren eller en administratör kan avsluta den.

force-quit-confirm = { $count ->
    [one] Vill du tvinga ”{ $name }” att avsluta?
   *[other] Vill du tvinga de { $count } markerade objekten att avsluta?
}
force-quit-confirm-message = Osparade ändringar går förlorade.
not-responding = { $name } (svarar inte)

menu-quit = Avsluta …
menu-force-quit = Tvinga avslut …
menu-show-in-files = Visa i Filer
menu-copy-name = Kopiera namn
menu-copy-pid = Kopiera PID
menu-quit-process = Avsluta process …
menu-force-quit-process = Tvinga processen att avsluta …
menu-find = Sök
menu-inspect = Granska
menu-view = Visa
menu-show-cores = Visa CPU-kärnor
menu-send-signal = Skicka signal till process
menu-signal-asks = { $signal } …
signal-hup = Lägg på (SIGHUP)
signal-int = Avbryt (SIGINT)
signal-stop = Pausa (SIGSTOP)
signal-cont = Fortsätt (SIGCONT)
signal-usr1 = Användare 1 (SIGUSR1)
signal-usr2 = Användare 2 (SIGUSR2)
signal-confirm = { $count ->
    [one] Vill du skicka { $signal } till ”{ $name }”?
   *[other] Vill du skicka { $signal } till de { $count } markerade objekten?
}
signal-message = En process som inte hanterar den här signalen avslutas direkt, och osparade ändringar går förlorade.
signal-pause-message = En pausad process gör ingenting förrän den får Fortsätt. Om du pausar en del av skrivbordet kan skärmen frysa.
button-send = Skicka
menu-frequency = Uppdateringsfrekvens
frequency-very-often = Mycket ofta ({ $seconds } s)
frequency-often = Ofta ({ $seconds } s)
frequency-normally = Normalt ({ $seconds } s)
filter-all = Alla processer
filter-hierarchy = Alla processer, hierarkiskt
filter-mine = Mina processer
filter-system = Systemprocesser
filter-active = Aktiva processer
filter-apps = Appar
filter-selected = Markerade processer

inspector-process = { $name } ({ $pid })
inspector-info = Info
inspector-threads = Trådar
inspector-files = Öppna filer
inspector-memory = Minneskarta
inspector-value = Värde
inspector-section-process = Process
inspector-section-family = Förälder och barn
inspector-section-command = Kommando
inspector-section-resources = Resurser
inspector-section-statistics = Statistik
inspector-faults = Sidfel
inspector-major-faults = Sidfel från disk
inspector-switches = Kontextbyten
inspector-involuntary-switches = Avbruten
inspector-read-calls = Läsanrop
inspector-write-calls = Skrivanrop
inspector-waiting-in = Väntar i
inspector-unit = Enhet
inspector-preventing-sleep = Förhindrar vila
inspector-parent = Förälder
inspector-children = Barn
inspector-command = Kommando
inspector-executable = Körbar fil
inspector-running = Har körts i
inspector-virtual = Virtuellt minne
inspector-tid = TID
inspector-state = Tillstånd
inspector-fd = FD
inspector-path = Sökväg
inspector-address = Adress
inspector-size = Storlek
inspector-permissions = Åtkomst
inspector-mapping = Mappning
inspector-not-permitted = Bara ägaren eller en administratör kan se detta.

state-running = Körs
state-sleeping = Sover
state-waiting = Väntar på disken
state-zombie = Avslutad, väntar på sin förälder
state-stopped = Stoppad
state-idle = Inaktiv
state-unknown = Okänt
state-ended = Avslutad
no-match = Inga processer matchar ”{ $text }”
clear-search = Rensa sökning
port-listening = { $protocol } { $local } (lyssnar)
