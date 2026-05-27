<#
.SYNOPSIS
    Build a release APK of touchHLE + the same Foundation fixes that make
    Chaos Rings run, then package it into a portable Android distribution.

.DESCRIPTION
    Wraps the upstream android/ Gradle project. Specifically:

      - Verifies prerequisites (Java, ANDROID_HOME, ANDROID_NDK_HOME,
        Rust aarch64-linux-android target, cargo-ndk).
      - Downloads Gradle 8.11.1 locally (to gradle/) if not present.
      - Temporarily patches android/app/build.gradle.kts to match the NDK
        version actually installed on this machine, if it differs from the
        pinned 25.2.9519653.
      - Generates android/local.properties pointing at ANDROID_HOME.
      - Runs `gradle assembleRelease`. cargo-ndk cross-compiles the touchHLE
        Rust crate to aarch64-linux-android; SDL2 and openal-soft compile
        through the NDK; the APK is signed with the debug key.
      - Bundles the resulting APK + license + RUNNING.md into
        dist/ChaosRing-Android-AArch64.zip.

    Players install the APK via `adb install <path>.apk` or by transferring
    the .apk to the device and tapping it. The .ipa file is then placed at
    Android/data/org.touchhle.android/files/touchHLE_apps/ (see RUNNING.md).

.PARAMETER SkipDownloadGradle
    Skip the Gradle download step. Use when a compatible Gradle (>= 8.11) is
    already on PATH or at gradle\gradle-8.11.1\bin\gradle.bat.

.PARAMETER Output
    Override the output zip path. Default: dist\ChaosRing-Android-AArch64.zip.

.EXAMPLE
    .\dev-scripts\make-android-distro.ps1

.EXAMPLE
    .\dev-scripts\make-android-distro.ps1 -SkipDownloadGradle
#>

[CmdletBinding()]
param(
    [switch]$SkipDownloadGradle,
    [string]$Output = "dist\ChaosRing-Android-AArch64.zip"
)

$ErrorActionPreference = 'Stop'

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

function Section([string]$msg) {
    Write-Host "`n=== $msg ===" -ForegroundColor Cyan
}

function Require-Cmd([string]$name, [string]$installHint) {
    $cmd = Get-Command $name -ErrorAction SilentlyContinue
    if (-not $cmd) { throw "Missing required tool: $name. $installHint" }
    Write-Host ("  {0,-12} {1}" -f $name, $cmd.Source)
}

# ---- 0. Resolve broken git symlinks ---------------------------------------
#
# Upstream touchHLE keeps the Android icon PNGs as git symlinks back to
# `res/`. When the repo is extracted from a zip on Windows (no symlink
# support), those become plain text files containing `../../../../res/...`
# and AAPT2 chokes when it tries to parse them as PNGs. Replace any such
# stubs with real copies of their target.

function Resolve-BrokenSymlinks {
    # Single-file stubs (png/jpg/ttf/otf): replace with a copy of the target file.
    $candidates = Get-ChildItem (Join-Path $RepoRoot 'android') -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Length -gt 0 -and $_.Length -lt 200 -and $_.Extension -in '.png', '.jpg', '.ttf', '.otf' }
    foreach ($f in $candidates) {
        $text = (Get-Content $f.FullName -TotalCount 1 -ErrorAction SilentlyContinue)
        if ($text -match '^\.\./') {
            $rel = $text.Trim()
            $target = Resolve-Path (Join-Path $f.DirectoryName $rel) -ErrorAction SilentlyContinue
            if ($target -and (Test-Path $target)) {
                Copy-Item -Path $target -Destination $f.FullName -Force
                Write-Host ("  Resolved symlink: {0} -> {1} ({2:N0} bytes)" -f $f.Name, $rel, (Get-Item $f.FullName).Length)
            } else {
                Write-Host ("  WARN: broken symlink and target not found: {0} -> {1}" -f $f.FullName, $rel) -ForegroundColor Yellow
            }
        }
    }

    # Stubs under assets/: anything 0 < size < 200 whose first line starts
    # with "../" is treated as a broken git symlink. Trailing "/" means it
    # pointed at a directory (touchHLE_dylibs, touchHLE_fonts) -> resolve to
    # a real recursive copy of that dir. No trailing "/" means it pointed at
    # a single file (touchHLE_default_options.txt and friends) -> resolve to
    # a real file copy. Either way the asset merger then sees a real
    # entity, not a 43-byte stub.
    $assets = Join-Path $RepoRoot 'android\app\src\main\assets'
    if (Test-Path $assets) {
        foreach ($f in Get-ChildItem $assets -File -ErrorAction SilentlyContinue) {
            if ($f.Length -gt 0 -and $f.Length -lt 200) {
                $text = (Get-Content $f.FullName -TotalCount 1 -ErrorAction SilentlyContinue)
                if ($text -match '^\.\./') {
                    $isDir = $text -match '/$'
                    $rel = $text.Trim().TrimEnd('/')
                    $target = Resolve-Path (Join-Path $f.DirectoryName $rel) -ErrorAction SilentlyContinue
                    if ($isDir) {
                        if ($target -and (Test-Path $target -PathType Container)) {
                            Remove-Item $f.FullName -Force
                            Copy-Item $target.Path $f.FullName -Recurse -Force
                            $count = (Get-ChildItem $f.FullName -Recurse -File).Count
                            Write-Host ("  Resolved dir symlink: {0} -> {1} ({2} files)" -f $f.Name, $rel, $count)
                        } else {
                            Write-Host ("  WARN: broken dir symlink and target not found: {0} -> {1}" -f $f.FullName, $rel) -ForegroundColor Yellow
                        }
                    } else {
                        if ($target -and (Test-Path $target -PathType Leaf)) {
                            Copy-Item $target.Path $f.FullName -Force
                            Write-Host ("  Resolved file symlink: {0} -> {1} ({2:N0} bytes)" -f $f.Name, $rel, (Get-Item $f.FullName).Length)
                        } else {
                            Write-Host ("  WARN: broken file symlink and target not found: {0} -> {1}" -f $f.FullName, $rel) -ForegroundColor Yellow
                        }
                    }
                }
            }
        }
    }
}

Section 'Resolving broken git symlinks (Windows-extracted repo)'
Resolve-BrokenSymlinks

# ---- 1. Verify prerequisites ----------------------------------------------

Section 'Checking prerequisites'

Require-Cmd 'java'  'Install JDK 11+ (Microsoft OpenJDK or Adoptium).'
Require-Cmd 'cargo' 'Install Rust toolchain via rustup.'

# cargo-ndk shows up as a cargo subcommand binary.
$cargoNdk = Get-Command 'cargo-ndk' -ErrorAction SilentlyContinue
if (-not $cargoNdk) {
    throw 'cargo-ndk not installed. Run: cargo install cargo-ndk@3.5.4'
}
Write-Host ("  {0,-12} {1}" -f 'cargo-ndk', $cargoNdk.Source)

if (-not $env:ANDROID_HOME) {
    throw 'ANDROID_HOME env var is not set. Install Android Studio or the command-line tools and set ANDROID_HOME to the SDK root.'
}
Write-Host ("  {0,-12} {1}" -f 'ANDROID_HOME', $env:ANDROID_HOME)

if (-not (Test-Path $env:ANDROID_HOME)) {
    throw "ANDROID_HOME points to $($env:ANDROID_HOME) but that directory does not exist."
}

if (-not $env:ANDROID_NDK_HOME -or -not (Test-Path $env:ANDROID_NDK_HOME)) {
    # Try to auto-detect from ANDROID_HOME\ndk\
    $ndkDir = Join-Path $env:ANDROID_HOME 'ndk'
    if (Test-Path $ndkDir) {
        $found = Get-ChildItem $ndkDir -Directory | Sort-Object Name -Descending | Select-Object -First 1
        if ($found) {
            $env:ANDROID_NDK_HOME = $found.FullName
            Write-Host ("  ANDROID_NDK_HOME not set; auto-detected: {0}" -f $env:ANDROID_NDK_HOME) -ForegroundColor Yellow
        }
    }
}
if (-not $env:ANDROID_NDK_HOME -or -not (Test-Path $env:ANDROID_NDK_HOME)) {
    throw 'ANDROID_NDK_HOME is not set and could not be auto-detected. Install an NDK via Android Studio SDK Manager and set ANDROID_NDK_HOME.'
}
Write-Host ("  {0,-12} {1}" -f 'NDK_HOME', $env:ANDROID_NDK_HOME)

$ndkVersionInstalled = Split-Path -Leaf $env:ANDROID_NDK_HOME
Write-Host ("  NDK version: {0}" -f $ndkVersionInstalled)

# dynarmic vendors an old fmt 10.x that fails to compile with NDK >= 27
# (Clang 17+ enforces consteval strictly and trips on fmt's code).
# Detect a known-incompatible NDK early and tell the user how to fix it,
# rather than letting them wait 3 minutes for the C++ build to die.
$ndkMajor = if ($ndkVersionInstalled -match '^(\d+)') { [int]$Matches[1] } else { 0 }
if ($ndkMajor -ge 27) {
    $msg = @"
Your installed NDK ($ndkVersionInstalled) is too new for the vendored fmt
library in dynarmic. Clang 17+ (shipped with NDK 27+) rejects the fmt code
with 'call to consteval function' errors.

Fix: install NDK r25c (version 25.2.9519653). Easiest paths:

  1. Android Studio -> SDK Manager -> SDK Tools tab -> tick "Show Package
     Details" -> tick "NDK (Side by side)" -> select 25.2.9519653 -> Apply.

  2. From the command line (if sdkmanager is on PATH):
       sdkmanager --install "ndk;25.2.9519653"

After install you'll have both NDKs side-by-side. Either:
  - Set ANDROID_NDK_HOME=$($env:ANDROID_HOME)\ndk\25.2.9519653 in this shell
    before re-running this script, or
  - Delete / move the 30.x NDK so this script auto-detects 25.2.9519653.
"@
    Write-Host "`n$msg" -ForegroundColor Yellow
    throw "Incompatible NDK version $ndkVersionInstalled (need r25c / 25.2.9519653)"
}

# Make ANDROID_NDK_ROOT and ANDROID_NDK aliases for the same path
# (different parts of the Android build system look at different vars).
$env:ANDROID_NDK_ROOT = $env:ANDROID_NDK_HOME
$env:ANDROID_NDK      = $env:ANDROID_NDK_HOME
$env:ANDROID_SDK_ROOT = $env:ANDROID_HOME

# Rust target
$targets = rustup target list --installed
if ($targets -notcontains 'aarch64-linux-android') {
    Write-Host '  aarch64-linux-android Rust target missing; installing...' -ForegroundColor Yellow
    rustup target add aarch64-linux-android
    if ($LASTEXITCODE -ne 0) { throw 'rustup target add failed' }
} else {
    Write-Host '  aarch64-linux-android Rust target: installed'
}

# ---- 2. Ensure a usable Gradle is available --------------------------------

Section 'Locating Gradle'

$gradleVersion = '8.11.1'
$localGradle   = Join-Path $RepoRoot ("gradle\gradle-$gradleVersion\bin\gradle.bat")

$gradleExe = $null
if (Test-Path $localGradle) {
    $gradleExe = $localGradle
    Write-Host "  Using local Gradle: $gradleExe"
} else {
    $onPath = Get-Command gradle -ErrorAction SilentlyContinue
    if ($onPath) {
        $gradleExe = $onPath.Source
        Write-Host "  Using Gradle on PATH: $gradleExe"
    }
}

if (-not $gradleExe) {
    if ($SkipDownloadGradle) {
        throw 'Gradle not found and -SkipDownloadGradle was set. Install Gradle or remove the flag.'
    }
    Section "Downloading Gradle $gradleVersion"
    $gradleZip  = Join-Path $RepoRoot ("gradle-$gradleVersion-bin.zip")
    $gradleDir  = Join-Path $RepoRoot 'gradle'
    if (-not (Test-Path $gradleDir)) {
        New-Item -ItemType Directory -Path $gradleDir | Out-Null
    }
    $gradleUrl = "https://services.gradle.org/distributions/gradle-$gradleVersion-bin.zip"
    Write-Host "  $gradleUrl"
    # Avoid the schannel revocation issue we hit elsewhere.
    curl.exe -L --fail --ssl-no-revoke -o $gradleZip $gradleUrl
    if ($LASTEXITCODE -ne 0) { throw "Failed to download Gradle from $gradleUrl" }
    Write-Host "  Extracting..."
    Expand-Archive -Path $gradleZip -DestinationPath $gradleDir -Force
    Remove-Item $gradleZip -Force
    $gradleExe = $localGradle
    if (-not (Test-Path $gradleExe)) {
        throw "Gradle download succeeded but $gradleExe not found"
    }
    Write-Host "  Installed at: $gradleExe"
}

# ---- 3. Patch ndkVersion if the installed NDK doesn't match the pin -------

$buildGradle = Join-Path $RepoRoot 'android\app\build.gradle.kts'
$buildGradleOrig = Get-Content $buildGradle -Raw

$pinnedNdk = [regex]::Match($buildGradleOrig, 'ndkVersion = "([^"]+)"').Groups[1].Value
$patchedNdkVersion = $false

if ($pinnedNdk -ne $ndkVersionInstalled) {
    Section "Patching ndkVersion ($pinnedNdk -> $ndkVersionInstalled)"
    Write-Host "  android/app/build.gradle.kts pins NDK $pinnedNdk; you have $ndkVersionInstalled."
    Write-Host "  Patching for this build only; the file will be restored on exit."
    $patched = $buildGradleOrig.Replace(
        "ndkVersion = ""$pinnedNdk""",
        "ndkVersion = ""$ndkVersionInstalled"""
    )
    Set-Content -Path $buildGradle -Value $patched -NoNewline
    $patchedNdkVersion = $true
}

# ---- 4. Generate local.properties -----------------------------------------

$localProps = Join-Path $RepoRoot 'android\local.properties'
$sdkDirEscaped = $env:ANDROID_HOME.Replace('\', '\\').Replace(':', '\:')
"sdk.dir=$sdkDirEscaped" | Set-Content -Path $localProps -Encoding ascii
Write-Host "  Wrote $localProps"

# ---- 5. Build the APK ------------------------------------------------------

try {
    Section 'Building APK (this takes several minutes on first run)'

    Push-Location (Join-Path $RepoRoot 'android')
    try {
        & $gradleExe assembleRelease --console=plain --no-daemon
        if ($LASTEXITCODE -ne 0) { throw "gradle assembleRelease failed (exit $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }

    $apkPath = Join-Path $RepoRoot 'android\app\build\outputs\apk\release\app-release.apk'
    if (-not (Test-Path $apkPath)) {
        # Some configurations produce different output filenames; do a search.
        $apkPath = Get-ChildItem (Join-Path $RepoRoot 'android\app\build\outputs\apk\release') -Filter '*.apk' -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
    }
    if (-not $apkPath -or -not (Test-Path $apkPath)) {
        throw 'APK not found in android\app\build\outputs\apk\release\'
    }
    Write-Host "`nAPK: $apkPath ($(Get-Item $apkPath | ForEach-Object { '{0:N0} bytes' -f $_.Length }))"
}
finally {
    # Always restore the patched gradle file.
    if ($patchedNdkVersion) {
        Set-Content -Path $buildGradle -Value $buildGradleOrig -NoNewline
        Write-Host "Restored original $buildGradle"
    }
}

# ---- 6. Package the distro -------------------------------------------------

Section 'Packaging distribution'

$staging = Join-Path $RepoRoot 'dist\ChaosRing-Android-AArch64'
if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Path $staging | Out-Null

Copy-Item $apkPath (Join-Path $staging 'ChaosRing.apk')
Copy-Item (Join-Path $RepoRoot 'LICENSE')                 (Join-Path $staging 'LICENSE.MPL2.txt')
Copy-Item (Join-Path $RepoRoot 'dev-scripts\gpl-3.0.txt') (Join-Path $staging 'LICENSE.GPL3.txt')
Copy-Item (Join-Path $RepoRoot 'README.md')               $staging

@'
# Running Chaos Rings on Android

## 1. Install the APK

Transfer `ChaosRing.apk` to your Android device and tap to install (you may
need to allow "Install from unknown sources" first).

Or from a computer with adb:

```
adb install ChaosRing.apk
```

The package ID is `org.touchhle.android` -- if you have a stock touchHLE
build installed already, uninstall it first to avoid signature conflicts.

## 2. Grant "All files access" on first launch

The wrapper reads and writes its data folder at `/sdcard/ChaosRing/`,
which on Android 11+ requires the "All files access" special permission.

The first time you open Chaos Ring it detects the missing permission,
shows a brief toast, and forwards you to the system settings page for
"All files access". Flip the toggle on for Chaos Ring, press back, then
re-launch the app from the launcher.

(On Android 10 and earlier the app instead asks for the legacy
`WRITE_EXTERNAL_STORAGE` runtime permission; same idea, smaller dialog.)

## 3. Place your .ipa file

The app looks for `.ipa` and `.app` bundles under:

```
/sdcard/ChaosRing/touchHLE_apps/
```

Drop your legally-obtained Chaos Rings IPA there. The wrapper auto-launches
the first bundle it finds (sorted by filename) and passes
`--disable-direct-memory-access` so Chaos Rings runs reliably:

- `CHAOS_RINGS_2.2.0_ios_3.0.ipa` (iPhone build)

(The iPad / HD build is installed and run via the separate
`org.touchhle.android.hd` package, which uses `/sdcard/ChaosRingsHD/`.)

To get a file there:

- After the permission has been granted (step 2), use the in-app
  "File manager" button on the picker to open the system file picker at
  `/sdcard/ChaosRing/`.
- Or push from a computer with adb:

```
adb push CHAOS_RINGS_2.2.0_ios_3.0.ipa /sdcard/ChaosRing/touchHLE_apps/
```

## 4. Launch

Open the app again. With "All files access" granted and an IPA in
`/sdcard/ChaosRing/touchHLE_apps/`, the wrapper boots straight into
Chaos Rings without showing the picker.

## 5. Saves

Saves are kept under your data folder at:

```
/sdcard/ChaosRing/touchHLE_sandbox/com.square-enix.ChaosRings/Documents/data.dat
```

Same caveats as the Windows build -- no DRM, can be backed up, occasional
corruption possible. Save often.

## 6. Known limitations

- Hardware character skinning is tolerated, not implemented; animated
  meshes render in bind pose.
- Performance varies dramatically by device. A flagship-class phone runs
  the iPhone build well; the iPad build wants more headroom.
- The Windows-specific crash handler does nothing on Android (it is
  gated by `cfg(windows)`); panics on Android go through Rust's default
  panic-on-stderr path, captured in logcat.
'@ | Set-Content -Path (Join-Path $staging 'RUNNING.md') -Encoding utf8

$outAbs = if ([IO.Path]::IsPathRooted($Output)) { $Output } else { Join-Path $RepoRoot $Output }
$outDir = Split-Path -Parent $outAbs
if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
if (Test-Path $outAbs) { Remove-Item $outAbs -Force }

Compress-Archive -Path $staging -DestinationPath $outAbs -CompressionLevel Optimal

$zipInfo = Get-Item $outAbs
Write-Host "`n[OK] Built: $($zipInfo.FullName)" -ForegroundColor Green
Write-Host ("  Size: {0:N0} bytes ({1:N1} MB)" -f $zipInfo.Length, ($zipInfo.Length / 1MB))
