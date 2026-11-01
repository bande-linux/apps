use std::collections::HashSet;

const X11_LIBRARIES: [&str; 6] = [
    "libx11-6",
    "libxt6",
    "libxt6t64",
    "libxaw7",
    "libxm4",
    "libmotif",
];
const WAYLAND_TOOLKITS: [&str; 7] = [
    "libgtk-3-",
    "libgtk-4-",
    "libqt5gui5",
    "libqt6gui6",
    "libsdl2-2.0",
    "libsdl3-",
    "libwayland-client",
];

fn depends(field: &str) -> impl Iterator<Item = &str> {
    field.split([',', '|']).filter_map(|item| {
        let name = item.split_whitespace().next()?;
        Some(name.split(':').next().unwrap_or(name))
    })
}

fn x11_only<'a>(mut names: impl Iterator<Item = &'a str>) -> bool {
    let names: Vec<_> = names.by_ref().collect();
    names.iter().any(|name| X11_LIBRARIES.contains(name))
        && !names.iter().any(|name| {
            WAYLAND_TOOLKITS
                .iter()
                .any(|toolkit| name.starts_with(toolkit))
        })
}

pub fn x11_only_packages(index: &str) -> HashSet<String> {
    index
        .split("\n\n")
        .filter_map(|stanza| {
            let mut package = None;
            let mut fields = Vec::new();
            for line in stanza.lines() {
                if let Some(name) = line.strip_prefix("Package: ") {
                    package = Some(name.trim());
                } else if let Some(field) = line
                    .strip_prefix("Depends: ")
                    .or_else(|| line.strip_prefix("Pre-Depends: "))
                {
                    fields.push(field);
                }
            }
            let package = package?;
            x11_only(fields.into_iter().flat_map(depends)).then(|| package.to_owned())
        })
        .collect()
}

pub fn flatpak_needs_x11(metadata: &str) -> bool {
    let mut in_context = false;
    for line in metadata.lines().map(str::trim) {
        if line.starts_with('[') {
            in_context = line == "[Context]";
        } else if in_context && let Some(sockets) = line.strip_prefix("sockets=") {
            let sockets: Vec<_> = sockets.split(';').map(str::trim).collect();
            return sockets.contains(&"x11")
                && !sockets.contains(&"wayland")
                && !sockets.contains(&"fallback-x11");
        }
    }
    false
}
