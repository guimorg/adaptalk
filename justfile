default:
    @just --list

fmt:
    cargo fmt --all

check:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets

run:
    cargo run

