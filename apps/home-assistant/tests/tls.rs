//! rumqttc panics when any of the system's root certificates fails to load. A session
//! must end with an error instead, so the adapter retries rather than aborting. This
//! is its own test binary because it changes the process environment.

use scent_core::Secret;
use scent_home_assistant::{config::Config, runtime};

#[tokio::test]
async fn unloadable_root_certificates_end_the_session_instead_of_panicking() {
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe { std::env::set_var("SSL_CERT_FILE", "/nonexistent/scent-control-roots.pem") };
    let config = Config::parse(include_bytes!("../config.example.json")).unwrap();
    let password = Secret::new("synthetic".into());
    assert_eq!(
        runtime::session(config, &password, &password).await,
        Err("Could not load the system's root certificates.")
    );
}
