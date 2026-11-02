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

pub fn app_menus(view: &MailView) -> Vec<Entry> {
    let targets = view
        .move_targets()
        .into_iter()
        .map(|(name, id)| Entry::item(&name, &format!("move-to:{id}")))
        .collect::<Vec<_>>();
    vec![
        Entry::submenu(
            &L10N.tr("menu-file"),
            vec![
                item("action-compose", "compose"),
                Entry::separator(),
                item("menu-add-account", "add-account"),
                Entry::separator(),
                item("menu-find", "find"),
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
                item("menu-mark-all-read", "mark-all-read"),
                Entry::separator(),
                item("menu-new-mailbox", "new-mailbox"),
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
                Entry::submenu(&L10N.tr("menu-move-to"), targets),
                item("action-delete", "delete"),
            ],
        ),
    ]
}
