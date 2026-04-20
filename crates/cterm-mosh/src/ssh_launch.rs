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
/// Runs: `ssh host "mosh-server new -s -c 256 -l LANG=... -l TERM=..."`
/// Parses stdout for `MOSH CONNECT <port> <key>` and optional `MOSH IP <addr>`.
pub async fn launch_mosh_server(
    host: &str,
    locale: Option<&str>,
    term: Option<&str>,
    extra_ssh_args: &[String],
) -> Result<MoshConnectInfo, MoshError> {
    let mut cmd = tokio::process::Command::new("ssh");

    for arg in extra_ssh_args {
        cmd.arg(arg);
    }

    cmd.arg("-tt"); // Force PTY allocation (triggers PAM/MOTD like the mobile app)
    cmd.arg(host);
    cmd.arg(build_mosh_server_cmd(locale, term));

    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    log::debug!("launching mosh-server via SSH on {}", host);

    let output = run_ssh_command(&mut cmd).await?;
    let combined = format!("{}{}", output.0, output.1);
    parse_mosh_output(&combined)
}

/// Build the mosh-server command string.
fn build_mosh_server_cmd(locale: Option<&str>, term: Option<&str>) -> String {
    let mut mosh_cmd = String::from("mosh-server new -s -c 256");
    if let Some(loc) = locale {
        mosh_cmd.push_str(&format!(" -l LANG={}", loc));
    }
    if let Some(t) = term {
        mosh_cmd.push_str(&format!(" -l TERM={}", t));
    }
    mosh_cmd
}

/// Run an SSH command and return (stdout, stderr).
async fn run_ssh_command(cmd: &mut tokio::process::Command) -> Result<(String, String), MoshError> {
    let output = cmd
        .output()
        .await
        .map_err(|e| MoshError::SshFailed(e.to_string()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(MoshError::SshFailed(format!(
            "SSH exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    Ok((stdout, stderr))
}

/// Parse mosh-server stdout for connection info.
fn parse_mosh_output(output: &str) -> Result<MoshConnectInfo, MoshError> {
    let mut port = None;
    let mut key = None;
    let mut ip = None;

    for line in output.lines() {
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

    #[test]
    fn build_mosh_cmd_full() {
        let cmd = build_mosh_server_cmd(Some("en_US.UTF-8"), Some("xterm-256color"));
        assert_eq!(
            cmd,
            "mosh-server new -s -c 256 -l LANG=en_US.UTF-8 -l TERM=xterm-256color"
        );
    }

    #[test]
    fn build_mosh_cmd_no_options() {
        let cmd = build_mosh_server_cmd(None, None);
        assert_eq!(cmd, "mosh-server new -s -c 256");
    }
}
