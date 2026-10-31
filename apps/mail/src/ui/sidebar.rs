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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Security, Server};

    fn account(id: i64, email: &str) -> Account {
        let server = Server {
            host: "h".to_owned(),
            port: 1,
            security: Security::Tls,
        };
        Account {
            id,
            name: String::new(),
            email: email.to_owned(),
            user: email.to_owned(),
            imap: server.clone(),
            smtp: server,
        }
    }

    fn mailbox(id: i64, path: &str, role: Option<Role>, unread: u32) -> Mailbox {
        Mailbox {
            id,
            account: 1,
            path: path.to_owned(),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            delimiter: Some('/'),
            role,
            selectable: true,
            unread,
        }
    }

    #[test]
    fn unified_mailboxes_come_first_then_each_account_in_role_order() {
        let accounts = vec![
            (
                account(1, "ann@example.com"),
                vec![
                    mailbox(1, "Projects", None, 2),
                    mailbox(2, "INBOX", Some(Role::Inbox), 3),
                    mailbox(3, "Projects/Old", None, 0),
                    mailbox(4, "Gesendet", Some(Role::Sent), 1),
                ],
            ),
            (
                account(2, "bob@example.com"),
                vec![mailbox(5, "INBOX", Some(Role::Inbox), 0)],
            ),
        ];
        let shown = rows(&accounts, |scope| match scope {
            Scope::AllOf(Role::Inbox) => 3,
            _ => 0,
        })
        .into_iter()
        .map(|row| match row {
            Row::Heading(title) => format!("# {title}"),
            Row::Place {
                title,
                unread,
                depth,
                ..
            } => format!("{}{title} {unread}", "  ".repeat(depth)),
        })
        .collect::<Vec<_>>();
        assert_eq!(
            shown,
            [
                "# Mailboxes",
                "Inbox 3",
                "Flagged 0",
                "Sent 0",
                "# ann@example.com",
                "Inbox 3",
                "Sent 0",
                "Projects 2",
                "  Old 0",
                "# bob@example.com",
                "Inbox 0",
            ]
        );
    }
}
