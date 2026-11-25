use seraphine_protocol::{decode, decode_length_prefix, encode, Message, MessageKind};
use serde_json::json;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Sidecar {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl Sidecar {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_seraphine-sidecar"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: &Message) {
        self.stdin.write_all(&encode(message).unwrap()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn receive(&mut self) -> Message {
        let mut prefix = [0_u8; 4];
        self.stdout.read_exact(&mut prefix).unwrap();
        let length = decode_length_prefix(&prefix).unwrap().unwrap();
        let mut payload = vec![0_u8; length];
        self.stdout.read_exact(&mut payload).unwrap();
        decode(&payload).unwrap()
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn stdio_handshake_health_ping_and_shutdown() {
    let mut sidecar = Sidecar::start();
    let hello = sidecar.receive();
    assert_eq!(hello.kind, MessageKind::Hello);
    assert_eq!(hello.version, 1);
    assert!(hello.capabilities.unwrap().contains(&"health".to_string()));

    sidecar.send(&Message::request(1, 0, "health", json!({}), Some(1000)));
    let health = sidecar.receive();
    assert_eq!(health.kind, MessageKind::Response);
    assert_eq!(health.id, Some(1));
    let health_data = health.data.unwrap();
    assert_eq!(health_data["protocol"], 1);
    let working_set = health_data["memory"]["workingSetBytes"].as_u64().unwrap();
    let private_usage = health_data["memory"]["privateUsageBytes"].as_u64().unwrap();
    assert!(working_set < 10 * 1024 * 1024, "working set: {working_set}");
    assert!(
        private_usage < 10 * 1024 * 1024,
        "private usage: {private_usage}"
    );
    println!("sidecar memory: working_set={working_set} private_usage={private_usage}");

    sidecar.send(&Message::ping(42));
    let pong = sidecar.receive();
    assert_eq!(pong.kind, MessageKind::Pong);
    assert_eq!(pong.nonce, Some(42));

    sidecar.send(&Message::shutdown());
    sidecar.child.wait().unwrap();
    assert!(sidecar.child.try_wait().unwrap().is_some());
}
