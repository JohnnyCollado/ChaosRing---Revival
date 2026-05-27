@echo off
REM Install the latest ChaosRing Android wrapper APK to the connected device via adb.
REM
REM Picks the newest APK out of, in order:
REM   dist\ChaosRing-Android-AArch64\ChaosRing.apk         (staged distro)
REM   android\app\build\outputs\apk\release\app-release.apk (raw gradle output)
REM   android\app\build\outputs\apk\debug\app-debug.apk    (debug fallback)
REM
REM Usage:  install_android.cmd            (install to the one connected device)
REM         install_android.cmd -s SERIAL  (forward -s SERIAL and any other adb args)

setlocal EnableExtensions EnableDelayedExpansion

pushd "%~dp0"

REM --- Locate adb -----------------------------------------------------------
REM Prefer the platform-tools bundled with this repo (kept up to date alongside
REM the build), fall back to PATH, then ANDROID_HOME/SDK_ROOT.
set "ADB="
if exist "%~dp0tools\platform-tools\adb.exe" set "ADB=%~dp0tools\platform-tools\adb.exe"
if not defined ADB where adb >nul 2>nul && set "ADB=adb"
if not defined ADB if defined ANDROID_HOME if exist "%ANDROID_HOME%\platform-tools\adb.exe" set "ADB=%ANDROID_HOME%\platform-tools\adb.exe"
if not defined ADB if defined ANDROID_SDK_ROOT if exist "%ANDROID_SDK_ROOT%\platform-tools\adb.exe" set "ADB=%ANDROID_SDK_ROOT%\platform-tools\adb.exe"
if not defined ADB (
    echo [error] adb not found. Expected at %~dp0tools\platform-tools\adb.exe,
    echo         on PATH, or under %%ANDROID_HOME%%\platform-tools\.
    popd & exit /b 1
)
echo Using adb : !ADB!

REM --- Pick the newest available APK ---------------------------------------
set "APK="
set "APK_NEWEST_TIME=0"
call :consider "dist\ChaosRing-Android-AArch64\ChaosRing.apk"
call :consider "android\app\build\outputs\apk\release\app-release.apk"
call :consider "android\app\build\outputs\apk\debug\app-debug.apk"

if not defined APK (
    echo [error] No APK found. Build one first:
    echo         powershell -ExecutionPolicy Bypass -File dev-scripts\make-android-distro.ps1
    popd & exit /b 1
)

echo Using APK: !APK!

REM --- Verify a device is reachable ----------------------------------------
"%ADB%" start-server >nul 2>nul

set "DEVICE_COUNT=0"
for /f "skip=1 tokens=1,2" %%a in ('"%ADB%" devices') do (
    if /i "%%b"=="device" set /a DEVICE_COUNT+=1
)
if "%DEVICE_COUNT%"=="0" (
    echo [error] No authorized devices connected. Plug the phone in, enable USB debugging,
    echo         and accept the host fingerprint on the device.
    "%ADB%" devices
    popd & exit /b 1
)

REM --- Install -------------------------------------------------------------
REM -r  replace existing app
REM -d  allow version downgrade
REM -g  grant all runtime permissions automatically
"%ADB%" %* install -r -d -g "!APK!"
set "RC=%ERRORLEVEL%"

if not "%RC%"=="0" (
    echo.
    echo [error] adb install failed ^(exit %RC%^).
    echo If you see INSTALL_FAILED_UPDATE_INCOMPATIBLE, the on-device signature
    echo differs from this build. Uninstall the existing app first:
    echo     "%ADB%" uninstall org.touchhle.android
    popd & exit /b %RC%
)

echo.
echo [ok] Installed. Launching org.touchhle.android/.MainActivity ...
"%ADB%" %* shell am start -n org.touchhle.android/.MainActivity >nul

popd
exit /b 0


:consider
REM %~1 = candidate path; updates APK / APK_NEWEST_TIME if newer than current pick.
if not exist "%~1" goto :eof
for %%F in ("%~1") do set "T=%%~tF"
REM Convert MM/DD/YYYY HH:MM (or DD/MM/YYYY, locale-dependent) into a sortable
REM key by stripping non-digits. Good enough for "newest of these three".
set "KEY=%T:/=%"
set "KEY=%KEY::=%"
set "KEY=%KEY: =%"
if "%KEY%" GTR "%APK_NEWEST_TIME%" (
    set "APK_NEWEST_TIME=%KEY%"
    set "APK=%~1"
)
goto :eof
