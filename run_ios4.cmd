@echo off
REM Launch Chaos Rings HD (iPad, iOS 4.3 build) in touchHLE.
REM Forwards any extra args (e.g. --fullscreen, --scale-hack=2) to touchHLE.

pushd "%~dp0"
"target\release\touchHLE.exe" "touchHLE_apps\CHAOS_RINGS_for_iPad_2.3.0_ios_4.3.ipa" %*
set rc=%errorlevel%
popd
exit /b %rc%
