# Repository Guidelines

## Project Structure & Module Organization
- `src/`: Rust sources. `cli.rs` (CLI args/reporting), `collector.rs` (collection entry), `db.rs` (SQLite schema/helpers), `sysfs.rs` (battery reads), `metrics.rs` (CPU/GPU/net/memory/disk/thermal/power collectors), `graph.rs` (plotting), `aggregate.rs` (battery aggregation).
- `src/bin/`: wrapper binaries `symmetri-collect.rs` and `symmetri-report.rs`.
- `systemd/`: sample service/timer units for periodic collection.
- `battery_*.png` / `symmetri_*.png`: generated report artifacts; safe to delete and excluded from tests.

## Build, Test, and Development Commands
- `nix develop`: enter the dev shell (fish) with Rust toolchain, pkg-config, fontconfig, pre-commit.
- `nix run . -- collect --help`: show CLI help; swap `collect` or `report` to run.
- `cargo test` or `nix develop -c cargo test`: run tests.
- `nix develop -c pre-commit run --all-files`: lint/spell-check before commits.
- `symmetri-report --days 1 --graph`: smoke test reporting when a DB exists.

## Coding Style & Naming Conventions
- Rust 2021 edition; 4-space indentation; prefer clear naming and small modules.
- Patterns: CLI in `cli.rs`, IO in `collector.rs`/`sysfs.rs`/`metrics.rs`, DB access in `db.rs`, rendering in `graph.rs`.
- Naming: snake_case for code, lowercase-dash for CLI flags, safe filenames via `default_graph_path`.
- Run `cargo fmt`, `cargo clippy`, and `typos` (pre-commit) to keep formatting and spelling consistent.

## Testing Guidelines
- Tests live alongside code in `src/*.rs` (unit tests).
- Prefer mocking sysfs/proc data in tests; avoid relying on host hardware.
- Add regression tests for new metrics, schema changes, timeframe logic, or CLI flags; cover edge cases (no samples, multiple batteries).
- Capture expected CLI snippets with integration-style tests using `assert_cmd` when behavior is guarded.

## Commit & Pull Request Guidelines
- Use short, imperative subjects (e.g., “Simplify image output CLI”); add body for rationale when helpful.
- Keep commits cohesive (code + tests + docs) and ensure pre-commit passes.
- PRs: describe scope, manual test notes (commands run, outputs/paths), linked issues, and graph screenshots when behavior changes.

## Configuration & Deployment Notes
- Default DB: `~/.local/share/symmetri/metrics.db`; override with `--db`, `SYMMETRI_DB` (legacy `BATTERY_MONITOR_DB` supported).
- Sample systemd units run collection every 5 minutes; adjust paths/env vars before installing to `/etc/systemd/system/` or `~/.config/systemd/user/`.
- Graphs default to the current directory; use `--graph-path` when scripting to avoid clutter.
