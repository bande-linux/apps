pub mod compose;
pub mod discover;
pub mod dns;
pub mod donate;
pub mod error;
pub mod html;
pub mod http;
pub mod imap;
pub mod message;
pub mod net;
pub mod picture;
pub mod portal;
pub mod settings;
pub mod setup;
pub mod smtp;
pub mod store;
pub mod sync;
pub mod ui;
pub mod worker;

pub const KEYRING: &str = "org.bande.Mail";

pub fn keyring_key<'a>(email: &'a str, host: &'a str) -> [(&'a str, &'a str); 2] {
    [("account", email), ("server", host)]
}

pub fn keyring_label(email: &str) -> String {
    format!("Mail password for {email}")
}
