#!/usr/bin/env sh
set -e
command -v cargo >/dev/null 2>&1 || { echo "Rust is not installed. Get it from https://rustup.rs"; exit 1; }
cargo build --release
cp target/release/sonar-lite ./SonarLite
echo "Done: ./SonarLite"
