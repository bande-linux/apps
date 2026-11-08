use bande_app_menu::{Entry, edit_submenu};
use bande_l10n::{Text, text, tr};
use bande_mail::settings::SortBy;
use bande_mail::ui::{MailView, SHORTCUTS};

fn item(message: Text, action: &str) -> Entry {
    let entry = Entry::item(&message.tr(), action);
    match SHORTCUTS.iter().find(|(known, _)| *known == action) {
        Some((_, shortcut)) => entry.shortcut(shortcut),
        None => entry,
    }
}

pub const VIEWER: &str = "viewer";

pub fn window_menu() -> Vec<Entry> {
    vec![Entry::item(&tr!("Message Viewer"), VIEWER)]
}

pub fn app_menus(view: &MailView, viewer: bool, windowed: bool) -> Vec<Entry> {
    let targets = view
        .move_targets()
        .into_iter()
        .map(|(name, id)| Entry::item(&name, &format!("move-to:{id}")))
        .collect::<Vec<_>>();
    let selection = |message: Text, action: &str| item(message, action).enabled(viewer);
    vec![
        Entry::submenu(
            &tr!("File"),
            vec![
                item(text!("New Message"), "compose"),
                Entry::separator(),
                item(text!("Add Account…"), "add-account"),
                Entry::separator(),
                selection(text!("Find"), "find"),
                Entry::separator(),
                bande_app_menu::print_item(viewer),
            ],
        ),
        edit_submenu(Vec::new()),
        Entry::submenu(&tr!("View"), {
            let settings = view.settings();
            let mut entries = SortBy::ALL
                .into_iter()
                .map(|sort| {
                    Entry::item(&sort_title(sort.name()), &format!("sort-{}", sort.name()))
                        .radio(settings.sort == sort)
                })
                .collect::<Vec<_>>();
            entries.push(Entry::separator());
            entries.push(
                Entry::item(&tr!("Descending"), "sort-descending").radio(!settings.ascending),
            );
            entries
                .push(Entry::item(&tr!("Ascending"), "sort-ascending").radio(settings.ascending));
            entries.push(Entry::separator());
            entries.push(
                Entry::item(&tr!("Bcc Address Field"), "bcc-field").radio(settings.bcc_field),
            );
            entries
        }),
        Entry::submenu(
            &tr!("Mailbox"),
            vec![
                item(text!("Get Mail"), "get-mail"),
                Entry::separator(),
                selection(text!("Mark All as Read"), "mark-all-read"),
                Entry::separator(),
                item(text!("New Mailbox…"), "new-mailbox"),
            ],
        ),
        Entry::submenu(
            &tr!("Message"),
            vec![
                item(text!("Send"), "send").enabled(windowed),
                Entry::separator(),
                selection(text!("Reply"), "reply"),
                selection(text!("Reply All"), "reply-all"),
                selection(text!("Forward"), "forward"),
                Entry::separator(),
                selection(text!("Mark as Read or Unread"), "toggle-read"),
                selection(text!("Flag or Unflag"), "toggle-flag"),
                Entry::submenu(
                    &tr!("Flag"),
                    MailView::flag_colors()
                        .into_iter()
                        .map(|(name, action)| Entry::item(&name, &action))
                        .collect(),
                )
                .enabled(viewer),
                Entry::separator(),
                selection(text!("Apply Rules"), "apply-rules"),
                Entry::separator(),
                selection(text!("Archive"), "archive"),
                selection(text!("Move to Junk"), "junk"),
                Entry::submenu(&tr!("Move To"), targets).enabled(viewer),
                selection(text!("Delete"), "delete"),
            ],
        ),
    ]
}

fn sort_title(name: &str) -> String {
    match name {
        "date" => tr!("Sort by Date"),
        "from" => tr!("Sort by From"),
        "subject" => tr!("Sort by Subject"),
        "size" => tr!("Sort by Size"),
        "unread" => tr!("Sort by Unread"),
        other => other.to_owned(),
    }
}
