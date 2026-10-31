#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bande_mail::secrets::Keyring;
use unbound_test_bus::SessionBus;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

#[derive(Clone)]
struct Stored {
    path: OwnedObjectPath,
    attributes: HashMap<String, String>,
    value: Vec<u8>,
    locked: bool,
}

#[derive(Clone, Default)]
struct Shelf(Arc<Mutex<Vec<Stored>>>);

const COLLECTION: &str = "/org/freedesktop/secrets/collection/login";

fn path(text: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(text.to_owned()).unwrap()
}

struct Service(Shelf);

#[zbus::interface(name = "org.freedesktop.Secret.Service")]
impl Service {
    fn open_session(
        &self,
        algorithm: &str,
        _input: OwnedValue,
    ) -> zbus::fdo::Result<(OwnedValue, OwnedObjectPath)> {
        if algorithm != "plain" {
            return Err(zbus::fdo::Error::NotSupported(algorithm.to_owned()));
        }
        Ok((
            OwnedValue::try_from(Value::from("")).unwrap(),
            path("/org/freedesktop/secrets/session/1"),
        ))
    }

    fn search_items(
        &self,
        attributes: HashMap<String, String>,
    ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
        let shelf = self.0.0.lock().unwrap();
        let matching = shelf.iter().filter(|item| {
            attributes
                .iter()
                .all(|(key, value)| item.attributes.get(key) == Some(value))
        });
        let (locked, unlocked): (Vec<_>, Vec<_>) = matching.partition(|item| item.locked);
        (
            unlocked.into_iter().map(|item| item.path.clone()).collect(),
            locked.into_iter().map(|item| item.path.clone()).collect(),
        )
    }

    fn unlock(&self, objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
        for item in self.0.0.lock().unwrap().iter_mut() {
            if objects.contains(&item.path) {
                item.locked = false;
            }
        }
        (objects, path("/"))
    }

    fn get_secrets(
        &self,
        items: Vec<OwnedObjectPath>,
        session: OwnedObjectPath,
    ) -> HashMap<OwnedObjectPath, Secret> {
        self.0
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|item| items.contains(&item.path) && !item.locked)
            .map(|item| {
                (
                    item.path.clone(),
                    (
                        session.clone(),
                        Vec::new(),
                        item.value.clone(),
                        "text/plain".to_owned(),
                    ),
                )
            })
            .collect()
    }

    fn read_alias(&self, name: &str) -> OwnedObjectPath {
        match name {
            "default" => path(COLLECTION),
            _ => path("/"),
        }
    }
}

struct Collection(Shelf);

#[zbus::interface(name = "org.freedesktop.Secret.Collection")]
impl Collection {
    async fn create_item(
        &self,
        properties: HashMap<String, OwnedValue>,
        secret: Secret,
        replace: bool,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        let attributes = properties
            .get("org.freedesktop.Secret.Item.Attributes")
            .and_then(|value| HashMap::<String, String>::try_from(value.try_clone().ok()?).ok())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("no attributes".to_owned()))?;
        let item_path = {
            let mut shelf = self.0.0.lock().unwrap();
            if replace {
                if let Some(existing) = shelf.iter_mut().find(|item| item.attributes == attributes)
                {
                    existing.value = secret.2;
                    return Ok((existing.path.clone(), path("/")));
                }
            }
            let item_path = path(&format!("{COLLECTION}/{}", shelf.len() + 1));
            shelf.push(Stored {
                path: item_path.clone(),
                attributes,
                value: secret.2,
                locked: false,
            });
            item_path
        };
        server
            .at(item_path.clone(), Item(self.0.clone(), item_path.clone()))
            .await?;
        Ok((item_path, path("/")))
    }
}

struct Item(Shelf, OwnedObjectPath);

#[zbus::interface(name = "org.freedesktop.Secret.Item")]
impl Item {
    fn delete(&self) -> OwnedObjectPath {
        self.0.0.lock().unwrap().retain(|item| item.path != self.1);
        path("/")
    }
}

fn keyring() -> (SessionBus, zbus::blocking::Connection, Shelf) {
    let bus = SessionBus::start().unwrap();
    let shelf = Shelf::default();
    let service = zbus::blocking::connection::Builder::address(bus.address())
        .unwrap()
        .name("org.freedesktop.secrets")
        .unwrap()
        .serve_at("/org/freedesktop/secrets", Service(shelf.clone()))
        .unwrap()
        .serve_at(COLLECTION, Collection(shelf.clone()))
        .unwrap()
        .build()
        .unwrap();
    (bus, service, shelf)
}

#[test]
fn passwords_are_saved_found_replaced_and_forgotten_per_account() {
    let (bus, _service, shelf) = keyring();
    let keyring = Keyring::open(bus.connect().unwrap()).unwrap();
    assert_eq!(
        keyring
            .password("ann@example.com", "imap.example.com")
            .unwrap(),
        None
    );

    keyring
        .save("ann@example.com", "imap.example.com", "first")
        .unwrap();
    keyring
        .save("bob@example.com", "imap.example.com", "bobs")
        .unwrap();
    keyring
        .save("ann@example.com", "imap.example.com", "sëcond")
        .unwrap();
    assert_eq!(shelf.0.lock().unwrap().len(), 2);
    let stored = shelf.0.lock().unwrap()[0].attributes.clone();
    assert_eq!(
        stored.get("application").map(String::as_str),
        Some("org.bande.Mail")
    );
    assert_eq!(
        keyring
            .password("ann@example.com", "imap.example.com")
            .unwrap()
            .as_deref(),
        Some("sëcond")
    );

    shelf.0.lock().unwrap()[1].locked = true;
    assert_eq!(
        keyring
            .password("bob@example.com", "imap.example.com")
            .unwrap()
            .as_deref(),
        Some("bobs")
    );

    keyring
        .forget("ann@example.com", "imap.example.com")
        .unwrap();
    assert_eq!(
        keyring
            .password("ann@example.com", "imap.example.com")
            .unwrap(),
        None
    );
    assert_eq!(
        keyring
            .password("bob@example.com", "imap.example.com")
            .unwrap()
            .as_deref(),
        Some("bobs")
    );
}
