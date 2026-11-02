use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Error, Result};

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
const NO_PROMPT: &str = "/";

type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

pub type Key<'a> = [(&'a str, &'a str)];

pub struct Keyring {
    connection: Connection,
    session: OwnedObjectPath,
    application: &'static str,
}

impl Keyring {
    pub fn open(application: &'static str) -> Result<Self> {
        let connection = Connection::session()?;
        let (_, session): (OwnedValue, OwnedObjectPath) = connection
            .call_method(
                Some(SERVICE),
                SERVICE_PATH,
                Some(SERVICE_INTERFACE),
                "OpenSession",
                &("plain", Value::from("")),
            )?
            .body()
            .deserialize()?;
        Ok(Self {
            connection,
            session,
            application,
        })
    }

    fn attributes<'a>(&self, key: &Key<'a>) -> HashMap<&'a str, &'a str> {
        key.iter()
            .copied()
            .chain([("application", self.application)])
            .collect()
    }

    fn service<B>(&self, method: &str, body: &B) -> Result<zbus::message::Message>
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.connection.call_method(
            Some(SERVICE),
            SERVICE_PATH,
            Some(SERVICE_INTERFACE),
            method,
            body,
        )
    }

    fn prompt(&self, prompt: &ObjectPath<'_>) -> Result<bool> {
        if prompt.as_str() == NO_PROMPT {
            return Ok(true);
        }
        let proxy = Proxy::new(&self.connection, SERVICE, prompt, PROMPT_INTERFACE)?;
        let mut completed = proxy.receive_signal("Completed")?;
        proxy.call_method("Prompt", &(""))?;
        let Some(signal) = completed.next() else {
            return Err(Error::Failure("the keyring prompt went away".to_owned()));
        };
        let (dismissed, _): (bool, OwnedValue) = signal.body().deserialize()?;
        Ok(!dismissed)
    }

    fn unlock(&self, paths: &[OwnedObjectPath]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
            self.service("Unlock", &(paths,))?.body().deserialize()?;
        match self.prompt(&prompt)? {
            true => Ok(()),
            false => Err(Error::Failure("the keyring stayed locked".to_owned())),
        }
    }

    fn items(&self, key: &Key<'_>) -> Result<Vec<OwnedObjectPath>> {
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = self
            .service("SearchItems", &(self.attributes(key),))?
            .body()
            .deserialize()?;
        self.unlock(&locked)?;
        Ok(unlocked.into_iter().chain(locked).collect())
    }

    pub fn password(&self, key: &Key<'_>) -> Result<Option<String>> {
        let items = self.items(key)?;
        if items.is_empty() {
            return Ok(None);
        }
        let secrets: HashMap<OwnedObjectPath, Secret> = self
            .service("GetSecrets", &(&items, &self.session))?
            .body()
            .deserialize()?;
        Ok(items
            .iter()
            .find_map(|item| secrets.get(item))
            .map(|(_, _, value, _)| String::from_utf8_lossy(value).into_owned()))
    }

    fn default_collection(&self) -> Result<OwnedObjectPath> {
        let (collection,): (OwnedObjectPath,) = self
            .service("ReadAlias", &("default",))?
            .body()
            .deserialize()?;
        if collection.as_str() != NO_PROMPT {
            return Ok(collection);
        }
        let properties = HashMap::from([(
            "org.freedesktop.Secret.Collection.Label",
            Value::from("Login"),
        )]);
        let (created, prompt): (OwnedObjectPath, OwnedObjectPath) = self
            .service("CreateCollection", &(properties, "default"))?
            .body()
            .deserialize()?;
        self.prompt(&prompt)?;
        match created.as_str() == NO_PROMPT {
            true => self.default_collection(),
            false => Ok(created),
        }
    }

    pub fn save(&self, key: &Key<'_>, label: &str, password: &str) -> Result<()> {
        let collection = self.default_collection()?;
        self.unlock(std::slice::from_ref(&collection))?;
        let properties = HashMap::from([
            ("org.freedesktop.Secret.Item.Label", Value::from(label)),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::from(self.attributes(key)),
            ),
        ]);
        let secret = (
            self.session.clone(),
            Vec::<u8>::new(),
            password.as_bytes().to_vec(),
            "text/plain",
        );
        let (_, prompt): (OwnedObjectPath, OwnedObjectPath) = self
            .connection
            .call_method(
                Some(SERVICE),
                &collection,
                Some(COLLECTION_INTERFACE),
                "CreateItem",
                &(properties, secret, true),
            )?
            .body()
            .deserialize()?;
        match self.prompt(&prompt)? {
            true => Ok(()),
            false => Err(Error::Failure(
                "saving the password was cancelled".to_owned(),
            )),
        }
    }

    pub fn forget(&self, key: &Key<'_>) -> Result<()> {
        for item in self.items(key)? {
            let (prompt,): (OwnedObjectPath,) = self
                .connection
                .call_method(Some(SERVICE), &item, Some(ITEM_INTERFACE), "Delete", &())?
                .body()
                .deserialize()?;
            self.prompt(&prompt)?;
        }
        Ok(())
    }
}
