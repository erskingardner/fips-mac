#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
DEFAULT_FIPS_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)/fips
FIPS_DIR=${FIPS_CHECKOUT_PATH:-$DEFAULT_FIPS_DIR}

if [ ! -f "$FIPS_DIR/Cargo.toml" ]; then
  echo "FIPS checkout not found at $FIPS_DIR" >&2
  echo "Set FIPS_CHECKOUT_PATH to the sibling FIPS repository." >&2
  exit 1
fi

cargo test --manifest-path "$FIPS_DIR/Cargo.toml" --lib control::protocol::tests -- --nocapture
cargo test --manifest-path "$FIPS_DIR/Cargo.toml" --lib control::queries::tests::snapshot_show_status -- --nocapture
cargo test --manifest-path "$FIPS_DIR/Cargo.toml" --lib control::queries::tests::snapshot_show_peers -- --nocapture
cargo test --manifest-path "$FIPS_DIR/Cargo.toml" --lib control::queries::tests::snapshot_show_transports -- --nocapture
cargo check --manifest-path "$FIPS_DIR/Cargo.toml" --bin fips --bin fipsctl
