use std::{io::Read, time::Duration};

use anyhow::{Context, Result};
use reqwest::{
    Url,
    blocking::{Client, Response},
    header::{CONTENT_TYPE, HeaderMap, HeaderValue, ORIGIN, REFERER, SERVER, USER_AGENT},
};

use crate::config::AppConfig;

const PASSWORD_PID: &str = "2";
const PASSWORD_CALG: &str = "12345678";
const BODY_READ_LIMIT: u64 = 8 * 1024;
const PROBE_READ_LIMIT: u64 = 4 * 1024;
const DEFAULT_USER_AGENT: &str =
    "campus-network-autologin/0.1 (+https://github.com/github/copilot)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeStatus {
    Online,
    NeedsLogin,
    Unreachable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginStatus {
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginOutcome {
    pub status: LoginStatus,
    pub detail: String,
}

pub struct PortalClient {
    client: Client,
}

impl PortalClient {
    pub fn new(timeout: Duration) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { client })
    }

    pub fn probe(&self, config: &AppConfig) -> Result<ProbeStatus> {
        let portal_host = Url::parse(&config.auth.portal_url)
            .ok()
            .and_then(|url| url.host_str().map(ToOwned::to_owned));

        let response = match self
            .client
            .get(&config.detect.probe_url)
            .header(USER_AGENT, DEFAULT_USER_AGENT)
            .send()
        {
            Ok(response) => response,
            Err(error) => {
                let detail = if error.is_timeout() {
                    format!("request timed out: {error}")
                } else if error.is_connect() {
                    format!("connection failed: {error}")
                } else if error.is_request() {
                    format!("request could not be sent: {error}")
                } else {
                    format!("request failed: {error}")
                };
                return Ok(ProbeStatus::Unreachable(detail));
            }
        };

        let final_url = response.url().clone();
        let server_header = response
            .headers()
            .get(SERVER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let snippet = read_text_prefix(response, PROBE_READ_LIMIT)?;

        if looks_like_portal(&final_url, &server_header, &snippet, portal_host.as_deref()) {
            Ok(ProbeStatus::NeedsLogin)
        } else {
            Ok(ProbeStatus::Online)
        }
    }

    pub fn login_and_verify(&self, config: &AppConfig) -> Result<LoginOutcome> {
        let body = self.submit_login(config)?;
        let body_result = classify_login_body(&body);
        let probe_status = self.probe(config)?;

        match (body_result, probe_status) {
            (_, ProbeStatus::Online) => Ok(LoginOutcome {
                status: LoginStatus::Success,
                detail: "authentication passed and the probe URL is reachable".to_owned(),
            }),
            (BodyResult::Failure(reason), _) => Ok(LoginOutcome {
                status: LoginStatus::Failed,
                detail: reason,
            }),
            (BodyResult::Success(reason), ProbeStatus::NeedsLogin) => Ok(LoginOutcome {
                status: LoginStatus::Failed,
                detail: format!("{reason}, but the probe URL is still redirected to the portal"),
            }),
            (BodyResult::Success(reason), ProbeStatus::Unreachable(detail)) => Ok(LoginOutcome {
                status: LoginStatus::Failed,
                detail: format!("{reason}, but the connectivity check failed: {detail}"),
            }),
            (BodyResult::Unknown, ProbeStatus::NeedsLogin) => Ok(LoginOutcome {
                status: LoginStatus::Failed,
                detail: "portal still intercepts traffic after login".to_owned(),
            }),
            (BodyResult::Unknown, ProbeStatus::Unreachable(detail)) => Ok(LoginOutcome {
                status: LoginStatus::Failed,
                detail: format!("login result was unclear and the probe failed: {detail}"),
            }),
        }
    }

    fn submit_login(&self, config: &AppConfig) -> Result<String> {
        let portal_url = Url::parse(&config.auth.portal_url).context("portal_url is invalid")?;
        let origin = portal_origin(&portal_url);
        let referer = portal_url.as_str().to_owned();
        let encoded_password = encode_password(&config.auth.password);

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_USER_AGENT));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        headers.insert(
            REFERER,
            HeaderValue::from_str(&referer).context("failed to build Referer header")?,
        );
        headers.insert(
            ORIGIN,
            HeaderValue::from_str(&origin).context("failed to build Origin header")?,
        );

        let response = self
            .client
            .post(portal_url)
            .headers(headers)
            .form(&[
                ("DDDDD", config.auth.username.as_str()),
                ("upass", encoded_password.as_str()),
                ("R1", "0"),
                ("R2", "1"),
                ("para", "00"),
                ("0MKKey", "123456"),
                ("v6ip", ""),
            ])
            .send()
            .context("portal login request failed")?;

        read_text_prefix(response, BODY_READ_LIMIT)
    }
}

fn encode_password(password: &str) -> String {
    let digest = md5::compute(format!("{PASSWORD_PID}{password}{PASSWORD_CALG}"));
    format!("{digest:x}{PASSWORD_CALG}{PASSWORD_PID}")
}

fn read_text_prefix(response: Response, limit: u64) -> Result<String> {
    let mut bytes = Vec::new();
    response
        .take(limit)
        .read_to_end(&mut bytes)
        .context("failed to read response body")?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn looks_like_portal(
    final_url: &Url,
    server_header: &str,
    snippet: &str,
    portal_host: Option<&str>,
) -> bool {
    if portal_host.is_some_and(|host| final_url.host_str() == Some(host)) {
        return true;
    }
    if server_header.contains("DrcomServer") {
        return true;
    }

    snippet.contains("Dr.COMWebLoginID")
        || snippet.contains("name=\"DDDDD\"")
        || snippet.contains("name=\"upass\"")
        || snippet.contains("function ee()")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BodyResult {
    Success(String),
    Failure(String),
    Unknown,
}

fn classify_login_body(body: &str) -> BodyResult {
    if body.contains("Msg=15") || body.contains("登录成功") {
        return BodyResult::Success("portal reported a successful login".to_owned());
    }

    if body.contains("Msg=7") {
        return BodyResult::Success("account is already online".to_owned());
    }

    if let Some(msg_code) = extract_numeric_assignment(body, "Msg=") {
        return match msg_code {
            0 | 1 => BodyResult::Failure(parse_failure_message(body)),
            2 => BodyResult::Failure("the account is already in use".to_owned()),
            3 => BodyResult::Failure("the account is restricted to a specific address".to_owned()),
            4 => BodyResult::Failure("the account is out of quota or time".to_owned()),
            5 => BodyResult::Failure("the account is suspended".to_owned()),
            6 => BodyResult::Failure("the portal system buffer is full".to_owned()),
            8 => BodyResult::Failure(
                "the account is already in use and cannot be modified".to_owned(),
            ),
            11 => {
                BodyResult::Failure("the account can only be used from a bound device".to_owned())
            }
            14 => BodyResult::Failure("portal reported logout success instead of login".to_owned()),
            15 => BodyResult::Success("portal reported a successful login".to_owned()),
            _ => BodyResult::Failure(format!("portal returned Msg={msg_code}")),
        };
    }

    BodyResult::Unknown
}

fn parse_failure_message(body: &str) -> String {
    if let Some(msga) = extract_single_quoted_value(body, "msga='") {
        return match msga.as_str() {
            "" => "invalid username or password".to_owned(),
            "userid error3" => "invalid username or password".to_owned(),
            "error0" => "this IP does not allow web login".to_owned(),
            "error1" => "this account does not allow web login".to_owned(),
            "error2" => "this account does not allow password changes".to_owned(),
            other => format!("portal rejected the login: {other}"),
        };
    }

    "invalid username or password".to_owned()
}

fn extract_numeric_assignment(body: &str, prefix: &str) -> Option<u32> {
    let start = body.find(prefix)? + prefix.len();
    let digits: String = body[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn extract_single_quoted_value(body: &str, prefix: &str) -> Option<String> {
    let start = body.find(prefix)? + prefix.len();
    let remainder = &body[start..];
    let end = remainder.find('\'')?;
    Some(remainder[..end].to_owned())
}

fn portal_origin(portal_url: &Url) -> String {
    let scheme = portal_url.scheme();
    let host = portal_url.host_str().unwrap_or_default();
    match portal_url.port() {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{BodyResult, classify_login_body, encode_password};

    #[test]
    fn encodes_password_like_the_portal_script() {
        let encoded = encode_password("password");
        assert_eq!(encoded, "919521029e7e69584d83e54a45e11a2b123456782");
    }

    #[test]
    fn parses_invalid_credentials() {
        let body = "Msg=01;msga='userid error3';";
        assert_eq!(
            classify_login_body(body),
            BodyResult::Failure("invalid username or password".to_owned())
        );
    }

    #[test]
    fn parses_login_success() {
        let body = "Msg=15;document.write(\"登录成功\")";
        assert_eq!(
            classify_login_body(body),
            BodyResult::Success("portal reported a successful login".to_owned())
        );
    }
}
