use std::time::Duration;

use scent_home_assistant::{config::Config, runtime};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (check, file) = match args.as_slice() {
        [file] if file != "--help" => (false, file),
        [flag, file] if flag == "--check" => (true, file),
        _ => return Err("Usage: scent-home-assistant [--check] <config.json>"),
    };
    let bytes = std::fs::read(file).map_err(|_| "Could not read the configuration file.")?;
    let config = Config::parse(&bytes)?;
    if check {
        println!("Configuration is valid. No broker connections were made.");
        return Ok(());
    }
    let upstream_password = config.diffuser_broker.password()?;
    let ha_password = config.home_assistant_broker.password()?;
    loop {
        // Never print network errors, config contents or raw device traffic: any
        // of those could contain credentials or personal device identifiers.
        let result = runtime::session(config.clone(), &upstream_password, &ha_password).await;
        if let Err(reason) = result {
            eprintln!("{reason} Retrying in 5 seconds with empty queues.");
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
