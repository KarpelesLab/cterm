//! File descriptor passing via Unix domain sockets using SCM_RIGHTS
//!
//! This module provides utilities for sending and receiving file descriptors
//! between processes using Unix domain sockets with the SCM_RIGHTS control message.

use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;

/// Calculate the size needed for control message buffer for `n` file descriptors
fn cmsg_space(n: usize) -> usize {
    // CMSG_SPACE(n * sizeof(int))
    unsafe { libc::CMSG_SPACE((n * std::mem::size_of::<RawFd>()) as u32) as usize }
}

/// Send file descriptors over a Unix domain socket using SCM_RIGHTS
///
/// # Arguments
/// * `socket` - The Unix domain socket to send over
/// * `fds` - Slice of file descriptors to send
/// * `data` - Additional data to send alongside the file descriptors
///
/// # Returns
/// * `Ok(())` on success
/// * `Err(io::Error)` on failure
pub fn send_fds(socket: &UnixStream, fds: &[RawFd], data: &[u8]) -> io::Result<()> {
    if fds.is_empty() {
        // No FDs to send, just send the data
        return send_data_only(socket, data);
    }

    let fd_bytes = std::mem::size_of_val(fds);
    let cmsg_buffer_len = cmsg_space(fds.len());

    // Allocate control message buffer (aligned)
    let mut cmsg_buffer = vec![0u8; cmsg_buffer_len];

    // Set up the iovec for the data
    let mut iov = libc::iovec {
        iov_base: if data.is_empty() {
            // Must have at least 1 byte of data for sendmsg to work
            &mut [0u8; 1] as *mut _ as *mut libc::c_void
        } else {
            data.as_ptr() as *mut libc::c_void
        },
        iov_len: if data.is_empty() { 1 } else { data.len() },
    };

    // Set up the msghdr
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buffer.as_mut_ptr() as *mut libc::c_void;
    // Cast needed: msg_controllen is usize on Linux, u32 on macOS
    msg.msg_controllen = cmsg_buffer_len as _;

    // Set up the control message header
    let cmsg: *mut libc::cmsghdr = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    if cmsg.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CMSG_FIRSTHDR returned null",
        ));
    }

    unsafe {
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        // Cast needed: cmsg_len is usize on Linux, u32 on macOS
        (*cmsg).cmsg_len = libc::CMSG_LEN(fd_bytes as u32) as _;

        // Copy file descriptors into the control message data
        let cmsg_data = libc::CMSG_DATA(cmsg);
        std::ptr::copy_nonoverlapping(fds.as_ptr(), cmsg_data as *mut RawFd, fds.len());
    }

    // Send the message
    let ret = unsafe { libc::sendmsg(socket.as_raw_fd(), &msg, 0) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Send data without file descriptors
fn send_data_only(socket: &UnixStream, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut socket_ref = socket;
    socket_ref.write_all(data)?;
    Ok(())
}

/// Receive file descriptors from a Unix domain socket using SCM_RIGHTS
///
/// # Arguments
/// * `socket` - The Unix domain socket to receive from
/// * `max_fds` - Maximum number of file descriptors expected
/// * `buf` - Buffer to receive data into
///
/// # Returns
/// * `Ok((fds, data_len))` - Vector of received file descriptors and length of data received
/// * `Err(io::Error)` on failure
pub fn recv_fds(
    socket: &UnixStream,
    max_fds: usize,
    buf: &mut [u8],
) -> io::Result<(Vec<RawFd>, usize)> {
    let cmsg_buffer_len = cmsg_space(max_fds);

    // Allocate control message buffer (aligned)
    let mut cmsg_buffer = vec![0u8; cmsg_buffer_len];

    // Set up the iovec for receiving data
    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr() as *mut libc::c_void,
        iov_len: buf.len(),
    };

    // Set up the msghdr
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = cmsg_buffer.as_mut_ptr() as *mut libc::c_void;
    // Cast needed: msg_controllen is usize on Linux, u32 on macOS
    msg.msg_controllen = cmsg_buffer_len as _;

    // Receive the message
    let ret = unsafe { libc::recvmsg(socket.as_raw_fd(), &mut msg, 0) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }

    let data_len = ret as usize;
    let mut fds = Vec::new();

    // Parse control messages to extract file descriptors
    let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    while !cmsg.is_null() {
        unsafe {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                // Calculate number of file descriptors in this control message
                // Cast needed: cmsg_len is usize on Linux, u32 on macOS
                let fd_bytes = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let num_fds = fd_bytes / std::mem::size_of::<RawFd>();

                // Extract file descriptors
                let cmsg_data = libc::CMSG_DATA(cmsg) as *const RawFd;
                for i in 0..num_fds {
                    fds.push(*cmsg_data.add(i));
                }
            }
            cmsg = libc::CMSG_NXTHDR(&msg, cmsg);
        }
    }

    Ok((fds, data_len))
}

/// Close multiple file descriptors, ignoring errors
pub fn close_fds(fds: &[RawFd]) {
    for &fd in fds {
        unsafe {
            libc::close(fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[test]
    fn test_send_recv_fds() {
        // Create a socket pair
        let (sender, receiver) = UnixStream::pair().expect("Failed to create socket pair");

        // Create a pipe to get file descriptors
        let mut pipe_fds = [0i32; 2];
        unsafe {
            assert_eq!(libc::pipe(pipe_fds.as_mut_ptr()), 0);
        }

        // Send the pipe read end
        let fds_to_send = vec![pipe_fds[0]];
        let data = b"hello";
        send_fds(&sender, &fds_to_send, data).expect("Failed to send FDs");

        // Receive
        let mut buf = [0u8; 1024];
        let (received_fds, data_len) =
            recv_fds(&receiver, 8, &mut buf).expect("Failed to receive FDs");

        assert_eq!(data_len, 5);
        assert_eq!(&buf[..data_len], b"hello");
        assert_eq!(received_fds.len(), 1);

        // Verify the received FD works by writing to the pipe write end and reading from received
        let test_data = b"test data";
        unsafe {
            libc::write(
                pipe_fds[1],
                test_data.as_ptr() as *const libc::c_void,
                test_data.len(),
            );

            let mut read_buf = [0u8; 64];
            let n = libc::read(
                received_fds[0],
                read_buf.as_mut_ptr() as *mut libc::c_void,
                read_buf.len(),
            );
            assert_eq!(n as usize, test_data.len());
            assert_eq!(&read_buf[..n as usize], test_data);
        }

        // Clean up
        close_fds(&[pipe_fds[0], pipe_fds[1]]);
        close_fds(&received_fds);
    }

    #[test]
    fn test_send_recv_multiple_fds() {
        let (sender, receiver) = UnixStream::pair().expect("Failed to create socket pair");

        // Create multiple pipes
        let mut pipe1 = [0i32; 2];
        let mut pipe2 = [0i32; 2];
        unsafe {
            assert_eq!(libc::pipe(pipe1.as_mut_ptr()), 0);
            assert_eq!(libc::pipe(pipe2.as_mut_ptr()), 0);
        }

        // Send multiple FDs
        let fds_to_send = vec![pipe1[0], pipe2[0]];
        send_fds(&sender, &fds_to_send, b"multi").expect("Failed to send");

        // Receive
        let mut buf = [0u8; 1024];
        let (received_fds, data_len) = recv_fds(&receiver, 8, &mut buf).expect("Failed to receive");

        assert_eq!(data_len, 5);
        assert_eq!(received_fds.len(), 2);

        // Clean up
        close_fds(&[pipe1[0], pipe1[1], pipe2[0], pipe2[1]]);
        close_fds(&received_fds);
    }

    #[test]
    fn test_send_recv_data_only() {
        let (sender, receiver) = UnixStream::pair().expect("Failed to create socket pair");

        // Send without FDs
        send_fds(&sender, &[], b"data only").expect("Failed to send");

        // Receive
        let mut buf = [0u8; 1024];
        let (received_fds, data_len) = recv_fds(&receiver, 8, &mut buf).expect("Failed to receive");

        assert_eq!(data_len, 9);
        assert_eq!(&buf[..data_len], b"data only");
        assert!(received_fds.is_empty());
    }
}
