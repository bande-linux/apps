use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::thread;

use bande_search::{Donation, IndexProxyBlocking};
use zbus::blocking::Connection;

use crate::store::Calendar;

pub const DOMAIN: &str = "org.bande.Calendar";
pub const EVENT_ARGUMENT: &str = "event:";

struct Offer {
    fingerprint: String,
    items: Vec<Donation>,
}

fn take_offer(offer: Offer) -> zbus::Result<()> {
    let session = Connection::session()?;
    let index = IndexProxyBlocking::new(&session)?;
    if index.cursor(DOMAIN)? != offer.fingerprint {
        index.forget_all(DOMAIN)?;
        index.donate(DOMAIN, offer.items, &offer.fingerprint)?;
    }
    Ok(())
}

fn offers() -> Option<&'static Sender<Offer>> {
    static OFFERS: OnceLock<Option<Sender<Offer>>> = OnceLock::new();
    OFFERS
        .get_or_init(|| {
            let (sender, received) = channel::<Offer>();
            thread::Builder::new()
                .name("search-offers".to_owned())
                .spawn(move || {
                    while let Ok(mut offer) = received.recv() {
                        while let Ok(newer) = received.try_recv() {
                            offer = newer;
                        }
                        if let Err(error) = take_offer(offer) {
                            tracing::debug!("search has not taken the events: {error}");
                        }
                    }
                })
                .ok()
                .map(|_| sender)
        })
        .as_ref()
}

pub fn offer_to_search(calendars: &[Calendar]) {
    let mut hasher = DefaultHasher::new();
    let mut items = Vec::new();
    for calendar in calendars {
        for stored in &calendar.objects {
            stored.data.hash(&mut hasher);
            let Some(event) = stored
                .parsed
                .as_ref()
                .and_then(|object| object.events.first())
            else {
                continue;
            };
            items.push(Donation {
                id: event.uid.clone(),
                kind: "event".to_owned(),
                title: event.summary.clone(),
                detail: format!("{} · {}", event.start.date(), calendar.info.name),
                target: format!("{EVENT_ARGUMENT}{}", event.uid),
                body: format!("{}\n{}", event.location, event.description),
            });
        }
    }
    if let Some(offers) = offers() {
        let _ = offers.send(Offer {
            fingerprint: format!("{:016x}", hasher.finish()),
            items,
        });
    }
}
