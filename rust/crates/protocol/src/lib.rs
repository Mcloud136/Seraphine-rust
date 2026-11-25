use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_FRAME_SIZE: usize = 1024 * 1024;
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 512 * 1024;
pub const MAX_EVENT_BYTES: usize = 512 * 1024;
pub const MAX_IPC_INFLIGHT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_EVENT_QUEUE_BYTES: usize = 1024 * 1024;
pub const MAX_PENDING_REQUESTS: usize = 64;
pub const MAX_EVENT_QUEUE: usize = 128;
pub const LATENCY_BUCKETS_US: [u64; 8] = [
    1_000,
    5_000,
    15_000,
    50_000,
    150_000,
    500_000,
    1_500_000,
    u64::MAX,
];

pub const CAPABILITY_HEALTH: &str = "health";
pub const CAPABILITY_PROCESS_LIST: &str = "process.list";
pub const CAPABILITY_DIAGNOSTICS: &str = "diagnostics";
pub const CAPABILITY_LCU_READ: &str = "lcu.read";
pub const CAPABILITY_LCU_WRITE: &str = "lcu.write";
pub const CAPABILITY_AUTOMATION: &str = "automation";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Hello,
    Request,
    Response,
    Event,
    Cancel,
    Ping,
    Pong,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            source: None,
        }
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub version: u8,
    #[serde(rename = "type")]
    pub kind: MessageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
}

impl Message {
    fn new(kind: MessageKind) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            kind,
            id: None,
            epoch: None,
            seq: None,
            op: None,
            args: None,
            data: None,
            error: None,
            topic: None,
            deadline_ms: None,
            nonce: None,
            capabilities: None,
            app_version: None,
        }
    }

    pub fn hello(capabilities: Vec<String>, app_version: impl Into<String>) -> Self {
        Self {
            capabilities: Some(capabilities),
            app_version: Some(app_version.into()),
            ..Self::new(MessageKind::Hello)
        }
    }

    pub fn request(
        id: u64,
        epoch: u64,
        op: impl Into<String>,
        args: Value,
        deadline_ms: Option<u64>,
    ) -> Self {
        Self {
            id: Some(id),
            epoch: Some(epoch),
            op: Some(op.into()),
            args: Some(args),
            deadline_ms,
            ..Self::new(MessageKind::Request)
        }
    }

    pub fn response(id: u64, epoch: u64, data: Value) -> Self {
        Self {
            id: Some(id),
            epoch: Some(epoch),
            data: Some(data),
            ..Self::new(MessageKind::Response)
        }
    }

    pub fn response_error(id: u64, epoch: u64, error: RpcError) -> Self {
        Self {
            id: Some(id),
            epoch: Some(epoch),
            error: Some(error),
            ..Self::new(MessageKind::Response)
        }
    }

    pub fn event(epoch: u64, seq: u64, topic: impl Into<String>, data: Value) -> Self {
        Self {
            epoch: Some(epoch),
            seq: Some(seq),
            topic: Some(topic.into()),
            data: Some(data),
            ..Self::new(MessageKind::Event)
        }
    }

    pub fn cancel(id: u64) -> Self {
        Self {
            id: Some(id),
            ..Self::new(MessageKind::Cancel)
        }
    }

    pub fn ping(nonce: u64) -> Self {
        Self {
            nonce: Some(nonce),
            ..Self::new(MessageKind::Ping)
        }
    }

    pub fn pong(nonce: u64) -> Self {
        Self {
            nonce: Some(nonce),
            ..Self::new(MessageKind::Pong)
        }
    }

    pub fn shutdown() -> Self {
        Self::new(MessageKind::Shutdown)
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(self.version));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    pub requests: u64,
    pub errors: u64,
    pub events: u64,
    pub dropped_events: u64,
    pub started_at_ms: u64,
    pub uptime_ms: u64,
    pub latency_buckets_us: [u64; LATENCY_BUCKETS_US.len()],
    pub latency_sum_us: u64,
    pub latency_max_us: u64,
    pub cache_hits: u64,
    pub cache_coalesced: u64,
}

impl Metrics {
    pub fn record_latency(&mut self, microseconds: u64) {
        self.latency_sum_us = self.latency_sum_us.saturating_add(microseconds);
        self.latency_max_us = self.latency_max_us.max(microseconds);
        if let Some(index) = LATENCY_BUCKETS_US
            .iter()
            .position(|bucket| microseconds <= *bucket)
        {
            self.latency_buckets_us[index] = self.latency_buckets_us[index].saturating_add(1);
        }
    }

    pub fn percentile_us(&self, ratio: f64) -> u64 {
        let total = self.latency_buckets_us.iter().sum::<u64>();
        if total == 0 {
            return 0;
        }
        let rank = ((total as f64) * ratio).ceil() as u64;
        let mut cumulative = 0;
        for (index, count) in self.latency_buckets_us.iter().enumerate() {
            cumulative += count;
            if cumulative >= rank {
                return LATENCY_BUCKETS_US[index].min(self.latency_max_us);
            }
        }
        self.latency_max_us
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
}

#[derive(Debug)]
pub enum ProtocolError {
    FrameTooLarge(usize),
    InvalidLength,
    Json(serde_json::Error),
    UnsupportedVersion(u8),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameTooLarge(size) => write!(formatter, "frame size {size} exceeds limit"),
            Self::InvalidLength => write!(formatter, "invalid frame length"),
            Self::Json(error) => write!(formatter, "invalid JSON frame: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported protocol version {version}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<serde_json::Error> for ProtocolError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub fn encode(message: &Message) -> Result<Vec<u8>, ProtocolError> {
    message.validate()?;
    let payload = serde_json::to_vec(message)?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge(payload.len()));
    }
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

pub fn decode(payload: &[u8]) -> Result<Message, ProtocolError> {
    if payload.len() > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge(payload.len()));
    }
    if payload.is_empty() {
        return Err(ProtocolError::InvalidLength);
    }
    let message: Message = serde_json::from_slice(payload)?;
    message.validate()?;
    Ok(message)
}

pub fn decode_length_prefix(bytes: &[u8]) -> Result<Option<usize>, ProtocolError> {
    if bytes.len() < 4 {
        return Ok(None);
    }
    let length = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if length > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge(length));
    }
    if length == 0 {
        return Err(ProtocolError::InvalidLength);
    }
    Ok(Some(length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn latency_percentiles_use_fixed_buckets() {
        let mut metrics = Metrics::default();
        for latency in [500, 2_000, 20_000, 100_000] {
            metrics.record_latency(latency);
        }
        assert_eq!(metrics.percentile_us(0.50), 5_000);
        assert_eq!(metrics.percentile_us(0.95), 100_000);
        assert_eq!(metrics.latency_max_us, 100_000);
    }

    #[test]
    fn request_response_round_trip() {
        let request = Message::request(7, 3, "health", json!({}), Some(1000));
        let frame = encode(&request).unwrap();
        let length = decode_length_prefix(&frame).unwrap().unwrap();
        let decoded = decode(&frame[4..4 + length]).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn rejects_oversized_frame() {
        let mut bytes = vec![0; 4];
        bytes.copy_from_slice(&((MAX_FRAME_SIZE + 1) as u32).to_le_bytes());
        assert!(matches!(
            decode_length_prefix(&bytes),
            Err(ProtocolError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn rejects_unknown_protocol_version() {
        let mut message = Message::ping(1);
        message.version = 99;
        assert!(matches!(
            message.validate(),
            Err(ProtocolError::UnsupportedVersion(99))
        ));
    }
}
