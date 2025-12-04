# Symmetri

Symmetri is a fast Rust collector + report/graph CLI for Linux (tested on NixOS). It tracks batteries plus broader system metrics and stores everything in SQLite for quick summaries over configurable hour/day/month windows (default: last 6 hours) or all history.

## Features
- Batteries: energy/percentage/health from `/sys/class/power_supply`
- CPU/GPU: usage %, current frequencies (best-effort per device)
- Network: rx/tx byte counters per interface
- Memory/disk: used/available bytes
- Thermal + power: thermal zone temperatures, hwmon power draw where exposed
- SQLite storage (bundled driver) with aggregate helpers and timeframe reports
- CLI binaries: `symmetri`, `symmetri-collect`, and `symmetri-report`
- PNG graphs rendered with Plotters; filenames auto-encode timeframe + timestamp + timezone
- Sample systemd service/timer for periodic sampling
- Nix flake for installation and a Rust dev shell

## Quick start (Nix)
```bash
nix run . -- collect --help
nix run . -- collect                  # one-shot collection
nix run . -- collect --interval 300   # keep sampling every 5 minutes
nix run . -- report --days 1 --graph  # render a table + save a graph
```

## Common Rust commands
```bash
cargo run -- collect --help      # run the CLI from source
cargo check                      # type-check without building release artifacts
cargo fmt                        # format the code
cargo clippy                     # lint
cargo test                       # run unit/integration tests
cargo build --release            # optimized binaries (target/release/)
cargo doc --open                 # browse documentation locally
```

## Database location
- Default: `~/.local/share/symmetri/metrics.db`
- Override via `--db PATH` or `SYMMETRI_DB` (legacy `BATTERY_MONITOR_DB` also works).

## systemd
Sample units are in `systemd/`:
- `symmetri.service`: runs one collection
- `symmetri.timer`: triggers every 5 minutes

Install (system-wide):
```bash
sudo cp systemd/symmetri.* /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now symmetri.timer
```
By default the service writes to `/var/lib/symmetri/metrics.db` via `SYMMETRI_DB`. Adjust as needed.

For a user service (no root), place the units in `~/.config/systemd/user/` and enable with `systemctl --user enable --now symmetri.timer`.

## CLI usage
```bash
# Collect once
symmetri-collect

# Collect repeatedly (60s interval)
symmetri-collect --interval 60

# Report last day and save graph with an auto-generated name in the cwd
symmetri-report --days 1 --graph

# Report last week and send the graph to a specific path
symmetri-report --days 7 --graph-path ~/battery-week.png
```

Use `--graph` to save a graph image with an informative filename in the current directory. Use `--graph-path` for a custom destination; without either flag the command prints only the textual report.

Timeframe controls:
- `--hours N` (default 6) when `--days/--months` are zero
- `--days N` overrides hours; `--months N` (~30 days each) overrides both
- `--all` shows the full history

## Development
```bash
direnv allow                      # optional: auto-load dev shell (needs direnv + nix-direnv)
nix develop                       # fish shell with Rust toolchain + pkg-config/fontconfig
nix develop -c cargo fmt          # format
nix develop -c cargo clippy       # lint
nix develop -c cargo test         # run unit tests
```

## NixOS integration
- Add the flake as an input and include `symmetri.packages.${system}.default` in `environment.systemPackages`.
- The systemd unit `ExecStart` can point to `${pkgs.symmetri}/bin/symmetri-collect` (or rely on `$PATH`).

## Notes
- Reads battery info from `/sys/class/power_supply/BAT*`
- If you have multiple batteries, each record is stored with its sysfs path (`source_path`) and reports aggregate the totals per collection
- Additional metrics are pulled from `/proc` + `/sys` (CPU/GPU load + clocks, network counters, memory/disk usage, thermal zones, hwmon power)
- SQLite schema and helpers live in `src/db.rs`
