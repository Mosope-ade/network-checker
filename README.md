# NetCheck

A local GUI for pinging one or multiple network endpoints simultaneously and logging downtime. **Every monitoring address comes from the user interface.** A new database starts empty. There are no preset addresses, fallback hosts, environment-provided monitoring targets, or configuration-file monitoring mode.

## Run

Install Rust 1.89 or newer, then run:

```sh
cargo run
```

Open the local URL printed in the terminal (normally **http://localhost:8787**), select **Add targets**, and enter your own IPv4 or IPv6 addresses, one per line. Optionally follow an address with a comma and a friendly name. Select the interval and failure/recovery thresholds, then start monitoring. No address is prefilled.

Use a representative unicast host IP for each network. The tool uses the running machine's routing table; it does not scan subnets, bind separate egress interfaces, or accept hostnames. An absent ICMP reply does not establish that every service on that network is down.

## Data and operation

Targets entered through the UI, lifetime packet counters, the latest 60 probe results per target, and all downtime incidents are stored in `data/netcheck.db`. Restarting resumes the user's saved enabled targets; paused targets stay paused. Removing a target retains its incidents. The log prioritizes ongoing incidents, then shows recent history, up to 1,000 records. **Export full CSV** includes all incidents. Browser dates use the browser's timezone; CSV dates use UTC.

Each target has an independent task. One slow or unreachable target cannot delay the others. The interval is 1–3,600 seconds; thresholds are 1–20 consecutive failures or successes. Probes time out after two seconds and never overlap for the same target, so its actual sampling interval can exceed the configured interval during timeouts.

- **Responding:** the latest probe received a reply.
- **Confirming failure:** failures have not yet reached the threshold.
- **Down:** the failure threshold has been reached. The incident starts at the first failed probe in that sequence.
- **Confirming recovery:** the target is replying but has not reached the recovery threshold. Another failure resets that sequence.
- **Recovered:** stable replies reached the recovery threshold. The incident ends at the first successful probe in that stable sequence.
- **Paused:** no new probes. Resuming preserves history and counters and starts fresh threshold sequences.
- **Probe error:** local ICMP socket or permission failure; this does not count as network downtime.
- **Monitoring delayed:** no recent recorded result; the UI does not present old results as current responses.

Packet loss counts all attempted probes without replies, including transient failures. It is not an uptime percentage. The probe strip distinguishes lost probes from latency measurements.

Keep the process and computer running to monitor. Closing the browser does not stop checks. Ctrl+C and, on Unix, SIGTERM stop monitoring and interrupt open incidents at their last observed probe. Restarting also closes incidents left by a crash. A long sampling gap or backward clock jump starts a fresh threshold sequence; unobserved time is excluded from continuous downtime. These remain sampled measurements, not proof of what happened between probes.

## Options

```sh
cargo run -- --port 9000 --database /path/to/netcheck.db
cargo run -- --help
cargo build --release
./target/release/netcheck
```

Only the web port and local database path are CLI options. The retired `--daemon` and `--config` flags return an error directing the user to the UI. Existing legacy TOML files are not read or imported. Existing GUI database targets are preserved.

The server binds to the operating system's IPv4 loopback constant. This is the local application listener, never a ping target. Expected Host and same-origin checks restrict access to the local browser application; it has no remote/multi-user mode. Database ownership is exclusive, including through file aliases, so a second NetCheck process cannot change an active monitor's state. Back up SQLite with its backup API or while NetCheck is stopped.

SQLite work runs on blocking workers rather than the asynchronous probe runtime. CSV exports currently materialize in memory and hold the storage mutex while reading; very large logs can temporarily delay writes. Export during quiet periods for unusually large histories. Network filesystems are not supported for the database owner lock.

## ICMP permissions

NetCheck first attempts datagram ICMP sockets. On Linux these work without root if the kernel's ping-group setting allows the current user's group. Permission failures are displayed as probe errors. If necessary, grant only the network capability to a built binary:

```sh
cargo build --release
sudo setcap cap_net_raw+ep target/release/netcheck
./target/release/netcheck
```

Rebuilding removes that capability. The web server does not require root.

## Verification

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
cargo build --locked
python3 tests/hygiene.py
python3 tests/smoke.py
```

The hygiene tests guard against direct IP literals and alternative target-config paths. The smoke test always checks an empty initial target list and rejection of retired flags. Set `NETCHECK_TEST_UNREACHABLE` to a host IP you have selected and know will not reply to enable its complete real-ping workflow. Test loopback addresses are resolved from the operating system, not embedded. The complete workflow checks IPv4/IPv6, independent failed/healthy probes, pause/resume idempotency, validation, Host/Origin protection, process ownership, SIGTERM, restarts, and CSV.

Optional browser checks require Node.js 20+:

```sh
npm ci
npx playwright install chromium
# Set NETCHECK_TEST_UNREACHABLE to your chosen nonresponding test host first.
npm run test:browser
```

To use an installed Chrome, set `CHROME_PATH` to its executable path. Browser checks use temporary storage and test bulk entry, actual probes, XSS resistance, delayed actions during polling, error retention, disconnect/reconnect, CSV, dialogs, keyboard navigation, and layouts at 320/768/1024/1440 pixels. Screenshots are written to the system temporary directory. Chrome verification does not imply Firefox or Safari verification.
