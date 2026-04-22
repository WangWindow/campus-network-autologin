use std::{
    net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs},
    time::Duration,
};

use anyhow::{Context, Result};
use ipnet::Ipv4Net;

use crate::config::AppConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CampusEnvironment {
    OnCampus(String),
    OffCampus(String),
}

pub fn detect_campus_environment(config: &AppConfig) -> Result<CampusEnvironment> {
    let local_ipv4 =
        local_ipv4_addresses().context("failed to inspect local network interfaces")?;
    if local_ipv4.is_empty() {
        return Ok(CampusEnvironment::OffCampus(
            "no non-loopback IPv4 addresses are active".to_owned(),
        ));
    }

    let timeout = Duration::from_secs(config.detect.request_timeout_secs);
    let reachable_gateways = config
        .campus
        .gateway_hosts
        .iter()
        .filter(|gateway| is_gateway_reachable(gateway, timeout))
        .cloned()
        .collect::<Vec<_>>();

    if reachable_gateways.is_empty() {
        return Ok(CampusEnvironment::OffCampus(format!(
            "local IPv4 {}, but no configured campus gateway responded ({})",
            format_ipv4_list(&local_ipv4),
            config.campus.gateway_hosts.join(", ")
        )));
    }

    let reason = if config.campus.ipv4_cidrs.is_empty() {
        format!(
            "gateway(s) {} responded while local IPv4 is {}",
            reachable_gateways.join(", "),
            format_ipv4_list(&local_ipv4)
        )
    } else {
        let campus_cidrs = parse_ipv4_cidrs(&config.campus.ipv4_cidrs)?;
        let matching_ips = local_ipv4
            .iter()
            .copied()
            .filter(|ip| campus_cidrs.iter().any(|cidr| cidr.contains(ip)))
            .collect::<Vec<_>>();

        if matching_ips.is_empty() {
            format!(
                "gateway(s) {} responded; local IPv4 {} did not match the optional CIDRs {}",
                reachable_gateways.join(", "),
                format_ipv4_list(&local_ipv4),
                config.campus.ipv4_cidrs.join(", ")
            )
        } else {
            format!(
                "gateway(s) {} responded and local IPv4 {} matched optional CIDRs {}",
                reachable_gateways.join(", "),
                format_ipv4_list(&matching_ips),
                config.campus.ipv4_cidrs.join(", ")
            )
        }
    };

    Ok(CampusEnvironment::OnCampus(reason))
}

fn parse_ipv4_cidrs(cidr_values: &[String]) -> Result<Vec<Ipv4Net>> {
    cidr_values
        .iter()
        .map(|cidr| {
            cidr.parse::<Ipv4Net>()
                .with_context(|| format!("invalid campus IPv4 CIDR: {cidr}"))
        })
        .collect()
}

fn is_gateway_reachable(gateway: &str, timeout: Duration) -> bool {
    resolve_gateway_targets(gateway)
        .map(|targets| {
            targets
                .into_iter()
                .any(|target| TcpStream::connect_timeout(&target, timeout).is_ok())
        })
        .unwrap_or(false)
}

fn resolve_gateway_targets(gateway: &str) -> Result<Vec<SocketAddr>> {
    let host = gateway.trim();
    let with_port = if host.contains(':') && !host.contains('.') {
        host.to_owned()
    } else {
        format!("{host}:80")
    };
    let targets = with_port
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve campus gateway {gateway}"))?
        .collect::<Vec<_>>();
    Ok(targets)
}

fn format_ipv4_list(addresses: &[std::net::Ipv4Addr]) -> String {
    addresses
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(any(unix, windows))]
fn local_ipv4_addresses() -> Result<Vec<Ipv4Addr>> {
    use std::net::IpAddr;

    use if_addrs::get_if_addrs;

    let mut addresses = get_if_addrs()?
        .into_iter()
        .filter_map(|iface| match iface.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() => Some(ip),
            _ => None,
        })
        .collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

#[cfg(not(any(unix, windows)))]
fn local_ipv4_addresses() -> Result<Vec<Ipv4Addr>> {
    anyhow::bail!("local IPv4 enumeration is not implemented for this platform")
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::parse_ipv4_cidrs;

    #[test]
    fn parses_multiple_ipv4_cidrs() {
        let cidrs = parse_ipv4_cidrs(&["172.18.0.0/16".to_owned(), "10.0.0.0/8".to_owned()])
            .expect("CIDRs should parse");
        assert!(
            cidrs
                .iter()
                .any(|cidr| cidr.contains(&Ipv4Addr::new(172, 18, 3, 3)))
        );
        assert!(
            cidrs
                .iter()
                .any(|cidr| cidr.contains(&Ipv4Addr::new(10, 1, 2, 3)))
        );
    }
}
