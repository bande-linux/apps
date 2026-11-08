use std::io::{BufRead, BufReader, Write};
use std::net::Ipv4Addr;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Error, Result};

const ANSWER_TIME: Duration = Duration::from_secs(2);
const SYNC_ID: u64 = 1_907_001;

fn send(stream: &mut UnixStream, command: &str, arguments: Value) -> Result<()> {
    let mut request = serde_json::to_vec(&json!({"execute": command, "arguments": arguments}))?;
    request.push(b'\n');
    stream.write_all(&request)?;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct GuestNetwork {
    pub address: Option<Ipv4Addr>,
    pub received: u64,
    pub sent: u64,
}

pub fn guest_address(agent_socket: &Path) -> Result<Option<Ipv4Addr>> {
    guest_network(agent_socket).map(|network| network.address)
}

pub fn guest_network(agent_socket: &Path) -> Result<GuestNetwork> {
    let mut stream = UnixStream::connect(agent_socket)?;
    stream.set_read_timeout(Some(ANSWER_TIME))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    send(&mut stream, "guest-sync", json!({"id": SYNC_ID}))?;
    let mut line = Vec::new();
    loop {
        line.clear();
        reader.read_until(b'\n', &mut line)?;
        if line.is_empty() {
            return Err(Error::Protocol("the guest agent closed".into()));
        }
        let reply: Value = match serde_json::from_slice(&line) {
            Ok(reply) => reply,
            Err(_) => continue,
        };
        if reply["return"] == json!(SYNC_ID) {
            break;
        }
    }
    send(&mut stream, "guest-network-get-interfaces", json!({}))?;
    line.clear();
    reader.read_until(b'\n', &mut line)?;
    let reply: Value = serde_json::from_slice(&line)?;
    let interfaces = &reply["return"];
    let total = |key: &str| {
        interfaces
            .as_array()
            .into_iter()
            .flatten()
            .filter(|interface| interface["name"] != "lo")
            .filter_map(|interface| interface["statistics"][key].as_u64())
            .sum()
    };
    Ok(GuestNetwork {
        address: first_address(interfaces),
        received: total("rx-bytes"),
        sent: total("tx-bytes"),
    })
}

fn first_address(interfaces: &Value) -> Option<Ipv4Addr> {
    interfaces
        .as_array()?
        .iter()
        .filter_map(|interface| interface["ip-addresses"].as_array())
        .flatten()
        .filter(|address| address["ip-address-type"] == "ipv4")
        .filter_map(|address| address["ip-address"].as_str()?.parse::<Ipv4Addr>().ok())
        .find(|address| !address.is_loopback())
}
