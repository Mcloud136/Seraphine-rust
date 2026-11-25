use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct SimulatedLcuEvent {
    pub sequence: u64,
    pub event_type: String,
    pub payload: String,
}

#[derive(Debug, Default)]
pub struct StressResult {
    pub events: u64,
    pub parsed: u64,
    pub errors: u64,
    pub elapsed_ms: u128,
}

pub fn simulated_event_json(sequence: u64) -> String {
    format!(
        r#"{{"sequence":{sequence},"event_type":"OnJsonApiEvent_lol-gameflow_v1_gameflow-phase","payload":"{{\"phase\":\"ChampSelect\",\"tick\":{sequence}}}"}}"#
    )
}

pub fn run_stress(duration: Duration, event_rate: u64) -> StressResult {
    let started = Instant::now();
    let mut sequence = 0_u64;
    let mut result = StressResult::default();

    while started.elapsed() < duration {
        let json = simulated_event_json(sequence);
        match serde_json::from_str::<SimulatedLcuEvent>(&json) {
            Ok(event)
                if event.sequence == sequence
                    && event.event_type == "OnJsonApiEvent_lol-gameflow_v1_gameflow-phase"
                    && !event.payload.is_empty() =>
            {
                result.parsed += 1;
            }
            _ => result.errors += 1,
        }
        result.events += 1;
        sequence += 1;

        let next = started + Duration::from_secs_f64(sequence as f64 / event_rate as f64);
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        }
    }

    result.elapsed_ms = started.elapsed().as_millis();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulated_lcu_event_is_valid_json() {
        let event: SimulatedLcuEvent = serde_json::from_str(&simulated_event_json(42)).unwrap();
        assert_eq!(event.sequence, 42);
        assert_eq!(
            event.event_type,
            "OnJsonApiEvent_lol-gameflow_v1_gameflow-phase"
        );
        assert!(event.payload.contains("ChampSelect"));
        assert!(event.payload.contains("42"));
    }
}
