use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::thread;

use bande_vm::{Qmp, RunState};
use serde_json::{Value, json};

fn fake_qemu(stream: UnixStream) -> std::io::Result<()> {
    let mut out = stream.try_clone()?;
    writeln!(
        out,
        "{}",
        json!({"QMP": {"version": {}, "capabilities": []}})
    )?;
    for line in BufReader::new(stream).lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let id = &request["id"];
        let job = &request["arguments"]["job-id"];
        let event = |name: &str, data: Value| json!({"event": name, "data": data});
        let replies = match request["execute"].as_str().unwrap_or_default() {
            "query-status" => vec![
                event("STOP", json!({})),
                json!({"return": {"status": "paused"}, "id": id}),
            ],
            "snapshot-save" => vec![
                event("JOB_STATUS_CHANGE", json!({"id": job, "status": "running"})),
                json!({"return": {}, "id": id}),
                event("RESUME", json!({})),
                event(
                    "JOB_STATUS_CHANGE",
                    json!({"id": job, "status": "concluded"}),
                ),
            ],
            "query-jobs" => vec![json!({"return": [{"id": "other", "error": "no"}], "id": id})],
            _ => vec![json!({"return": {}, "id": id})],
        };
        for reply in replies {
            writeln!(out, "{reply}")?;
        }
    }
    Ok(())
}

#[test]
fn events_around_replies_and_jobs_are_kept_in_order() -> Result<(), Box<dyn std::error::Error>> {
    let (client, server) = UnixStream::pair()?;
    let qemu = thread::spawn(move || fake_qemu(server));
    let mut qmp = Qmp::handshake(client)?;
    assert_eq!(qmp.status()?, RunState::Paused);
    qmp.snapshot_save("before-upgrade", "disk", &["disk", "vars"])?;
    let events: Vec<String> = qmp
        .drain_events()?
        .into_iter()
        .map(|event| event.name)
        .collect();
    assert_eq!(events, ["STOP", "RESUME"]);
    drop(qmp);
    qemu.join().map_err(|_| "fake QEMU panicked")??;
    Ok(())
}
