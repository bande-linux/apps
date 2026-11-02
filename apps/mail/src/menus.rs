use bande_app_menu::{Entry, edit_submenu};
use bande_mail::L10N;
use bande_mail::settings::SortBy;
use bande_mail::ui::{MailView, SHORTCUTS};

fn item(message: &str, action: &str) -> Entry {
    let entry = Entry::item(&L10N.tr(message), action);
    match SHORTCUTS.iter().find(|(known, _)| *known == action) {
        Some((_, shortcut)) => entry.shortcut(shortcut),
        None => entry,
    }
}

pub const VIEWER: &str = "viewer";

pub fn window_menu() -> Vec<Entry> {
    vec![Entry::item(&L10N.tr("menu-viewer"), VIEWER)]
}

pub fn app_menus(view: &MailView, viewer: bool, windowed: bool) -> Vec<Entry> {
    let targets = view
        .move_targets()
        .into_iter()
        .map(|(name, id)| Entry::item(&name, &format!("move-to:{id}")))
        .collect::<Vec<_>>();
    let selection = |message: &str, action: &str| item(message, action).enabled(viewer);
    vec![
        Entry::submenu(
            &L10N.tr("menu-file"),
            vec![
                item("action-compose", "compose"),
                Entry::separator(),
                item("menu-add-account", "add-account"),
                Entry::separator(),
                selection("menu-find", "find"),
            ],
        ),
        edit_submenu(Vec::new()),
        Entry::submenu(&L10N.tr("menu-view"), {
            let settings = view.settings();
            let mut entries = SortBy::ALL
                .into_iter()
                .map(|sort| {
                    Entry::item(
                        &L10N.tr(&format!("menu-sort-{}", sort.name())),
                        &format!("sort-{}", sort.name()),
                    )
                    .radio(settings.sort == sort)
                })
                .collect::<Vec<_>>();
            entries.push(Entry::separator());
            entries.push(
                Entry::item(&L10N.tr("menu-sort-descending"), "sort-descending")
                    .radio(!settings.ascending),
            );
            entries.push(
                Entry::item(&L10N.tr("menu-sort-ascending"), "sort-ascending")
                    .radio(settings.ascending),
            );
            entries.push(Entry::separator());
            entries.push(
                Entry::item(&L10N.tr("menu-bcc-field"), "bcc-field").radio(settings.bcc_field),
            );
            entries
        }),
        Entry::submenu(
            &L10N.tr("menu-mailbox"),
            vec![
                item("action-get-mail", "get-mail"),
                Entry::separator(),
                selection("menu-mark-all-read", "mark-all-read"),
                Entry::separator(),
                item("menu-new-mailbox", "new-mailbox"),
            ],
        ),
        Entry::submenu(
            &L10N.tr("menu-message"),
            vec![
                item("compose-send", "send").enabled(windowed),
                Entry::separator(),
                selection("action-reply", "reply"),
                selection("action-reply-all", "reply-all"),
                selection("action-forward", "forward"),
                Entry::separator(),
                selection("menu-toggle-read", "toggle-read"),
                selection("menu-toggle-flag", "toggle-flag"),
                Entry::separator(),
                selection("action-archive", "archive"),
                selection("action-junk", "junk"),
                Entry::submenu(&L10N.tr("menu-move-to"), targets).enabled(viewer),
                selection("action-delete", "delete"),
            ],
        ),
    ]
}
