window-title = Activity Monitor

tab-cpu = CPU
tab-memory = Memory
tab-disk = Disk
tab-network = Network

column-name = Name
column-interface = Interface
column-cpu = % CPU
column-cpu-time = CPU Time
column-threads = Threads
column-pid = PID
column-user = User
column-memory = Memory
column-resident = Resident
column-read = Read/s
column-written = Written/s
column-read-total = Bytes Read
column-written-total = Bytes Written
column-received = Received/s
column-sent = Sent/s
column-received-total = Bytes Received
column-sent-total = Bytes Sent

group-kernel = Kernel

summary-system = System
summary-user = User
summary-idle = Idle
summary-processes = Processes
summary-threads = Threads
summary-physical = Physical Memory
summary-used = Memory Used
summary-cached = Cached Files
summary-swap = Swap Used
summary-compressed = Compressed
summary-pressure = Memory Pressure
summary-read-rate = Data Read/s
summary-written-rate = Data Written/s
summary-read = Data Read
summary-written = Data Written
summary-received-rate = Data Received/s
summary-sent-rate = Data Sent/s
summary-received = Data Received
summary-sent = Data Sent

size-bytes = { $count ->
    [one] { $count } byte
   *[other] { $count } bytes
}
size-kilobytes = { $value } KB
size-megabytes = { $value } MB
size-gigabytes = { $value } GB
size-terabytes = { $value } TB
per-second = { $size }/s

search-placeholder = Search
stop-label = Quit Process
inspect-label = Inspect Process
tab-label = Show
button-ok = OK
button-cancel = Cancel
button-quit = Quit
button-force-quit = Force Quit
quit-title = { $count ->
    [one] Do you want to quit “{ $name }”?
   *[other] Do you want to quit the { $count } selected items?
}
quit-message = Quit asks it to close. Force Quit ends it at once, and any unsaved changes are lost.
quit-denied = “{ $name }” belongs to another user.
quit-denied-message = Only its owner or an administrator can quit it.

force-quit-title = Force Quit Apps
force-quit-hint = If an app does not respond, select it and click Force Quit.
force-quit-empty = No apps are open.
force-quit-confirm = { $count ->
    [one] Do you want to force quit “{ $name }”?
   *[other] Do you want to force quit the { $count } selected items?
}
force-quit-confirm-message = Any unsaved changes are lost.
not-responding = { $name } (Not Responding)

menu-quit = Quit…
menu-force-quit = Force Quit…
menu-show-in-files = Show in Files
menu-copy-name = Copy Name
menu-copy-pid = Copy PID
menu-quit-process = Quit Process…
menu-force-quit-process = Force Quit Process…
menu-find = Find
menu-inspect = Inspect
menu-edit = Edit
menu-view = View

inspector-process = { $name } ({ $pid })
inspector-info = Info
inspector-threads = Threads
inspector-files = Open Files
inspector-memory = Memory Map
inspector-value = Value
inspector-parent = Parent
inspector-children = Children
inspector-command = Command
inspector-executable = Executable
inspector-running = Running For
inspector-virtual = Virtual Memory
inspector-tid = TID
inspector-state = State
inspector-fd = FD
inspector-path = Path
inspector-address = Address
inspector-size = Size
inspector-permissions = Access
inspector-mapping = Mapping
inspector-not-permitted = Only its owner or an administrator can see this.

state-running = Running
state-sleeping = Sleeping
state-waiting = Waiting for the disk
state-zombie = Ended, waiting for its parent
state-stopped = Stopped
state-idle = Idle
state-unknown = Unknown
state-ended = Ended
