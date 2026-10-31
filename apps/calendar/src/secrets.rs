use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
const DEFAULT_COLLECTION: &str = "/org/freedesktop/secrets/aliases/default";
const APPLICATION: &str = "bande-calendar";

type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

pub struct Keyring {
    connection: Connection,
    session: OwnedObjectPath,
}

fn attributes(account: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", APPLICATION), ("account", account)])
}

fn no_prompt(path: &OwnedObjectPath) -> bool {
    path.as_str() == "/"
}

impl Keyring {
    pub fn open() -> zbus::Result<Self> {
        let connection = Connection::session()?;
        let service = Proxy::new(&connection, SERVICE, SERVICE_PATH, SERVICE_INTERFACE)?;
        let (_, session): (OwnedValue, OwnedObjectPath) =
            service.call("OpenSession", &("plain", Value::from("")))?;
        Ok(Self {
            connection,
            session,
        })
    }

    fn service(&self) -> zbus::Result<Proxy<'_>> {
        Proxy::new(&self.connection, SERVICE, SERVICE_PATH, SERVICE_INTERFACE)
    }

    fn prompt(&self, path: &OwnedObjectPath) -> zbus::Result<bool> {
        if no_prompt(path) {
            return Ok(true);
        }
        let prompt = Proxy::new(&self.connection, SERVICE, path.as_str(), PROMPT_INTERFACE)?;
        let mut completed = prompt.receive_signal("Completed")?;
        prompt.call_method("Prompt", &(""))?;
        match completed.next() {
            Some(message) => {
                let (dismissed, _): (bool, OwnedValue) = message.body().deserialize()?;
                Ok(!dismissed)
            }
            None => Ok(false),
        }
    }

    fn find(&self, account: &str) -> zbus::Result<Option<OwnedObjectPath>> {
        let (mut unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = self
            .service()?
            .call("SearchItems", &(attributes(account)))?;
        if unlocked.is_empty() && !locked.is_empty() {
            let (now_unlocked, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
                self.service()?.call("Unlock", &(locked.clone()))?;
            unlocked = now_unlocked;
            if unlocked.is_empty() && self.prompt(&prompt)? {
                unlocked = locked;
            }
        }
        Ok(unlocked.into_iter().next())
    }

    pub fn password(&self, account: &str) -> zbus::Result<Option<String>> {
        let Some(item) = self.find(account)? else {
            return Ok(None);
        };
        let secrets: HashMap<OwnedObjectPath, Secret> = self
            .service()?
            .call("GetSecrets", &(vec![item.clone()], &self.session))?;
        Ok(secrets
            .get(&item)
            .map(|(_, _, value, _)| String::from_utf8_lossy(value).into_owned()))
    }

    pub fn store(&self, account: &str, label: &str, password: &str) -> zbus::Result<()> {
        let collection = Proxy::new(
            &self.connection,
            SERVICE,
            DEFAULT_COLLECTION,
            COLLECTION_INTERFACE,
        )?;
        let properties: HashMap<&str, Value> = HashMap::from([
            ("org.freedesktop.Secret.Item.Label", Value::from(label)),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::from(attributes(account)),
            ),
        ]);
        let secret = (
            &self.session,
            Vec::<u8>::new(),
            password.as_bytes().to_vec(),
            "text/plain; charset=utf8",
        );
        let (_, prompt): (OwnedObjectPath, OwnedObjectPath) =
            collection.call("CreateItem", &(properties, secret, true))?;
        match self.prompt(&prompt)? {
            true => Ok(()),
            false => Err(zbus::Error::Failure("the keyring stayed locked".to_owned())),
        }
    }

    pub fn forget(&self, account: &str) -> zbus::Result<()> {
        if let Some(item) = self.find(account)? {
            let item = Proxy::new(&self.connection, SERVICE, item.as_str(), ITEM_INTERFACE)?;
            let prompt: OwnedObjectPath = item.call("Delete", &())?;
            self.prompt(&prompt)?;
        }
        Ok(())
    }
}
