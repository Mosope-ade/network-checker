# NetCheck implementation progress

## Product direction

Local GUI for pinging one or multiple network endpoints concurrently and logging downtime. The supplied daemon-first `plan.md` is historical context; the GUI is now the default entry point.

## Implemented

- Rust/Axum loopback-only server with embedded HTML, CSS, and JavaScript.
- Bulk addition of named IPv4/IPv6 targets; duplicate and invalid input rejection; maximum 128 targets.
- Independent Tokio monitor task per target using the existing surge-ping ICMP check.
- Live target status, latency, lifetime packet loss, probe counts, and last 60 probe results.
- Per-target interval and consecutive failure/recovery thresholds.
- Pause/resume and removal; target history survives removal.
- Threshold-based outage lifecycle and SQLite storage of all incidents.
- Explicit interrupted incidents when monitoring stops without confirmed recovery.
- Restart persistence without counting unobserved process downtime.
- Local ICMP socket failures displayed separately from network outages.
- Downtime log and complete CSV export with UTC timestamps and spreadsheet formula escaping.
- Responsive layout, native keyboard-accessible dialogs, keyboard-operated tabs, disconnected state, empty states, and escaped user content.
- Original console diagnostics available through `--daemon`.
- Unit tests, real-ping API smoke test, and optional Playwright browser checks.

## Scope boundaries

A target is one host IP representing a network. This version uses the host machine's routing table; it does not scan ranges, bind separate egress interfaces, or accept hostnames. Monitoring requires the process and computer to remain running. There are no Telegram/desktop alerts or installed background services. These are separate extensions from the original daemon roadmap.

## Verification completed — 2026-11-12

- Eight Rust unit tests passed.
- `cargo clippy --all-targets -- -D warnings` passed without warnings.
- `cargo build`, `cargo fmt --check`, and `git diff --check` passed.
- Real-ping API smoke test passed, including IPv4, IPv6, concurrent healthy/unreachable targets, pause/resume, restart persistence, retained incidents, CSV, invalid/duplicate input, and Host/Origin protection.
- Headless Google Chrome browser checks passed for bulk add, real status updates, latency bars, pause/resume, removal/cancellation, search, CSV download, HTML escaping, dialog focus, keyboard tabs, and 320/768/1024/1440px layouts, with zero console errors or warnings.
- Desktop, narrow-screen, and add-dialog screenshots were visually inspected. This is Chrome verification; Firefox and Safari have not been tested.
