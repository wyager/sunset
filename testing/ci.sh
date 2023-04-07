#!/bin/bash

set -v
set -e

# Set OFFLINE=1 to avoid rustup. cargo might still run offline.

if ! grep -sq '^name = "sunset"' Cargo.toml; then
    echo "Run ci.sh from the toplevel sunset directory"
    exit 2
fi

mkdir -p ci_out
OUT="$(realpath ci_out)"

# disabled for now, doesn't like unstable features
#export RUSTDOCFLAGS='-D warnings'

# dependencies
which cargo-bloat > /dev/null || cargo install cargo-bloat
if [ -z "$OFFLINE" ]; then
    rustup toolchain add nightly
fi

# stable - disabled now due to async fn in Behaviour
#cargo test
# build non-testing, will be no_std
#cargo build
cargo +nightly build
# nightly
cargo +nightly test

cargo +nightly doc
cargo +nightly test --doc

(
cd async
# only test lib since some examples are broken
cargo test --lib
cargo build --example sunsetc
)

(
cd embassy
cargo test
cargo test --doc
cargo doc
)

(
cd embassy/demos/std
cargo build
)

(
cd embassy/demos/picow
cargo build --release
cargo bloat --release -n 100 | tee "$OUT/picow-bloat.txt"
cargo bloat --release --crates | tee "$OUT/picow-bloat-crates.txt"
size target/thumbv6m-none-eabi/release/sunset-demo-embassy-picow | tee "$OUT/picow-size.txt"
)


# other checks

if [ $(find embassy -name rust-toolchain.toml -print0 | xargs -0 grep -h ^channel | uniq | wc -l) -ne 1 ]; then
    echo "rust-toolchain.toml has varying toolchains"
    find embassy -name rust-toolchain.toml -print0 | xargs -0 grep .
    exit 1
fi

echo success
