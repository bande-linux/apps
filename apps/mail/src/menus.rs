use bande_mail::L10N;
use bande_mail::ui::MailView;
use unbound_app_menu::{Entry, app_submenu, close_window, window_submenu};

fn item(message: &str, action: &str, shortcut: &str) -> Entry {
    let entry = Entry::item(&L10N.tr(message), action);
    match shortcut.is_empty() {
        true => entry,
        false => entry.shortcut(shortcut),
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
                item("action-compose", "compose", "Super+N"),
                item("action-get-mail", "get-mail", "Shift+Super+N"),
                Entry::separator(),
                item("menu-add-account", "add-account", ""),
                Entry::submenu(&L10N.tr("menu-remove-account"), accounts),
                Entry::separator(),
                item("menu-find", "find", "Super+F"),
                Entry::separator(),
                close_window(),
            ],
        ),
        Entry::submenu(
            &L10N.tr("menu-message"),
            vec![
                item("compose-send", "send", "Shift+Super+D"),
                Entry::separator(),
                item("action-reply", "reply", "Super+R"),
                item("action-reply-all", "reply-all", "Shift+Super+R"),
                item("action-forward", "forward", "Shift+Super+F"),
                Entry::separator(),
                item("menu-toggle-read", "toggle-read", "Shift+Super+U"),
                item("menu-toggle-flag", "toggle-flag", "Shift+Super+L"),
                Entry::separator(),
                item("action-archive", "archive", "Shift+Super+A"),
                item("action-junk", "junk", "Shift+Super+J"),
                item("action-delete", "delete", "Super+Backspace"),
            ],
        ),
        window_submenu(Vec::new()),
    ]
}
