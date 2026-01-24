#!/bin/sh
set -e
cargo build --release
exec ./target/release/cterm
