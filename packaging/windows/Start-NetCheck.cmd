@echo off
setlocal
cd /d "%~dp0"
if not defined LOCALAPPDATA (
  echo NetCheck could not find your Windows user data folder.
  echo Run netcheck.exe --database with a writable database file path.
  pause
  exit /b 1
)
echo NetCheck stores your targets and history in %LOCALAPPDATA%\NetCheck.
echo Open http://localhost:8787 in your browser after startup.
echo Keep this window open while monitoring. Press Ctrl+C to stop.
"%~dp0netcheck.exe" --database "%LOCALAPPDATA%\NetCheck\netcheck.db"
if errorlevel 1 (
  echo.
  echo NetCheck stopped with an error. Read the message above before trying again.
  pause
  exit /b 1
)
