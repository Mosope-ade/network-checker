# NetCheck

A local GUI ping tool for monitoring one or many networks simultaneously and keeping a persistent downtime log. The Rust process sends real ICMP probes; a browser provides the interface. No frontend build step, account, or external service is needed.

## Run

Install Rust with Cargo, then run from this directory:

```sh
cargo run
```

Open **http://127.0.0.1:8787**. Add one IPv4 or IPv6 address per line, optionally followed by a comma and a friendly name:

```text
192.168.1.1, Office router
10.0.0.1, Branch gateway
1.1.1.1, Internet reachability
::1, IPv6 loopback
```

Use a representative host address for each network. This tool probes from the machine running NetCheck using its routing table. It does not bind targets to separate interfaces, scan CIDR subnets, or resolve hostname targets. ICMP failure indicates no echo reply; it does not prove that all services on a network are down.

The GUI starts with no targets. Targets added through the GUI, lifetime packet counters, the last 60 probe results per target, and all confirmed incidents are saved in `data/netcheck.db`. The GUI displays the latest 1,000 incidents; **Export full CSV** includes the complete incident log. Removing a target retains its incidents. Browser timestamps use the browser's timezone; CSV timestamps use UTC.

Keep the Rust process running to monitor. You can close the browser and reopen it without stopping checks. Stop the process with Ctrl+C. A stopped computer, sleeping computer, or stopped process cannot monitor connectivity.

### Options

```sh
cargo run -- --port 9000 --database /path/to/netcheck.db
cargo run -- --help
cargo build --release
./target/release/netcheck
```

The UI and API bind only to IPv4 loopback (`127.0.0.1`). Cross-origin requests and unexpected Host headers are rejected. Run a single NetCheck process per database; concurrent processes sharing one database are unsupported. Back up the SQLite database while the process is stopped, or use SQLite's backup API while it is running.

### Thresholds and downtime

Each target has an independent task, check interval (1–3,600 seconds), failure threshold (1–20), and recovery threshold (1–20). An unreachable target cannot hold up checks for the others. Each probe times out after two seconds. A target never overlaps its own probes, so effective intervals can exceed the configured interval when probes time out. Missed interval ticks are skipped.

- **Responding:** the latest probe received a reply.
- **Confirming failure:** probes have failed, but the failure threshold has not been reached.
- **Down:** consecutive failures reached the failure threshold. One incident is opened, dated from the first failed probe in that sequence.
- **Confirming recovery:** a down target is replying, but the recovery threshold has not been reached. A failed probe resets this recovery sequence.
- **Recovered incident:** consecutive successful replies reached the recovery threshold. The incident ends at the first successful probe in that stable recovery sequence.
- **Paused:** no new probes are sent. Resuming starts a fresh threshold sequence and preserves counters and history.
- **Probe error:** an ICMP socket could not be opened. This is a local monitoring problem, not confirmed network downtime.

Packet loss is the percentage of attempted probes without a reply since the target was added. It includes transient failures even if they never become confirmed outages. It is not an uptime percentage. A lost probe is shown as a short red mark in the last-60-probes strip; successful bar heights show relative latency within that strip.

Pausing, removing a target, or encountering a local socket error closes any open incident as **interrupted**, never as recovered. At restart, an open incident is interrupted at its last recorded probe; the period when NetCheck was not running is excluded. Interrupted duration means the observed outage interval ended without confirmed recovery.

### ICMP permissions on Linux

NetCheck first attempts a datagram ICMP socket, which works without root where the kernel's `net.ipv4.ping_group_range` allows the user's group. If your system denies ICMP sockets, the GUI displays a probe error. On Linux, you can grant only the network capability to a built binary:

```sh
cargo build --release
sudo setcap cap_net_raw+ep target/release/netcheck
./target/release/netcheck
```

Rebuilding the binary removes that capability. The GUI server itself does not require root.

## Original diagnostic mode

The original gateway → Internet → DNS console monitor remains available:

```sh
cp config/monitor.example.toml config/monitor.toml
# Edit config/monitor.toml: set gateway.address, internet.targets, and dns.hostname.
cargo run -- --daemon --config config/monitor.toml
```

The shipped example contains no selected targets and cannot start monitoring until you configure it. Machine-specific `config/*.toml` files are ignored by Git; only the example is tracked. Do not force-add local target configuration or notification credentials.

This mode reads the legacy TOML configuration and logs hierarchical checks and local gateway diagnostics. Its configuration is separate from GUI targets. The alert fields in that TOML belong to the earlier roadmap; desktop and Telegram notification delivery are not implemented. The GUI's SQLite incident logging is used in GUI mode.

`plan.md` is the original, locally supplied daemon roadmap. The implemented GUI scope and behavior are documented here and in `PROGRESS.md`.

## Verification

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo build
python3 tests/smoke.py
```

The smoke test launches an isolated server and temporary database. It checks real IPv4/IPv6 loopback probes, independent failure detection for a documentation-only unreachable address, pause/resume, persistence across process restart, CSV export, validation, and local-origin restrictions. It requires working ICMP socket permissions and assumes `192.0.2.1` does not respond in your network.

Optional browser checks:

```sh
npm ci
npx playwright install chromium
npm run test:browser
```

To use an installed Chrome instead of downloading Chromium:

```sh
CHROME_PATH=/usr/bin/google-chrome npm run test:browser
```

The browser test launches its own temporary server and verifies the real UI at 320, 768, 1,024, and 1,440 pixels. Screenshots are written to the system temporary directory (`/tmp` on Linux). Node.js and npm are only needed for these optional tests.
