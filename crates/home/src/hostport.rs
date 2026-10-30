//! Go `net.SplitHostPort` / `net.JoinHostPort`, and roomnet's `HostPort`
//! (I:internal/mesh/roomnet/roomnet.go:938-959), which reads a typed server
//! address with the port defaulted.

/// Go `net.JoinHostPort`: a host with a colon gets brackets (the mesh
/// crate's copy, which `NodeAddr::dial` uses).
pub use mesh::servers::join_host_port;

/// Go `net.SplitHostPort`, `None` where Go returns an error.
pub fn split_host_port(hostport: &str) -> Option<(String, String)> {
    let i = hostport.rfind(':')?;
    let (host, j, k);
    if hostport.starts_with('[') {
        let end = hostport.find(']')?;
        if end + 1 == hostport.len() {
            return None; // missing port
        }
        if end + 1 != i {
            return None; // "]" not followed by the last ":"
        }
        host = &hostport[1..end];
        j = 1;
        k = end + 1;
    } else {
        host = &hostport[..i];
        if host.contains(':') {
            return None; // too many colons
        }
        j = 0;
        k = 0;
    }
    if hostport[j..].contains('[') || hostport[k..].contains(']') {
        return None;
    }
    Some((host.to_string(), hostport[i + 1..].to_string()))
}

/// Go `strconv.Atoi` for the port check: optional sign, decimal digits.
fn atoi(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok()
}

/// roomnet `HostPort`: "1.2.3.4" or "pc.example" alone gets `port`, an
/// IPv6 address may go without brackets. The errors are Go's sentences.
pub fn host_port(input: &str, port: i64) -> Result<String, String> {
    let input = input.trim_matches(|c: char| c.is_whitespace());
    if input.is_empty() {
        return Err("no address".into());
    }
    if let Some((h, p)) = split_host_port(input) {
        match atoi(&p) {
            Some(n) if n > 0 && n <= 65535 && !h.is_empty() => {
                return Ok(join_host_port(&h, &p));
            }
            _ => return Err("that isn't a host and port".into()),
        }
    }
    if port <= 0 {
        return Err("the address needs a port, like 203.0.113.7:24817".into());
    }
    if input.contains(['/', ' ']) {
        return Err("that isn't a host name or address".into());
    }
    Ok(join_host_port(input.trim_matches(['[', ']']), port))
}

/// Go `splitPublic` (home.go:808-817): a typed public address, host or
/// host:port, as host and port.
pub fn split_public(input: &str, port: i64) -> Result<(String, i64), String> {
    let hp = host_port(input, port)?;
    // Go ignores this split's error (`h, p, _ :=`): an odd result is "", 0.
    let (h, p) = split_host_port(&hp).unwrap_or_default();
    Ok((h, atoi(&p).unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_cases() {
        assert_eq!(host_port("1.2.3.4", 25817).unwrap(), "1.2.3.4:25817");
        assert_eq!(host_port(" pc.example:443 ", 1).unwrap(), "pc.example:443");
        assert_eq!(host_port("2001:db8::7", 9).unwrap(), "[2001:db8::7]:9");
        assert_eq!(host_port("[2001:db8::7]:9", 1).unwrap(), "[2001:db8::7]:9");
        assert_eq!(host_port("[::1]", 9).unwrap(), "[::1]:9");
        assert_eq!(host_port("", 9).unwrap_err(), "no address");
        assert_eq!(
            host_port("h:0", 9).unwrap_err(),
            "that isn't a host and port"
        );
        assert_eq!(
            host_port(":80", 9).unwrap_err(),
            "that isn't a host and port"
        );
        assert_eq!(
            host_port("h", 0).unwrap_err(),
            "the address needs a port, like 203.0.113.7:24817"
        );
        assert_eq!(
            host_port("not an address/", 1).unwrap_err(),
            "that isn't a host name or address"
        );
        assert_eq!(
            split_public("pc.example:443", 1).unwrap(),
            ("pc.example".into(), 443)
        );
    }
}
