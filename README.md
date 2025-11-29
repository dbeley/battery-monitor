# Battery Monitor (Rust)

Battery collector + graph/report tools for Linux (tested on NixOS). Collects battery metrics from `/sys/class/power_supply`, stores the records in SQLite, and prints tables/graphs for configurable hour/day/month windows (default: last 6 hours) or all history. The project is now implemented in Rust for faster startup and lower runtime overhead compared to the original Python version.

## Features
- Collect energy/percentage/health for each battery detected in sysfs
- SQLite storage (bundled driver) with quick aggregate helpers
- CLI binaries: `battery-monitor`, `battery-monitor-collect`, and `battery-monitor-report`
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
- Default: `~/.local/share/battery-monitor/battery.db`
- Override via `--db PATH` or `BATTERY_MONITOR_DB`.

## systemd
Sample units are in `systemd/`:
- `battery-monitor.service`: runs one collection
- `battery-monitor.timer`: triggers every 5 minutes

Install (system-wide):
```bash
sudo cp systemd/battery-monitor.* /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now battery-monitor.timer
```
By default the service writes to `/var/lib/battery-monitor/battery.db` via `BATTERY_MONITOR_DB`. Adjust as needed.

For a user service (no root), place the units in `~/.config/systemd/user/` and enable with `systemctl --user enable --now battery-monitor.timer`.

## CLI usage
```bash
# Collect once
battery-monitor-collect

# Collect repeatedly (60s interval)
battery-monitor-collect --interval 60

# Report last day and save graph with an auto-generated name in the cwd
battery-monitor-report --days 1 --graph

# Report last week and send the graph to a specific path
battery-monitor-report --days 7 --graph-path ~/battery-week.png
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
- Add the flake as an input and include `battery-monitor.packages.${system}.default` in `environment.systemPackages`.
- The systemd unit `ExecStart` can point to `${pkgs.battery-monitor}/bin/battery-monitor-collect` (or rely on `$PATH`).

## Notes
- Reads battery info from `/sys/class/power_supply/BAT*`
- If you have multiple batteries, each record is stored with its sysfs path (`source_path`) and reports aggregate the totals per collection
- SQLite schema and helpers live in `src/db.rs`
