#![windows_subsystem = "windows"]

use seraphine_winui3_spike::stress;

use std::io::{self, BufReader, prelude::*};
use std::time::Duration;

use interprocess::local_socket::{GenericNamespaced, ListenerOptions, prelude::*};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Command {
    duration_ms: u64,
    event_rate: u64,
}

#[derive(Debug, Serialize)]
struct Response {
    mode: &'static str,
    pid: u32,
    events: u64,
    parsed: u64,
    errors: u64,
    elapsed_ms: u128,
}

fn run() -> io::Result<()> {
    let pipe_name = std::env::args()
        .nth(1)
        .expect("usage: seraphine-winui3-sidecar <pipe-name>");
    let name = pipe_name.to_ns_name::<GenericNamespaced>()?;
    let listener = ListenerOptions::new().name(name).create_sync()?;
    let mut connection = BufReader::new(listener.incoming().next().unwrap()?);
    let mut line = String::new();
    connection.read_line(&mut line)?;
    let command: Command = serde_json::from_str(&line)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let result = stress::run_stress(
        Duration::from_millis(command.duration_ms),
        command.event_rate,
    );
    let response = Response {
        mode: "sidecar",
        pid: std::process::id(),
        events: result.events,
        parsed: result.parsed,
        errors: result.errors,
        elapsed_ms: result.elapsed_ms,
    };

    let mut payload = serde_json::to_vec(&response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    payload.push(b'\n');
    connection.get_mut().write_all(&payload)?;
    connection.get_mut().flush()
}

fn main() {
    if let Err(error) = run() {
        eprintln!("sidecar failed: {error}");
        std::process::exit(1);
    }
}
