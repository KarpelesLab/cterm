//! cterm-proto: Protobuf definitions and type conversions for the cterm gRPC protocol
//!
//! This crate contains the shared protocol definitions used by both ctermd (daemon)
//! and cterm (UI client) for communication over Unix sockets or SSH.

pub mod convert;

/// Generated protobuf and gRPC code
pub mod proto {
    // tonic generates every RPC method returning `Result<_, tonic::Status>`,
    // and `tonic::Status` is ~176 bytes, which trips `result_large_err`. This
    // is inherent to the generated code (the Err type can't be boxed here), so
    // allow it for the whole generated module.
    #![allow(clippy::result_large_err)]
    tonic::include_proto!("cterm.terminal");
}
