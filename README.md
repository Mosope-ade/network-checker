# NetCheck

[![Windows package](https://github.com/Mosope-ade/network-checker/actions/workflows/windows-package.yml/badge.svg)](https://github.com/Mosope-ade/network-checker/actions/workflows/windows-package.yml)

**Ping multiple network endpoints at once and keep a readable record of downtime.**

NetCheck runs on your computer and provides a local browser interface. Add the hosts you want to monitor, watch their latency and packet loss, and see exactly what happened when a connection failed or recovered. Targets and history are saved locally in SQLite.

[Download a Windows build](https://github.com/Mosope-ade/network-checker/actions/workflows/windows-package.yml) · [Report an issue](https://github.com/Mosope-ade/network-checker/issues)

## Features

- Monitor up to 128 IPv4 or IPv6 targets independently and simultaneously.
- Add multiple named targets in one submission.
- View live status, latency, packet loss, and the latest 60 probe results per target.
- Confirm failures and recoveries using consecutive-check thresholds.
- Pause, resume, or remove individual targets.
- Read a searchable activity log with full dates, times, timezone, and event descriptions.
- Review outage start/end times and observed downtime durations.
- Export complete activity and downtime histories as CSV.
- Resume saved monitoring targets after restarting the app.

All monitoring addresses are entered through the interface. A fresh installation starts with an empty target list.

## Get started on Windows

The portable Windows x64 package contains `netcheck.exe`, a launcher, instructions, build information, and SHA-256 checksums. Rust, Node.js, and a separate SQLite installation are not required to run it.

1. Open [Windows package builds](https://github.com/Mosope-ade/network-checker/actions/workflows/windows-package.yml).
2. Select a **successful** run and download **NetCheck-windows-x64** under **Artifacts**. GitHub may require you to sign in.
3. Extract the downloaded artifact. Inside it, extract `NetCheck-windows-x64.zip` to a folder of your choice.
4. Run **Start-NetCheck.cmd** from the extracted application folder.
5. Open **http://localhost:8787** in your browser.

Keep the console window open while monitoring. Press **Ctrl+C** to stop NetCheck. Closing the browser leaves monitoring running.

The package is an unsigned portable application, not an installer or a Windows service. Build artifacts are retained for 30 days.

### Update the Windows app

Stop NetCheck, download a newer successful build, and replace the extracted application files. The launcher stores targets and history separately in:

```text
%LOCALAPPDATA%\NetCheck\netcheck.db
```

Keep this data folder when updating. The `.owner.lock` file alongside the database is used to prevent another NetCheck instance from opening the same database location; leave it in place while the app is running.

## Monitor your networks

1. Select **Add targets**.
2. Enter one host address per line. To give a target a name, follow its address with a comma and the name.
3. Choose the check interval, failure threshold, and recovery threshold.
4. Select **Start monitoring**.

Use one representative host address for each network you want to watch. Each target runs on its own schedule, so an unreachable host does not hold up the other targets.

The **Targets** view shows current status, the last latency, lifetime packet loss, reply counts, and recent probes. Use **Pause**, **Resume**, and **Remove** to manage each target. Removing a target retains its historical activity and outages.

### Understand the statuses

- **Responding:** the latest probe received a reply.
- **Confirming failure:** a probe failed; NetCheck is waiting for enough consecutive failures to confirm an outage.
- **Down:** the failure threshold was reached and an outage was recorded.
- **Confirming recovery:** replies are returning; NetCheck is waiting for enough consecutive successes to confirm recovery.
- **Paused:** checks are stopped for this target.
- **Probe error:** a local permission or socket problem prevented the check from running. This does not count as confirmed network downtime.
- **Monitoring delayed:** no recent probe result was recorded. Old results are not presented as current responses.

Check intervals range from 1 to 3,600 seconds. Failure and recovery thresholds range from 1 to 20 checks. Each probe times out after two seconds. A target's probes never overlap, so timeouts can make its actual sampling interval longer than the selected interval.

Packet loss measures unanswered probes since the target was added. It includes transient failures and is **not an uptime percentage**.

## Read activity and downtime

### Activity

The **Activity** tab answers **what happened, to which target, and when**. Entries show the full date, a 24-hour time with seconds, the target name/address, and a plain-language explanation. The timezone is displayed above the log.

Events include failed pings, confirmed outages, returning replies, recoveries, pauses, resumes, target removal, local probe errors, monitoring gaps, clock changes, service stops, and restarts. Search by target, event, or displayed time. The latest three events also appear beneath the live targets.

### Downtime

The **Downtime** tab summarizes each outage: when replies were first lost, when they returned or monitoring stopped, the observed duration, and the reason.

An outage is confirmed only after its failure threshold is reached, but its start is dated from the **first failed probe** in that sequence. Recovery is confirmed after its success threshold is reached, with downtime ending at the **first reply in that stable recovery sequence**. The activity log distinguishes these observation and confirmation times.

**Monitoring stopped** means recovery was not confirmed before monitoring was interrupted. Pauses, shutdowns, restarts, long sampling gaps, and backward clock changes interrupt the observed outage; unobserved time is excluded. Existing older incidents appear as clearly labelled historical records.

Both views display up to 1,000 records. Downtime prioritizes ongoing incidents; activity is shown latest first. Their CSV exports include the **complete saved history**, with UTC timestamps. Exports stream without blocking the monitor's database mutex.

## Storage and backups

The Windows launcher uses `%LOCALAPPDATA%\NetCheck\netcheck.db`. When the executable is run directly, or through Cargo, the default is `data/netcheck.db` relative to the working directory. Use `--database` to select another location.

The database retains targets, counters, recent probe results, activity, and incidents. Enabled targets resume after restart; paused targets remain paused.

Stop NetCheck before copying the database for a backup. Use a local filesystem and one database location per running instance. Windows uses a canonical-path ownership sidecar; do not access the same database through separately named hard links. Unix uses an advisory lock on the database inode.

## Troubleshooting

### The browser cannot connect

Check that NetCheck is running and open the local URL printed in its console. If you selected another port, use that port in the browser. Connection failures are shown separately from storage failures; displayed results may be stale until the connection returns.

### Windows reports that local ping probes are unavailable

Close NetCheck, then right-click **Start-NetCheck.cmd** and choose **Run as administrator**. Windows policies can require elevation for raw ICMP sockets. If checks still fail, review the host's firewall and network policy.

### Linux reports an ICMP permission error

NetCheck first attempts datagram ICMP sockets. Whether these work without elevation depends on the system's ping-group permissions. If necessary, grant the network capability to the built binary:

```sh
cargo +1.95.0 build --release --locked
sudo setcap cap_net_raw+ep target/release/netcheck
./target/release/netcheck
```

Rebuilding the executable removes that capability.

### The port or database is already in use

Close the other instance or choose a different port or database path. Run `netcheck.exe --help` on Windows, or `./netcheck --help` on Linux, to see the options.

### A request timed out or could not be saved

A timed-out change may already have been saved. Check the target list before trying again. Pause/resume requests set an explicit state, so retrying the same request cannot accidentally toggle it back.

For storage errors, follow the displayed message and check disk space, folder permissions, or another program holding the database open. Preserve existing data before attempting repairs. Detailed diagnostics are available by starting NetCheck with `RUST_LOG=debug` set in your environment.

## Scope

NetCheck probes from the computer on which it runs, using that computer's routing table. It accepts individual unicast host addresses; it does not scan CIDR ranges, resolve hostname targets, bind targets to separate egress interfaces, or measure bandwidth.

An unanswered ping can mean the host or route is unavailable, or that ICMP is blocked. It does not establish that every service on a network is down. Monitoring also requires the computer to remain awake and the app to keep running.

The web interface binds to local IPv4 loopback and checks request origins. This is a single-machine tool; remote access, multiple user accounts, desktop/Telegram alerts, and an installed background service are not implemented.

## Build from source

Requirements: Rust and Cargo, plus a native C compiler for bundled SQLite. Windows source builds use the MSVC toolchain and C++ build tools. CI uses Rust **1.95.0**.

```sh
git clone https://github.com/Mosope-ade/network-checker.git
cd network-checker
rustup toolchain install 1.95.0 --profile minimal --component rustfmt --component clippy
cargo +1.95.0 run --locked
```

Open the local URL printed in the terminal and add targets through the interface.

To build an optimized executable:

```sh
cargo +1.95.0 build --release --locked
```

The result is `target/release/netcheck` on Linux or `target/release/netcheck.exe` on Windows.

### Runtime options

```text
netcheck --port 9000 --database path/to/netcheck.db
netcheck --help
```

The port defaults to `8787`. Setting it to `0` selects an available port; the app prints the resulting URL. Only the web port and database path are command-line settings. Monitoring addresses and thresholds are supplied through the UI.

### Checks and tests

```sh
cargo +1.95.0 fmt --check
cargo +1.95.0 clippy --locked --all-targets -- -D warnings
cargo +1.95.0 test --locked
cargo +1.95.0 build --locked
python3 tests/hygiene.py
```

Python is used only for testing. On Linux, run `python3 tests/smoke.py` for API integration checks. Set `NETCHECK_TEST_UNREACHABLE` to a valid host address you have selected and know will not reply to enable the complete outage workflow; that portion is skipped when the variable is unset. The suite also checks slow exports without blocking probe writes.

To check a built executable, run `python3 tests/package_smoke.py target/debug/netcheck` on Linux, or use `python tests/package_smoke.py target/debug/netcheck.exe` on Windows. This verifies startup, the embedded interface, an actual loopback reply, database ownership, pause, and exports. ICMP checks require suitable local permissions.

Browser tests additionally require Node.js 20+. Set `NETCHECK_TEST_UNREACHABLE` to your selected nonresponding host before running the suite:

```sh
npm ci
npx playwright install chromium
npm run test:browser
```

`CHROME_PATH` can select an installed Chrome executable; `NETCHECK_TEST_TIMEZONE` can select the test browser's timezone. The suite covers monitoring, histories, downloads, error states, keyboard navigation, and responsive layouts. Browser verification is scoped to Chrome.

### Windows build automation

[`.github/workflows/windows-package.yml`](.github/workflows/windows-package.yml) runs on pushes and pull requests to `main`, and can also be started manually. It builds for `x86_64-pc-windows-msvc` with a statically linked C runtime.

The workflow checks formatting, Clippy, native Windows unit tests, target hygiene, and the **release executable itself** before packaging and uploading the ZIP. The package includes build identity and checksums, and excludes user databases and target configuration.

### Repository layout

```text
src/                  Rust server, ICMP probes, storage, activity and exports
web/                  Embedded browser interface
packaging/windows/    Windows launcher and end-user instructions
tests/                API, executable, browser and hygiene checks
.github/workflows/    Windows build and packaging workflow
```

See [PROGRESS.md](PROGRESS.md) for the implementation and audit record. When reporting an issue, include the build commit, operating system, steps to reproduce, and the relevant error message. Redact private target names and addresses from shared logs.
