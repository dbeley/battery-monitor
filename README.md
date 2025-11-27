# Battery Monitor

Battery collector + graphing tools for Linux (tested on NixOS). Collects battery metrics from `/sys/class/power_supply`, stores them in SQLite, and provides quick plotting for the last hour/day/week/month.

## Features
- Collect energy/percentage/health for each battery detected in sysfs
- SQLite storage with optional CSV export via `sqlite3`
- CLI: `battery-monitor-collect` and `battery-monitor-graph`
- systemd service + timer for periodic sampling
- Nix flake for installation and dev shell

## Quick start (Nix)
```bash
nix run . -- collect --help
nix run . -- collect                  # one-shot collection
nix run . -- graph --period last_day --output battery.png
```

## Database location
- Default: `~/.local/share/battery-monitor/battery.db`
- Override via `--db PATH` or `BATTERY_MONITOR_DB` environment variable.

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
By default the service writes to `/var/lib/battery-monitor/battery.db` using the `BATTERY_MONITOR_DB` env var in the unit. Adjust as needed.

For a user service (no root), place the units in `~/.config/systemd/user/` and enable with `systemctl --user enable --now battery-monitor.timer`.

## CLI usage
```bash
# Collect once
battery-monitor-collect

# Collect repeatedly (60s interval)
battery-monitor-collect --interval 60

# Graph last day and save to png
battery-monitor-graph --period last_day --output ~/battery-day.png

# Graph last hour and show interactively
battery-monitor-graph --period last_hour --show
```

Supported periods: `last_hour`, `last_day`, `last_week`, `last_month`, `all`.

## Development
```bash
direnv allow                        # optional: auto-load dev shell (needs direnv + nix-direnv)
nix develop                         # drops you in a fish shell with dependencies
nix develop -c pytest               # run unit tests
nix develop -c pre-commit install   # install git hooks (ruff, typos)
nix develop -c pre-commit run --all-files
```
If you use fish, add `direnv hook fish | source` to your config so the direnv integration works.

## NixOS integration
- Add the flake as an input and include `battery-monitor.packages.${system}.default` in `environment.systemPackages`
- Point the systemd unit `ExecStart` to `${pkgs.battery-monitor}/bin/battery-monitor-collect` (or rely on `$PATH`)

## Notes
- Reads battery info from `/sys/class/power_supply/BAT*`
- If you have multiple batteries, each sample is stored with its sysfs path (`source_path` column)
- SQLite schema is defined in `battery_monitor/db.py`
