//! SCP argument parsing for host extraction.

/// Extract unique remote hosts from SCP arguments.
///
/// Skips option flags (starting with `-`) and extracts the host portion
/// from arguments containing `:` (the `host:path` format).
#[must_use]
pub fn extract_hosts(args: &[String]) -> Vec<String> {
    let mut hosts = Vec::new();
    let mut skip_next = false;

    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }

        // Options that take a value argument
        if matches!(
            arg.as_str(),
            "-c" | "-D" | "-F" | "-i" | "-J" | "-l" | "-o" | "-P" | "-S" | "-X" | "-Y" | "-Z"
        ) {
            skip_next = true;
            continue;
        }

        // Skip flag-only options
        if arg.starts_with('-') {
            continue;
        }

        // Extract host from user@host:path or host:path
        if let Some(colon_pos) = arg.find(':') {
            let host_part = &arg[..colon_pos];
            if !host_part.is_empty() {
                // Handle user@host format
                let host = host_part
                    .rsplit_once('@')
                    .map_or(host_part, |(_, h)| h)
                    .to_owned();
                if !host.is_empty() && !hosts.contains(&host) {
                    hosts.push(host);
                }
            }
        }
    }

    hosts
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::extract_hosts;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| String::from(*a)).collect()
    }

    #[test]
    fn single_remote_host() {
        let result = extract_hosts(&s(&["local.txt", "server1:/remote/path"]));
        assert_eq!(result, vec!["server1"]);
    }

    #[test]
    fn user_at_host() {
        let result = extract_hosts(&s(&["user@server2:/path", "local.txt"]));
        assert_eq!(result, vec!["server2"]);
    }

    #[test]
    fn multiple_hosts_deduped() {
        let result = extract_hosts(&s(&["server1:/a", "server2:/b", "server1:/c"]));
        assert_eq!(result, vec!["server1", "server2"]);
    }

    #[test]
    fn local_file_only() {
        let result = extract_hosts(&s(&["local1.txt", "local2.txt"]));
        assert!(result.is_empty());
    }

    #[test]
    fn skip_option_flags() {
        let result = extract_hosts(&s(&["-P", "22", "-r", "server:/path"]));
        assert_eq!(result, vec!["server"]);
    }

    #[test]
    fn empty_args() {
        let result = extract_hosts(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn skip_identity_option() {
        let result = extract_hosts(&s(&["-i", "/key", "host:/path"]));
        assert_eq!(result, vec!["host"]);
    }
}
