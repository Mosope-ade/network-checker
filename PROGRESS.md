# NetCheck progress

## Product contract

All monitoring addresses originate in the user interface and are saved in the local database. Fresh installations have no targets. There is no config-file, CLI-address, prefilled, seeded, or fallback-target monitoring path. Standard-library loopback constants are used only to bind the local application; automated tests resolve their own loopback hosts and take any failure-test target from the test operator.

## Codebase audit — 2026-11-12

Reviewed the Rust entry point, ICMP checks, state machine, storage and incident lifecycle, HTTP API, browser code, styles, test harnesses, dependencies, and documentation.

Corrections:

- Removed the obsolete config-driven daemon, its unsupported notification settings and hierarchical monitoring code. Renamed the shared gateway-specific ping helper to a generic typed probe module. Removed the unused TOML dependency.
- Removed address literals from source, UI examples, docs and tests. Added a repository hygiene regression test for this contract. Saved user data is preserved.
- Bind the web listener before touching monitoring state. Lock the database to one process, so a failed second launch cannot interrupt a live outage.
- Validate persisted target settings before starting tasks. Malformed settings cause an explicit storage failure rather than a panic/restart loop.
- Interrupt incidents at the last observed result after long monitoring gaps, backward clock jumps, shutdown and restart. Reset threshold sequences atomically and reject late results using generation numbers.
- Supervise all probe tasks and abort child probes during shutdown. Handle Ctrl+C and Unix SIGTERM. Move SQLite operations off asynchronous runtime threads.
- Use typed local probe errors and distinct ICMP identifiers. Local socket failures are not network outages. Reject invalid latency samples.
- Replace toggle actions with idempotent explicit enabled-state updates. Keep controls disabled across polling while requests are pending; preserve failed-action errors; discard older refresh responses; capture deletion IDs before awaiting requests.
- Render user-controlled values with DOM text nodes, not HTML strings. Freeze bulk-entry settings during submission and provide a whitespace-only input error.
- Display delayed monitoring explicitly and prioritize ongoing incidents in the log. Validate unicast target input, normalize equivalent address forms, and guard CSV formula prefixes including leading whitespace.

Validation and any remaining limits are recorded after the audit run. CSV exports still materialize in memory and hold the storage mutex during reading; unusually large histories can delay writes. Database locks require a local filesystem. Monitoring cannot establish events between samples. Browser verification is scoped to Chrome.

## Error handling and dependency follow-up

- Added a consistent JSON error envelope for API validation, malformed JSON, unknown targets/routes, incorrect methods, oversized requests, blocked origins, busy/full storage, and internal failures. Internal SQL and framework details are not sent to the browser.
- Added actionable startup messages, friendly ICMP explanations, separate connection/data-unavailable banners, persistent/dismissible action errors, malformed-response handling, timeout guidance, and in-app CSV export failures. Repeated polling no longer repeatedly rewrites identical live-region messages.
- Capture probe observation timestamps before waiting for storage. Database ownership failures remain typed rather than inferred from an error string.
- Updated Tokio to 1.53.3 for [RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html). Updated rusqlite to 0.41.1 with libsqlite3-sys 0.39.1 for [RUSTSEC-2026-0459](https://rustsec.org/advisories/RUSTSEC-2026-0459.html). Read upstream release notes and validated the upgrades separately; the newer rusqlite requires reading SQLite counts as signed integers.
- Queried all 129 registry package/version pairs from the final Rust lockfile against the [OSV API](https://api.osv.dev/v1/querybatch): no reported advisories after the updates. `npm audit` reported no vulnerabilities. This is a dated advisory check, not a guarantee against undiscovered vulnerabilities.

## Verification

- Reproduced the original ownership, malformed saved-settings and monitoring-gap defects with failing tests before correcting them. Reproduced the old plain-text error-response defect before introducing consistent JSON errors.
- 16 Rust unit tests passed. Clippy passed with warnings denied. The final executable built successfully.
- Two repository hygiene checks passed: no direct IP literals in source/frontend/tests/docs; no alternate configuration monitor or HTML-string rendering of user input.
- Three API smoke tests cover startup/retired flags, complete actual IPv4/IPv6 ping/incident lifecycle and restart, validation, duplicates, Origin/Host checks, conflicting launches, idempotent state changes, SIGTERM, export retention and structured errors including malformed requests and internal storage failures.
- Chrome browser checks cover bulk add, actual monitoring, escaped names, latency bars, delayed state updates across polling, pause/resume, cancel/remove, CSV downloads, error retention/dismissal, invalid addresses, failed exports without bogus downloads, unavailable storage, disconnect/reconnect, unreadable responses, request timeouts, dialogs, keyboard tabs and 320/768/1024/1440px layouts.
- Inspected screenshots of desktop/mobile operation and validation, action and storage errors. No unexpected browser console errors were observed. Deliberately injected failed requests are recognized by the tests.
- Checks were performed on Linux with Rust 1.95 and Google Chrome. No fresh Firefox, Safari or other-platform verification is claimed.
