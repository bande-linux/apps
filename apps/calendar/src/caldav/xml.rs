use super::http::Error;

pub const DAV: &str = "DAV:";
pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
pub const CALSERVER: &str = "http://calendarserver.org/ns/";
pub const APPLE: &str = "http://apple.com/ns/ical/";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Element {
    pub namespace: String,
    pub name: String,
    pub text: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    fn from_node(node: roxmltree::Node) -> Self {
        let children: Vec<Self> = node
            .children()
            .filter(|child| child.is_element())
            .map(Self::from_node)
            .collect();
        let text = match children.is_empty() {
            true => node.text().unwrap_or_default().trim().to_owned(),
            false => String::new(),
        };
        Self {
            namespace: node.tag_name().namespace().unwrap_or_default().to_owned(),
            name: node.tag_name().name().to_owned(),
            text,
            attributes: node
                .attributes()
                .map(|attribute| (attribute.name().to_owned(), attribute.value().to_owned()))
                .collect(),
            children,
        }
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn is(&self, namespace: &str, name: &str) -> bool {
        self.namespace == namespace && self.name == name
    }

    pub fn child(&self, namespace: &str, name: &str) -> Option<&Self> {
        self.children.iter().find(|child| child.is(namespace, name))
    }

    pub fn find(&self, namespace: &str, name: &str) -> Option<&Self> {
        self.children
            .iter()
            .find_map(|child| match child.is(namespace, name) {
                true => Some(child),
                false => child.find(namespace, name),
            })
    }

    pub fn hrefs(&self) -> Vec<String> {
        let mut found = Vec::new();
        self.collect_hrefs(&mut found);
        found
    }

    fn collect_hrefs(&self, found: &mut Vec<String>) {
        for child in &self.children {
            match child.is(DAV, "href") {
                true => found.push(child.text.clone()),
                false => child.collect_hrefs(found),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    pub href: String,
    pub status: Option<u16>,
    pub props: Vec<Element>,
}

impl Entry {
    pub fn prop(&self, namespace: &str, name: &str) -> Option<&Element> {
        self.props.iter().find(|prop| prop.is(namespace, name))
    }

    pub fn text(&self, namespace: &str, name: &str) -> Option<&str> {
        self.prop(namespace, name)
            .map(|prop| prop.text.as_str())
            .filter(|text| !text.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Multistatus {
    pub entries: Vec<Entry>,
    pub sync_token: Option<String>,
}

fn status_code(text: &str) -> Option<u16> {
    text.split_whitespace().nth(1)?.parse().ok()
}

pub fn parse_multistatus(body: &str) -> Result<Multistatus, Error> {
    let document =
        roxmltree::Document::parse(body).map_err(|error| Error::Xml(error.to_string()))?;
    let root = Element::from_node(document.root_element());
    if !root.is(DAV, "multistatus") {
        return Err(Error::Xml(format!(
            "expected a multistatus, got {}",
            root.name
        )));
    }
    let mut status = Multistatus {
        sync_token: root
            .child(DAV, "sync-token")
            .map(|token| token.text.clone()),
        ..Multistatus::default()
    };
    for response in root
        .children
        .iter()
        .filter(|child| child.is(DAV, "response"))
    {
        let Some(href) = response.child(DAV, "href") else {
            continue;
        };
        let mut entry = Entry {
            href: href.text.clone(),
            status: response
                .child(DAV, "status")
                .and_then(|status| status_code(&status.text)),
            props: Vec::new(),
        };
        for propstat in response
            .children
            .iter()
            .filter(|child| child.is(DAV, "propstat"))
        {
            let fine = propstat
                .child(DAV, "status")
                .and_then(|status| status_code(&status.text))
                .is_none_or(|code| (200..300).contains(&code));
            if let (true, Some(prop)) = (fine, propstat.child(DAV, "prop")) {
                entry.props.extend(prop.children.iter().cloned());
            }
        }
        status.entries.push(entry);
    }
    Ok(status)
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const NAMESPACES: &str = "xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\" xmlns:cs=\"http://calendarserver.org/ns/\" xmlns:a=\"http://apple.com/ns/ical/\"";

pub fn propfind(props: &[&str]) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<d:propfind {NAMESPACES}><d:prop>{}</d:prop></d:propfind>",
        props
            .iter()
            .map(|prop| format!("<{prop}/>"))
            .collect::<String>()
    )
}

pub fn sync_collection(token: Option<&str>) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<d:sync-collection {NAMESPACES}><d:sync-token>{}</d:sync-token><d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>",
        escape(token.unwrap_or_default())
    )
}

pub fn multiget(hrefs: &[String]) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<c:calendar-multiget {NAMESPACES}><d:prop><d:getetag/><c:calendar-data/></d:prop>{}</c:calendar-multiget>",
        hrefs
            .iter()
            .map(|href| format!("<d:href>{}</d:href>", escape(href)))
            .collect::<String>()
    )
}

pub fn calendar_query_events() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<c:calendar-query {NAMESPACES}><d:prop><d:getetag/></d:prop><c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\"/></c:comp-filter></c:filter></c:calendar-query>"
    )
}

pub fn set_property(element: &str, value: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<d:propertyupdate {NAMESPACES}><d:set><d:prop><{element}>{}</{element}></d:prop></d:set></d:propertyupdate>",
        escape(value)
    )
}

pub fn mkcalendar(name: &str, color: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<c:mkcalendar {NAMESPACES}><d:set><d:prop><d:displayname>{}</d:displayname><a:calendar-color>{}</a:calendar-color><c:supported-calendar-component-set><c:comp name=\"VEVENT\"/></c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>",
        escape(name),
        escape(color)
    )
}
