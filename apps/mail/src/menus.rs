use bande_app_menu::{Entry, app_submenu, close_window, window_submenu};
use bande_mail::L10N;
use bande_mail::ui::{MailView, SHORTCUTS};

fn item(message: &str, action: &str) -> Entry {
    let entry = Entry::item(&L10N.tr(message), action);
    match SHORTCUTS.iter().find(|(known, _)| *known == action) {
        Some((_, shortcut)) => entry.shortcut(shortcut),
        None => entry,
    }
}

pub fn app_menus(view: &MailView) -> Vec<Entry> {
    let accounts = view
        .accounts()
        .into_iter()
        .map(|account| Entry::item(&account.email, &format!("remove-account:{}", account.id)))
        .collect::<Vec<_>>();
    vec![
        app_submenu(&L10N.tr("app-name"), Vec::new()),
        Entry::submenu(
            &L10N.tr("menu-file"),
            vec![
                item("action-compose", "compose"),
                item("action-get-mail", "get-mail"),
                Entry::separator(),
                item("menu-add-account", "add-account"),
                Entry::submenu(&L10N.tr("menu-remove-account"), accounts),
                Entry::separator(),
                item("menu-find", "find"),
                Entry::separator(),
                close_window(),
            ],
        ),
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
        window_submenu(Vec::new()),
    ]
}
