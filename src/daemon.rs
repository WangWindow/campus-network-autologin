use std::{thread, time::Duration};

use anyhow::Result;

use crate::{
    config::AppConfig,
    network::{CampusEnvironment, detect_campus_environment},
    portal::{LoginStatus, PortalClient, ProbeStatus},
};

const RETRY_BACKOFF_SECS: [u64; 5] = [5, 15, 30, 60, 120];

pub fn run_daemon(config: &AppConfig) -> Result<()> {
    let client = PortalClient::new(Duration::from_secs(config.detect.request_timeout_secs))?;
    let online_sleep = Duration::from_secs(config.daemon.online_check_interval_secs);
    let mut retry_step = 0usize;
    let mut last_state = String::new();

    loop {
        match detect_campus_environment(config)? {
            CampusEnvironment::OnCampus(reason) => {
                emit_state(&mut last_state, "on-campus");
                eprintln!("campus network detected: {reason}");
            }
            CampusEnvironment::OffCampus(reason) => {
                emit_state(&mut last_state, "off-campus");
                retry_step = 0;
                eprintln!("campus network not detected: {reason}");
                thread::sleep(online_sleep);
                continue;
            }
        }

        match client.probe(config)? {
            ProbeStatus::Online => {
                emit_state(&mut last_state, "online");
                retry_step = 0;
                thread::sleep(online_sleep);
            }
            ProbeStatus::NeedsLogin => {
                emit_state(&mut last_state, "portal");
                let result = client.login_and_verify(config)?;
                match result.status {
                    LoginStatus::Success => {
                        eprintln!("login successful: {}", result.detail);
                        retry_step = 0;
                        last_state.clear();
                        thread::sleep(online_sleep);
                    }
                    LoginStatus::Failed => {
                        eprintln!("login failed: {}", result.detail);
                        thread::sleep(retry_delay(retry_step));
                        retry_step = (retry_step + 1).min(RETRY_BACKOFF_SECS.len() - 1);
                    }
                }
            }
            ProbeStatus::Unreachable(detail) => {
                emit_state(&mut last_state, "unreachable");
                eprintln!("probe failed: {detail}");
                thread::sleep(retry_delay(retry_step));
                retry_step = (retry_step + 1).min(RETRY_BACKOFF_SECS.len() - 1);
            }
        }
    }
}

fn retry_delay(step: usize) -> Duration {
    Duration::from_secs(RETRY_BACKOFF_SECS[step])
}

fn emit_state(last_state: &mut String, next_state: &str) {
    if last_state != next_state {
        eprintln!("state changed: {next_state}");
        last_state.clear();
        last_state.push_str(next_state);
    }
}
