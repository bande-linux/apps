app-name = Velox
pane-containers = Containers
sidebar-containers = Containers
menu-container = Container
menu-view = View
action-start = Start
action-stop = Stop
find = Find
filter-containers = Filter containers
column-name = Name
column-image = Image
column-status = Status
column-ports = Ports
pending-start = Starting…
pending-stop = Stopping…
project-running = { $running } of { $total } running
empty-title = No Containers
empty-hint = Run one with docker run.
engine-connecting = Connecting…
engine-none = No engine
engine-docker = Docker { $version }
engine-podman = Podman { $version }
engine-rootless = { $engine } · rootless
cannot-start = Cannot start “{ $name }”
cannot-stop = Cannot stop “{ $name }”
button-ok = OK
copy = Copy
not-installed = Install a container engine to continue
docker-not-running = Docker is not running
podman-not-running = Podman is not running
no-access = No access to Docker
relogin = Log out and back in to finish
too-old = Update Docker to continue
steps-docker = Docker Engine (recommended)
steps-podman = Or Podman (rootless, from Debian)
steps-start-docker = Start Docker
steps-start-podman = Start Podman
steps-access = Add yourself to the docker group, then log out and back in
docker-group-note = The docker group is root-equivalent.
podman-rootless = Podman, rootless
pane-overview = Overview
pane-images = Images
pane-volumes = Volumes
pane-networks = Networks
menu-file = File
menu-image = Image
menu-volume = Volume
menu-network = Network
button-cancel = Cancel
action-failed = Action failed
remove = Remove
remove-ellipsis = Remove…
remove-selected = Remove Selected
prune-unused-ellipsis = Prune Unused…
copy-name = Copy Name
column-repository = Repository
column-tag = Tag
column-arch = Arch
column-in-use = In Use
column-created = Created
column-size = Size
column-used-by = Used By
column-driver = Driver
column-subnet = Subnet
column-containers = Containers
age-now = Just now
age-minutes = { $count ->
    [one] 1 minute ago
   *[other] { $count } minutes ago
}
age-hours = { $count ->
    [one] 1 hour ago
   *[other] { $count } hours ago
}
age-days = { $count ->
    [one] 1 day ago
   *[other] { $count } days ago
}
age-weeks = { $count ->
    [one] 1 week ago
   *[other] { $count } weeks ago
}
age-months = { $count ->
    [one] 1 month ago
   *[other] { $count } months ago
}
age-years = { $count ->
    [one] 1 year ago
   *[other] { $count } years ago
}
arch-multi = multi
filter-images = Filter images
pull = Pull
pull-placeholder = Pull image (e.g. nginx:latest)
pull-starting = Starting…
pulling = Pulling { $reference }: { $status }
pull-image-menu = Pull Image…
run-image-menu = Run Image…
run-image = Run…
run-ellipsis = Run…
copy-reference = Copy Reference
copy-image-id = Copy Image ID
prune-images = Prune Unused Images
prune-images-title = Prune unused images?
prune-images-message = Removes every image no container uses. This cannot be undone.
pruned-images = Pruned unused images
remove-image-title = Remove “{ $name }”?
remove-images-title = Remove { $count } images?
remove-images = Remove { $count } Images
remove-image-message = An image still used by a container cannot be removed.
cannot-pull = Cannot pull “{ $name }”
cannot-run = Cannot run “{ $name }”
cannot-remove-images = Cannot remove every image
reclaimed = Reclaimed { $size }.
run-title = Run Image
run-message = Starts detached. The new container appears in Containers.
run = Run
run-image-label = Image
run-image-placeholder = nginx:latest
run-name = Name
run-name-placeholder = Optional
run-ports = Ports
run-ports-placeholder = 8080:80, 5432:5432
run-env = Environment
run-env-placeholder = KEY=value, OTHER=value
run-volumes = Volumes
run-volumes-placeholder = data:/var/lib/data
run-restart = Restart
restart-no = Never
restart-always = Always
restart-unless-stopped = Unless stopped
restart-on-failure = On failure
run-remove = Remove when it exits (--rm)
filter-volumes = Filter volumes
dangling = dangling
no-containers = no containers
volume-count = { $count ->
    [one] 1 volume
   *[other] { $count } volumes
}
copy-mountpoint = Copy Mountpoint
export-volume = Export as tar…
export-volume-title = Export Volume
import-volume = Import Volume
import-volume-ellipsis = Import Volume…
import-volume-title = Import Volume
import-volume-message = Creates a volume from { $file }.
import = Import
volume-name = Volume name
filter-tar = Tar archive
prune-volumes = Prune Unused Volumes
prune-volumes-title = Prune unused volumes?
prune-volumes-message = Removes every volume no container uses, with its data. This cannot be undone.
pruned-volumes = Pruned unused volumes
remove-volume-title = Remove “{ $name }”?
remove-volumes-title = Remove { $count } volumes?
remove-volume-message = Its data is deleted. A volume still used by a container cannot be removed.
imported-volume = Imported into “{ $name }”
exported-volume = Exported “{ $name }”
cannot-import = Cannot import into “{ $name }”
cannot-export = Cannot export “{ $name }”
cannot-remove-volumes = Cannot remove every volume
filter-networks = Filter networks
no-attached-containers = No attached containers
create = Create
create-network = New Network
create-network-ellipsis = New Network…
create-network-title = New Network
create-network-message = A bridge network on this computer.
network-name = Name
network-name-placeholder = backend
network-subnet = Subnet
network-subnet-placeholder = Automatic
remove-network-title = Remove “{ $name }”?
remove-networks-title = Remove { $count } networks?
remove-network-message = Disconnect its containers first.
cannot-create-network = Cannot create “{ $name }”
cannot-remove-networks = Cannot remove every network
reclaim-space-menu = Reclaim Space…
reclaim-title = Reclaim Space
reclaim-message = Removes engine data nothing uses. Volumes hold real data: leave them off unless you are sure.
reclaim = Reclaim
reclaim-containers = Stopped containers
reclaim-images = Unused images
reclaim-build-cache = Build cache
reclaim-volumes = Unused volumes
about-size = ≈ { $size }
reclaimed-title = Reclaimed { $size }
cannot-reclaim = Cannot reclaim space
engine-api = API { $version }
rootless = rootless
card-disk = Disk
card-cpu = CPU
card-memory = Memory
card-disk-io = Disk I/O
card-network = Network
card-of-total = of { $total } total
card-total = { $size } total
card-stored = { $size } stored
card-reclaimable = { $size } reclaimable
no-running-containers = No running containers
per-second = { $size }/s
disk-rates = Read { $read } · Write { $written }
network-rates = In { $received } · Out { $sent }
pulling-percent = Pulling { $reference }… { $percent }%
pull-downloading = Downloading
idle-rate = 0 KB/s
