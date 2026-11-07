# Quickstart: Application Foundation (Phase 1)

## 1. Toolchain

```bash
rustup show     # picks up rust-toolchain.toml (1.99.1 + rustfmt + clippy)
```

## 2. Deterministic run (no external services — works on any machine)

```bash
cp .env.example .env          # fill DATABASE_URL/VALKEY_URL if you have real services
cargo test --workspace        # uses MemoryStorage for storage-dependent tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
bash .github/scripts/check-boundaries.sh
```

This is the **doloop verification command** (constitution, Development Workflow):
`cargo test -p <crate> && cargo clippy -p <crate> --all-targets -- -D warnings`

## 3. Full environment (MSSQL + Valkey)

```bash
cp .env.example .env          # set MSSQL_SA_PASSWORD / DATABASE_URL / VALKEY_URL
docker compose up -d          # mssql + valkey
cargo run -p host             # binds :5000, runs migrations, serves
curl -s localhost:5000/healthz
```

The full-environment integration suite (`tests/`) runs only when `DATABASE_URL`
points at a live database (otherwise it is skipped, not faked — Principle XVII).

## 4. Health check

```bash
curl -s localhost:5000/healthz
# {"status":"ok","process":"alive","database":"reachable","session_store":"reachable"}
```

## 5. Machine notes

- **Linux dev machine**: no docker daemon → use the deterministic run (§2); full
  environment verification happens on the Windows machine or in CI (Principle XVI).
- **Windows dev machine**: git-bash runs all scripts; cargo/cargo-fmt/cargo-clippy as
  above. If the docker daemon is stopped, start Docker Desktop before §3.
