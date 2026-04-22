use std::{thread, time::Duration};

use anyhow::Result;

use crate::{
    config::AppConfig,
    logging::DaemonLogger,
    network::{CampusEnvironment, detect_campus_environment},
    portal::{LoginStatus, PortalClient, ProbeStatus},
};

const RETRY_BACKOFF_SECS: [u64; 5] = [5, 15, 30, 60, 120];

pub fn run_daemon(config: &AppConfig) -> Result<()> {
    let client = PortalClient::new(Duration::from_secs(config.detect.request_timeout_secs))?;
    let online_sleep = Duration::from_secs(config.daemon.online_check_interval_secs);
    let offline_sleep = Duration::from_secs(config.daemon.offline_check_interval_secs);
    let mut retry_step = 0usize;
    let mut last_state = String::new();
    let mut logger = DaemonLogger::new();

    logger.info("daemon started");

    loop {
        let campus_env = match detect_campus_environment(config) {
            Ok(value) => value,
            Err(error) => {
                emit_state(&mut last_state, "detect-error", &mut logger);
                logger.warn(format!("failed to detect network environment: {error}"));
                thread::sleep(retry_delay(retry_step));
                retry_step = (retry_step + 1).min(RETRY_BACKOFF_SECS.len() - 1);
                continue;
            }
        };

        match campus_env {
            CampusEnvironment::OnCampus(reason) => {
                emit_state(&mut last_state, "on-campus", &mut logger);
                logger.info(format!("campus network detected: {reason}"));
            }
            CampusEnvironment::OffCampus(reason) => {
                emit_state(&mut last_state, "off-campus", &mut logger);
                logger.info(format!("campus network not detected: {reason}"));
                retry_step = 0;
                thread::sleep(offline_sleep);
                continue;
            }
        }

        let probe_status = match client.probe(config) {
            Ok(status) => status,
            Err(error) => {
                emit_state(&mut last_state, "probe-error", &mut logger);
                logger.warn(format!("probe error: {error}"));
                thread::sleep(retry_delay(retry_step));
                retry_step = (retry_step + 1).min(RETRY_BACKOFF_SECS.len() - 1);
                continue;
            }
        };

        match probe_status {
            ProbeStatus::Online => {
                emit_state(&mut last_state, "online", &mut logger);
                retry_step = 0;
                thread::sleep(online_sleep);
            }
            ProbeStatus::NeedsLogin => {
                emit_state(&mut last_state, "portal", &mut logger);
                match client.login_and_verify(config) {
                    Ok(outcome) => match outcome.status {
                        LoginStatus::Success => {
                            logger.info(format!("login successful: {}", outcome.detail));
                            retry_step = 0;
                            thread::sleep(online_sleep);
                        }
                        LoginStatus::Failed => {
                            logger.warn(format!("login failed: {}", outcome.detail));
                            thread::sleep(retry_delay(retry_step));
                            retry_step = (retry_step + 1).min(RETRY_BACKOFF_SECS.len() - 1);
                        }
                    },
                    Err(error) => {
                        logger.warn(format!("login request error: {error}"));
                        thread::sleep(retry_delay(retry_step));
                        retry_step = (retry_step + 1).min(RETRY_BACKOFF_SECS.len() - 1);
                    }
                }
            }
            ProbeStatus::Unreachable(detail) => {
                emit_state(&mut last_state, "unreachable", &mut logger);
                logger.warn(format!("probe failed: {detail}"));
                thread::sleep(retry_delay(retry_step));
                retry_step = (retry_step + 1).min(RETRY_BACKOFF_SECS.len() - 1);
            }
        }
    }
}

fn retry_delay(step: usize) -> Duration {
    Duration::from_secs(RETRY_BACKOFF_SECS[step])
}

fn emit_state(last_state: &mut String, next_state: &str, logger: &mut DaemonLogger) {
    if last_state != next_state {
        logger.info(format!("state changed: {next_state}"));
        last_state.clear();
        last_state.push_str(next_state);
    }
}
