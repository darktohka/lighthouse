//! Pure CIDR/IP helpers shared by the service-account IP allowlist and the
//! trusted-proxy gate.
//!
//! Everything here is side-effect free and synchronous: parsing and matching so
//! the callers only ever deal with already-normalized values.

use std::net::{IpAddr, SocketAddr};

use ipnet::IpNet;

/// Parses and normalizes a single CIDR or bare IP into its canonical string.
///
/// A bare address becomes a host route (`/32` for IPv4, `/128` for IPv6); a
/// network is truncated to its base address. IPv4-mapped IPv6 ranges
/// (`::ffff:0:0/96`) are rejected because they would silently alias IPv4 space.
pub fn parse_cidr(raw: &str) -> Result<String, String> {
    parse_one(raw).map(|network| network.to_string())
}

/// Parses a comma-separated CIDR list, skipping empty entries. Fails on the
/// first invalid value.
pub fn parse_cidr_list(raw: &str) -> Result<Vec<IpNet>, String> {
    let parsed: Result<Vec<IpNet>, String> = raw
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(parse_one)
        .collect();
    parsed
}

/// Normalizes a client address into a canonical [`IpAddr`].
///
/// Accepts a bare IP, a `host:port` socket address (including bracketed IPv6)
/// and, defensively, a bracketed IPv6 literal without a port. Returns `None`
/// when the value cannot be interpreted as an address.
pub fn normalize_client_ip(raw: &str) -> Option<IpAddr> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Some(ip.to_canonical());
    }
    if let Ok(socket) = trimmed.parse::<SocketAddr>() {
        return Some(socket.ip().to_canonical());
    }
    // Defensive fallbacks for forms neither parser accepts: `[::1]` (bracketed,
    // no port) and an unbracketed `address:port` whose port is numeric.
    let stripped = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map(str::to_string)
        .or_else(|| {
            trimmed.rsplit_once(':').and_then(|(host, port)| {
                (!host.is_empty() && !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
                    .then(|| host.to_string())
            })
        });
    stripped
        .and_then(|value| value.parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
}

/// True when `candidate` (any client-address form) falls inside `network`.
pub fn contains(network: &IpNet, candidate: &str) -> bool {
    normalize_client_ip(candidate).is_some_and(|ip| network.contains(&ip))
}

/// Parses one CIDR or bare address into a truncated [`IpNet`].
fn parse_one(raw: &str) -> Result<IpNet, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("CIDR must not be empty".to_string());
    }

    if let Ok(network) = trimmed.parse::<IpNet>() {
        if let IpNet::V6(v6) = &network {
            if v6.addr().to_ipv4_mapped().is_some() {
                return Err(format!(
                    "IPv4-mapped IPv6 ranges are not allowed: {trimmed}"
                ));
            }
        }
        return Ok(network.trunc());
    }

    let ip: IpAddr = trimmed
        .parse()
        .map_err(|_| format!("invalid CIDR or IP address: {trimmed}"))?;
    let prefix = match ip {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    };
    IpNet::new(ip, prefix)
        .map(|network| network.trunc())
        .map_err(|err| format!("invalid CIDR {trimmed}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_ipv4_becomes_a_host_route() {
        assert_eq!(parse_cidr("10.0.0.5").expect("parse"), "10.0.0.5/32");
    }

    #[test]
    fn bare_ipv6_becomes_a_host_route() {
        assert_eq!(parse_cidr("2001:db8::1").expect("parse"), "2001:db8::1/128");
    }

    #[test]
    fn networks_are_truncated_to_their_base() {
        assert_eq!(parse_cidr("10.1.2.3/24").expect("parse"), "10.1.2.0/24");
    }

    #[test]
    fn rejects_ipv4_mapped_ranges() {
        assert!(parse_cidr("::ffff:1.2.3.0/120").is_err());
    }

    #[test]
    fn rejects_garbage_and_empty() {
        assert!(parse_cidr("not-a-network").is_err());
        assert!(parse_cidr("   ").is_err());
        assert!(parse_cidr_list("10.0.0.0/8, nope").is_err());
    }

    #[test]
    fn skips_empty_list_entries() {
        let list = parse_cidr_list(" 10.0.0.0/8 ,, 192.168.0.0/16 ").expect("parse");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn mapped_client_matches_its_ipv4_network() {
        let network: IpNet = parse_cidr("10.0.0.0/8")
            .expect("parse")
            .parse()
            .expect("net");
        assert!(contains(&network, "::ffff:10.0.0.5"));
    }

    #[test]
    fn normalizes_socket_addresses() {
        assert_eq!(
            normalize_client_ip("1.2.3.4:443"),
            Some("1.2.3.4".parse().expect("ip"))
        );
        assert_eq!(
            normalize_client_ip("[2001:db8::1]:443"),
            Some("2001:db8::1".parse().expect("ip"))
        );
        assert_eq!(
            normalize_client_ip("[2001:db8::1]"),
            Some("2001:db8::1".parse().expect("ip"))
        );
    }
}
