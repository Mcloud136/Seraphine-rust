use seraphine_lcu::{ExternalClient, LcuClient, SgpClient};

// Keep the process-level memory assertion in its own test binary: lib tests run
// concurrently and their allocator high-water marks would contaminate it.
#[test]
fn isolated_clients_stay_within_idle_memory_budget() {
    let _lcu = LcuClient::new(55123, "test-token").unwrap();
    let _sgp = SgpClient::new("HN1").unwrap();
    let _external = ExternalClient::new(None).unwrap();
    let memory = seraphine_platform::current_process_memory().unwrap();
    println!(
        "isolated clients: working_set={} private_usage={}",
        memory.working_set_bytes, memory.private_usage_bytes
    );
    assert!(memory.working_set_bytes < 10 * 1024 * 1024);
    assert!(memory.private_usage_bytes < 10 * 1024 * 1024);
}
