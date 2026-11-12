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
