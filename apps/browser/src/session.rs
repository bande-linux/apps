use web_net::Url;

use crate::view::SavedWindow;

const WINDOW: &str = "window";

pub(crate) fn write(windows: &[SavedWindow]) -> String {
    let mut text = String::new();
    for window in windows {
        text.push_str(&format!("{WINDOW} {}\n", window.active));
        for url in &window.tabs {
            text.push_str(url.as_str());
            text.push('\n');
        }
    }
    text
}

pub(crate) fn parse(text: &str) -> Vec<SavedWindow> {
    let mut windows: Vec<SavedWindow> = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(active) = line.strip_prefix(WINDOW) {
            windows.push(SavedWindow {
                tabs: Vec::new(),
                active: active.trim().parse().unwrap_or(0),
            });
            continue;
        }
        let Some(window) = windows.last_mut() else {
            continue;
        };
        if let Ok(url) = Url::parse(line)
            && matches!(url.scheme(), "http" | "https")
        {
            window.tabs.push(url);
        }
    }
    windows.retain(|window| !window.tabs.is_empty());
    for window in &mut windows {
        window.active = window.active.min(window.tabs.len() - 1);
    }
    windows
}
