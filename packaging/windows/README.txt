NetCheck for Windows x64 - portable package

1. Extract the entire NetCheck-windows-x64 folder from the ZIP.
2. Double-click Start-NetCheck.cmd.
3. Open http://localhost:8787 in your browser.
4. Add your own monitoring targets through the interface. No targets are preset.

Keep the console window open while monitoring. Press Ctrl+C to stop.
The interface runs in your browser; it is embedded in netcheck.exe. Rust,
Node.js, a separate web server, and a separate SQLite installation are not needed.

The launcher stores your targets and history under:
  %LOCALAPPDATA%\NetCheck\netcheck.db
Do not delete this folder when updating. Stop NetCheck, replace the extracted
application files, and start the launcher again. Existing data stays in place.
The .owner.lock file beside the database is normal and must not be deleted while
NetCheck is running.

If local ICMP probes are unavailable, close NetCheck and run Start-NetCheck.cmd
using "Run as administrator". Some Windows policies require elevation for raw
ICMP sockets. A missing reply can also mean a host or firewall blocks ping;
it does not prove that all services on the target network are down.

For another port or data file, use a terminal:
  netcheck.exe --port 9000 --database "C:\YourFolder\netcheck.db"

If the default port is occupied, close the other instance or choose another port.
The UI, local storage and monitoring require the app to remain running.

BUILD.txt identifies the source commit and toolchain. SHA256SUMS.txt contains
the executable checksum. The download also includes a checksum for the ZIP.
This is a portable, unsigned executable, not an MSI installer or installed service.
