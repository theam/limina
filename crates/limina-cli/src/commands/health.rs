//! `limina _healthcheck`: the container health probe (exit 0 on a 2xx answer, 1 otherwise).

use std::time::Duration;

use super::{Failure, Outcome};
use crate::output;

/// How long the probe waits; the container health check allows three seconds.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn healthcheck(url: &str) -> Outcome {
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .user_agent(concat!("limina/", env!("CARGO_PKG_VERSION"), " healthcheck"))
        .build();
    let response = match client {
        Ok(client) => client.get(url).send().await,
        Err(error) => {
            output::error_line(&format!("unhealthy: {error}"));
            return Err(Failure::Exit(1));
        }
    };
    match response {
        Ok(response) if response.status().is_success() => Ok(()),
        Ok(response) => {
            output::error_line(&format!("unhealthy: {url} answered HTTP {}", response.status()));
            Err(Failure::Exit(1))
        }
        Err(error) => {
            output::error_line(&format!("unhealthy: {url}: {error}"));
            Err(Failure::Exit(1))
        }
    }
}
