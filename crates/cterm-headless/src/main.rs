//! ctermd - Headless terminal daemon with gRPC API

use cterm_headless::cli::Cli;
use cterm_headless::server::run_server;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse CLI arguments
    let cli = Cli::parse_args();

    // Handle --print-socket-path: print and exit
    if cli.print_socket_path {
        let path = cterm_headless::cli::default_socket_path();
        println!("{}", path.display());
        return Ok(());
    }

    // Initialize logging
    let log_level = cli.log_level.parse().unwrap_or(log::LevelFilter::Info);
    env_logger::Builder::new()
        .filter_level(log_level)
        .format_timestamp_secs()
        .init();

    log::info!("ctermd starting...");

    let config = cli.to_server_config();

    // Daemonize if not running in foreground and not using TCP
    #[cfg(unix)]
    if !config.foreground && !config.use_tcp {
        daemonize()?;
    }

    // Run the server
    run_server(config).await?;

    Ok(())
}

/// Fork to background (Unix daemonization)
#[cfg(unix)]
fn daemonize() -> anyhow::Result<()> {
    use std::os::unix::io::AsRawFd;

    // Fork
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(anyhow::anyhow!("fork() failed"));
    }
    if pid > 0 {
        // Parent — exit immediately
        std::process::exit(0);
    }

    // Child — become session leader
    if unsafe { libc::setsid() } < 0 {
        return Err(anyhow::anyhow!("setsid() failed"));
    }

    // Redirect stdin/stdout/stderr to /dev/null
    if let Ok(devnull) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")
    {
        let fd = devnull.as_raw_fd();
        unsafe {
            libc::dup2(fd, 0);
            libc::dup2(fd, 1);
            libc::dup2(fd, 2);
        }
    }

    log::info!("Daemonized (pid={})", std::process::id());
    Ok(())
}
