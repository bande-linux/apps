use bande_app_menu::{Entry, edit_submenu};
use bande_mail::L10N;
use bande_mail::ui::SHORTCUTS;

fn item(message: &str, action: &str) -> Entry {
    let entry = Entry::item(&L10N.tr(message), action);
    match SHORTCUTS.iter().find(|(known, _)| *known == action) {
        Some((_, shortcut)) => entry.shortcut(shortcut),
        None => entry,
    }
}

pub fn app_menus() -> Vec<Entry> {
    vec![
        Entry::submenu(
            &L10N.tr("menu-file"),
            vec![
                item("action-compose", "compose"),
                item("action-get-mail", "get-mail"),
                Entry::separator(),
                item("menu-add-account", "add-account"),
                Entry::separator(),
                item("menu-find", "find"),
            ],
        ),
        edit_submenu(Vec::new()),
        Entry::submenu(
            &L10N.tr("menu-message"),
            vec![
                item("compose-send", "send"),
                Entry::separator(),
                item("action-reply", "reply"),
                item("action-reply-all", "reply-all"),
                item("action-forward", "forward"),
                Entry::separator(),
                item("menu-toggle-read", "toggle-read"),
                item("menu-toggle-flag", "toggle-flag"),
                Entry::separator(),
                item("action-archive", "archive"),
                item("action-junk", "junk"),
                item("action-delete", "delete"),
            ],
        ),
    ]
}
