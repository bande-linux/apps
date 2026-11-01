title = Package Installer
reading = Reading the package…
invalid = This file isn't a valid Debian package.
invalid-hint = The download may be incomplete. Try downloading it again.
wrong-architecture = “{ $name }” is built for { $arch }. This computer needs a package for { $native }.
wrong-architecture-hint = Look for a { $native } download on the developer's website.
arch-amd64 = Intel and AMD computers (amd64)
arch-arm64 = 64-bit ARM (arm64)
arch-armhf = 32-bit ARM (armhf)
arch-i386 = older 32-bit Intel and AMD computers (i386)
version-size = Version { $version } · { $size }
present = Version { $version } is installed.
new-install = Installs version { $version }
upgrade-from = Upgrades from { $from } to { $to }
downgrade-from = Downgrades from { $from } to { $to }
downgrade-caution = This is older than the installed version { $from }.
also-installs = { $count ->
    [one] Also installs 1 package
   *[other] Also installs { $count } packages
}
removes = { $count ->
    [one] Removes 1 package: { $names }
   *[other] Removes { $count } packages: { $names }
}
trust = This package doesn't come from your software sources. Its install scripts get full control of this computer, and it may add its own software source. Install it only if you trust where you got it.
from-maintainer = From { $maintainer }
blocked = Can't install: it needs packages that aren't available
needs = Needs { $names }
details = Details
packages = Packages
status-waiting = Waiting for another installation to finish…
status-downloading = Downloading…
status-installing = Installing…
status-removing = Removing…
status-working = Working…
install = Install
install-and-remove = Install and Remove
cancel = Cancel
close = Close
done = Done
installed = “{ $name }” is installed.
restart-to-finish = Restart the computer to finish installing.
open-app = Open { $app }
remove = Remove…
remove-confirm = Remove “{ $name }”?
remove-also = { $count ->
    [one] This also removes 1 package: { $names }
   *[other] This also removes { $count } packages: { $names }
}
remove-button = Remove
removed = “{ $name }” was removed.
failed-locked = Another installation is running. Try again when it finishes.
failed-system = This package is part of the system and can't be removed.
failed-unavailable = Installing packages needs PackageKit, which isn't installed.
failed-other = The package couldn't be installed.
