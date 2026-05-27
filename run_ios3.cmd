@echo off
REM Launch Chaos Rings (iPhone, iOS 3.0 build) in touchHLE.
REM Forwards any extra args (e.g. --fullscreen, --scale-hack=2) to touchHLE.

pushd "%~dp0"
REM --disable-direct-memory-access forces every guest memory access through
REM the callback path, avoiding host AVs from JIT'd direct page-table reads.
"target\release\touchHLE.exe" "touchHLE_apps\CHAOS_RINGS_2.2.0_ios_3.0.ipa" --disable-direct-memory-access %*
set rc=%errorlevel%
popd
exit /b %rc%
