use crate::L10N;
use crate::imap::Role;
use crate::store::{Account, Mailbox, MailboxId, Scope};
use crate::ui::symbols::Symbol;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    All(Role),
    Flagged,
    Mailbox(MailboxId),
}

impl Place {
    pub fn scope(&self) -> Scope {
        match self {
            Self::All(role) => Scope::AllOf(*role),
            Self::Flagged => Scope::Flagged,
            Self::Mailbox(id) => Scope::Mailbox(*id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Heading(String),
    Place {
        place: Place,
        title: String,
        symbol: Symbol,
        unread: usize,
        depth: usize,
    },
}

const UNIFIED: [Role; 6] = [
    Role::Inbox,
    Role::Drafts,
    Role::Sent,
    Role::Archive,
    Role::Junk,
    Role::Trash,
];

pub fn role_title(role: Role) -> String {
    L10N.tr(match role {
        Role::Inbox => "mailbox-inbox",
        Role::Drafts => "mailbox-drafts",
        Role::Sent => "mailbox-sent",
        Role::Archive => "mailbox-archive",
        Role::Junk => "mailbox-junk",
        Role::Trash => "mailbox-trash",
        Role::All => "mailbox-all",
        Role::Flagged => "mailbox-flagged",
    })
}

pub fn rows(accounts: &[(Account, Vec<Mailbox>)], unread: impl Fn(&Scope) -> usize) -> Vec<Row> {
    let mut rows = vec![Row::Heading(L10N.tr("sidebar-mailboxes"))];
    for role in UNIFIED {
        let present = accounts
            .iter()
            .any(|(_, mailboxes)| mailboxes.iter().any(|mailbox| mailbox.role == Some(role)));
        if present || role == Role::Inbox {
            let place = Place::All(role);
            let unread = match role {
                Role::Inbox | Role::Junk => unread(&place.scope()),
                _ => 0,
            };
            rows.push(Row::Place {
                place,
                title: role_title(role),
                symbol: Symbol::for_role(Some(role)),
                unread,
                depth: 0,
            });
        }
        if role == Role::Inbox {
            rows.push(Row::Place {
                place: Place::Flagged,
                title: role_title(Role::Flagged),
                symbol: Symbol::Flag,
                unread: 0,
                depth: 0,
            });
        }
    }
    for (account, mailboxes) in accounts {
        rows.push(Row::Heading(account.email.clone()));
        let mut ordered = mailboxes.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|mailbox| {
            (
                mailbox.role.map_or(UNIFIED.len(), |role| {
                    UNIFIED
                        .iter()
                        .position(|known| *known == role)
                        .unwrap_or(UNIFIED.len())
                }),
                mailbox.path.to_lowercase(),
            )
        });
        for mailbox in ordered {
            let title = match (mailbox.role, mailbox.depth()) {
                (Some(role), 0) if role != Role::All => role_title(role),
                _ => mailbox.name.clone(),
            };
            rows.push(Row::Place {
                place: Place::Mailbox(mailbox.id),
                title,
                symbol: Symbol::for_role(mailbox.role),
                unread: match mailbox.role {
                    Some(Role::Sent | Role::Drafts | Role::Trash | Role::Archive) => 0,
                    _ => mailbox.unread as usize,
                },
                depth: mailbox.depth().min(4),
            });
        }
    }
    rows
}
