//! Start mosh-server via SSH and parse connection info.

use crate::MoshError;

/// Connection info returned by mosh-server.
#[derive(Debug, Clone)]
pub struct MoshConnectInfo {
    /// UDP port to connect to
    pub port: u16,
    /// Base64-encoded AES-128 key
    pub key: String,
    /// Optional IP address (from MOSH IP line)
    pub ip: Option<String>,
}

/// Launch mosh-server on a remote host via SSH.
///
/// Runs: `ssh [-J proxy] host "mosh-server new -s -c 256 -l LANG=... -l TERM=..."`
/// Parses stdout for `MOSH CONNECT <port> <key>` and optional `MOSH IP <addr>`.
pub async fn launch_mosh_server(
    host: &str,
    proxy_jump: Option<&str>,
    locale: Option<&str>,
    term: Option<&str>,
    extra_ssh_args: &[String],
) -> Result<MoshConnectInfo, MoshError> {
    let mut cmd = tokio::process::Command::new("ssh");

    // Proxy jump for relay support
    if let Some(proxy) = proxy_jump {
        cmd.arg("-J").arg(proxy);
    }

    // Extra SSH args (e.g. -p port, -i identity)
    for arg in extra_ssh_args {
        cmd.arg(arg);
    }

    cmd.arg(host);

    // Build mosh-server command
    let mut mosh_cmd = String::from("mosh-server new -s -c 256");
    if let Some(loc) = locale {
        mosh_cmd.push_str(&format!(" -l LANG={}", loc));
    }
    if let Some(t) = term {
        mosh_cmd.push_str(&format!(" -l TERM={}", t));
    }
    cmd.arg(mosh_cmd);

    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    log::debug!("launching mosh-server via SSH on {}", host);

    let output = cmd
        .output()
        .await
        .map_err(|e| MoshError::SshFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        return Err(MoshError::SshFailed(format!(
            "SSH exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    parse_mosh_output(&stdout)
}

/// Parse mosh-server stdout for connection info.
fn parse_mosh_output(stdout: &str) -> Result<MoshConnectInfo, MoshError> {
    let mut port = None;
    let mut key = None;
    let mut ip = None;

    for line in stdout.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("MOSH CONNECT ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                port = Some(
                    parts[0]
                        .parse::<u16>()
                        .map_err(|_| MoshError::InvalidMoshConnect)?,
                );
                key = Some(parts[1].to_string());
            }
        } else if let Some(rest) = line.strip_prefix("MOSH IP ") {
            ip = Some(rest.trim().to_string());
        }
    }

    match (port, key) {
        (Some(p), Some(k)) => Ok(MoshConnectInfo {
            port: p,
            key: k,
            ip,
        }),
        _ => Err(MoshError::InvalidMoshConnect),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_standard_output() {
        let output = "\n\nMOSH CONNECT 60001 AbCdEfGhIjKlMnOpQrStUw==\n\n";
        let info = parse_mosh_output(output).unwrap();
        assert_eq!(info.port, 60001);
        assert_eq!(info.key, "AbCdEfGhIjKlMnOpQrStUw==");
        assert!(info.ip.is_none());
    }

    #[test]
    fn parse_with_ip() {
        let output = "MOSH IP 192.168.1.100\nMOSH CONNECT 60002 TestKey12345678==\n";
        let info = parse_mosh_output(output).unwrap();
        assert_eq!(info.port, 60002);
        assert_eq!(info.key, "TestKey12345678==");
        assert_eq!(info.ip.as_deref(), Some("192.168.1.100"));
    }

    #[test]
    fn parse_missing_connect_fails() {
        let output = "some random output\n";
        assert!(parse_mosh_output(output).is_err());
    }
}
