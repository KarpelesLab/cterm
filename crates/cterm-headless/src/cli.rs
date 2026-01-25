//! CLI argument parsing for ctermd

use crate::server::ServerConfig;
use clap::Parser;

/// ctermd - Headless terminal daemon with gRPC API
#[derive(Parser, Debug)]
#[command(name = "ctermd")]
#[command(about = "Headless terminal daemon with gRPC API")]
#[command(version)]
pub struct Cli {
    /// Unix socket path
    #[arg(short = 'l', long = "listen", default_value = "/tmp/ctermd.sock")]
    pub socket_path: String,

    /// Use TCP instead of Unix socket
    #[arg(long = "tcp")]
    pub use_tcp: bool,

    /// TCP port (only used with --tcp)
    #[arg(short = 'p', long = "port", default_value = "50051")]
    pub port: u16,

    /// TCP bind address (only used with --tcp)
    #[arg(long = "bind", default_value = "127.0.0.1")]
    pub bind_addr: String,

    /// Log level
    #[arg(long = "log-level", default_value = "info")]
    pub log_level: String,

    /// Run in foreground (don't daemonize)
    #[arg(short = 'f', long = "foreground")]
    pub foreground: bool,
}

impl Cli {
    /// Parse command-line arguments
    pub fn parse_args() -> Self {
        Cli::parse()
    }

    /// Convert CLI arguments to ServerConfig
    pub fn to_server_config(&self) -> ServerConfig {
        ServerConfig {
            use_tcp: self.use_tcp,
            bind_addr: self.bind_addr.clone(),
            port: self.port,
            socket_path: self.socket_path.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let cli = Cli::parse_from(["ctermd"]);
        assert_eq!(cli.socket_path, "/tmp/ctermd.sock");
        assert!(!cli.use_tcp);
        assert_eq!(cli.port, 50051);
        assert_eq!(cli.bind_addr, "127.0.0.1");
        assert_eq!(cli.log_level, "info");
        assert!(!cli.foreground);
    }

    #[test]
    fn test_tcp_mode() {
        let cli = Cli::parse_from(["ctermd", "--tcp", "-p", "8080"]);
        assert!(cli.use_tcp);
        assert_eq!(cli.port, 8080);
    }

    #[test]
    fn test_custom_socket() {
        let cli = Cli::parse_from(["ctermd", "-l", "/var/run/ctermd.sock"]);
        assert_eq!(cli.socket_path, "/var/run/ctermd.sock");
    }
}
