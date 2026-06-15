# ZK Rollup Portfolio

This repository is a Rust-first blockchain infrastructure portfolio. It starts
with wallet and Merkle primitives, then grows into indexing, rollup simulation,
analytics APIs, and a dashboard.

## Workspace

```text
crates/
  wallet-core/      Reusable signing, verification, and address logic
apps/
  wallet-cli/       CLI wrapper around wallet-core
docs/
  phase-1-wallet.md
```

## Phase 1: Wallet CLI

Create a development keypair:

```bash
cargo run -p wallet-cli -- create
```

Create and save a plaintext development keypair:

```bash
cargo run -p wallet-cli -- create --output dev-keypair.json
```

Load and validate a saved development keypair:

```bash
cargo run -p wallet-cli -- load --key-file dev-keypair.json
```

Import a keypair from an existing secret key:

```bash
cargo run -p wallet-cli -- import \
  --secret-key "<base64-secret-key>" \
  --output imported-keypair.json
```

Derive a public key from a secret key:

```bash
cargo run -p wallet-cli -- public-key --secret-key "<base64-secret-key>"
```

Derive a public key from a saved key file:

```bash
cargo run -p wallet-cli -- public-key --key-file dev-keypair.json
```

Derive an address from a public key:

```bash
cargo run -p wallet-cli -- address --public-key "<base64-public-key>"
```

Derive an address from a saved key file:

```bash
cargo run -p wallet-cli -- address --key-file dev-keypair.json
```

Sign a message:

```bash
cargo run -p wallet-cli -- sign \
  --secret-key "<base64-secret-key>" \
  --message "hello rollup"
```

Sign a message with a saved key file:

```bash
cargo run -p wallet-cli -- sign \
  --key-file dev-keypair.json \
  --message "hello rollup"
```

Verify a signature:

```bash
cargo run -p wallet-cli -- verify \
  --public-key "<base64-public-key>" \
  --message "hello rollup" \
  --signature "<base64-signature>"
```

Run tests:

```bash
cargo test
```
