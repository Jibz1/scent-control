//! The contract with the listener: the messages in `fixtures/scent-control/` are exactly
//! what it publishes (apps/server tests render them), and they must decode here.

use std::{fs, path::PathBuf};

use scent_core::{ConnectedAt, Heartbeat, SharedState, Stamped, decode_heartbeat, decode_state};

const NOW: i64 = 1_792_510_230;

fn contract(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/scent-control")
        .join(name);
    fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_state_fixture_decodes() {
    let at = 1_792_510_100;
    let fields = [
        ("power", 1),
        ("intensity", 2),
        ("charging", 2),
        ("battery", 200),
        ("rssi", -66),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), Stamped { value, at }))
    .collect();
    assert_eq!(
        decode_state(&contract("state-v1.json"), NOW),
        Ok(SharedState {
            updated_at: 1_792_510_230,
            heard_at: Some(1_792_510_229),
            ping_at: Some(1_792_510_200),
            fields,
            connected: Some(ConnectedAt {
                fw: "2.30".to_owned(),
                model: "D".to_owned(),
                variant: "B".to_owned(),
                manufacturer: "01".to_owned(),
                at: 1_792_509_000,
            }),
        })
    );
}

#[test]
fn the_heartbeat_and_will_fixtures_decode() {
    assert_eq!(
        decode_heartbeat(&contract("heartbeat-v1.json"), NOW),
        Ok(Heartbeat {
            running: true,
            at: Some(1_792_510_230)
        })
    );
    assert_eq!(
        decode_heartbeat(&contract("will-v1.json"), NOW),
        Ok(Heartbeat {
            running: false,
            at: None
        })
    );
}
