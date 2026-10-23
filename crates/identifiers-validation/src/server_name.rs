use crate::error::Error;

pub fn validate(server_name: &str) -> Result<(), Error> {
    use std::net::Ipv6Addr;

    // A server name is at most a 255-byte hostname, a colon, and a 5-digit port.
    // https://spec.matrix.org/v1.19/appendices/#server-name
    if server_name.is_empty() || server_name.len() > 261 {
        return Err(Error::InvalidServerName);
    }

    let end_of_host = if server_name.starts_with('[') {
        let end_of_ipv6 = match server_name.find(']') {
            Some(idx) => idx,
            None => return Err(Error::InvalidServerName),
        };

        if server_name[1..end_of_ipv6].parse::<Ipv6Addr>().is_err() {
            return Err(Error::InvalidServerName);
        }

        end_of_ipv6 + 1
    } else {
        #[allow(clippy::unnecessary_lazy_evaluations)]
        let end_of_host = server_name.find(':').unwrap_or_else(|| server_name.len());

        if server_name[..end_of_host]
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.'))
        {
            return Err(Error::InvalidServerName);
        }

        end_of_host
    };

    if end_of_host == 0 {
        return Err(Error::InvalidServerName);
    }

    if server_name.len() != end_of_host
        && (
            // hostname is followed by something other than ":port"
            server_name.as_bytes()[end_of_host] != b':'
            // the remaining characters after ':' are not a valid port
            || server_name[end_of_host + 1..].parse::<u16>().is_err()
        )
    {
        Err(Error::InvalidServerName)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::error::Error;

    #[test]
    fn enforces_server_name_length_bound() {
        assert!(validate(&format!("{}:65535", "a".repeat(255))).is_ok());
        assert_eq!(validate(&"a".repeat(262)), Err(Error::InvalidServerName));
        assert_eq!(
            validate(&format!("{}:65535", "a".repeat(256))),
            Err(Error::InvalidServerName),
        );
    }

    #[test]
    fn accepts_ip_addresses_and_regular_hosts() {
        for name in ["example.org", "example.org:8448", "127.0.0.1", "[::1]:8448"] {
            assert!(validate(name).is_ok(), "{name}");
        }
    }
}
