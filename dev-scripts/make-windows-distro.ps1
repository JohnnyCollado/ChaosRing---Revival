<#
.SYNOPSIS
    Build and package a portable Windows distribution of touchHLE + Chaos Rings.

.DESCRIPTION
    Produces dist/ChaosRing-Windows-x64.zip containing:
      - touchHLE.exe (release build)
      - touchHLE_dylibs/ + touchHLE_fonts/ runtime assets (with license files)
      - touchHLE_default_options.txt + touchHLE_options.txt + OPTIONS_HELP.txt
      - "Run Chaos Ring.cmd" + "Run Chaos Ring HD.cmd" launchers
      - LICENSE + COPYING.txt (GPL3, for binary distribution)
      - README.md (the project journey) + RUNNING.md (end-user playbook)
      - touchHLE_apps/README.txt explaining where to drop the user's IPAs

    The Chaos Rings IPAs themselves are never bundled into the distro —
    they are copyrighted and end users must supply their own.

    Excludes: vendor/, src/, target/ debug, the IPA files (copyrighted),
    touchHLE_sandbox/, all logs, and the build environment generally.

.PARAMETER SkipBuild
    Skip running cargo build. Use when touchHLE.exe is already up to date.

.PARAMETER Output
    Override the output zip path. Default: dist/ChaosRing-Windows-x64.zip.

.EXAMPLE
    .\dev-scripts\make-windows-distro.ps1

.EXAMPLE
    .\dev-scripts\make-windows-distro.ps1 -SkipBuild -Output dist/build42.zip
#>

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$Output = 'dist\ChaosRing-Windows-x64.zip'
)

$ErrorActionPreference = 'Stop'

# Repo root = parent of dev-scripts/
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

function Section([string]$msg) {
    Write-Host "`n=== $msg ===" -ForegroundColor Cyan
}

# ---- 1. Build ---------------------------------------------------------------

if (-not $SkipBuild) {
    Section 'Building touchHLE (release)'
    $env:CARGO_HTTP_CHECK_REVOKE = 'false'
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
} else {
    Section 'Skipping build (-SkipBuild)'
}

$exe = Join-Path $RepoRoot 'target\release\touchHLE.exe'
if (-not (Test-Path $exe)) {
    throw "touchHLE.exe not found at $exe -- run without -SkipBuild or build first"
}
Write-Host "Using binary: $exe ($(Get-Item $exe | ForEach-Object { '{0:N0} bytes, {1}' -f $_.Length, $_.LastWriteTime }))"

# ---- 2. Stage the distribution tree ----------------------------------------

Section 'Staging distribution tree'
$staging = Join-Path $RepoRoot 'dist\ChaosRing-Windows-x64'
if (Test-Path $staging) {
    Remove-Item $staging -Recurse -Force
}
New-Item -ItemType Directory -Path $staging | Out-Null

# Binary
Copy-Item $exe (Join-Path $staging 'touchHLE.exe')

# Runtime asset directories (verbatim copies; these include their license files)
Copy-Item (Join-Path $RepoRoot 'touchHLE_dylibs') $staging -Recurse
Copy-Item (Join-Path $RepoRoot 'touchHLE_fonts')  $staging -Recurse

# Options and help
Copy-Item (Join-Path $RepoRoot 'touchHLE_default_options.txt') $staging
Copy-Item (Join-Path $RepoRoot 'touchHLE_options.txt')         $staging
Copy-Item (Join-Path $RepoRoot 'OPTIONS_HELP.txt')             $staging

# Launchers. The cmds in the repo root reference target\release\touchHLE.exe
# because that's where the binary lives when running from a dev checkout. In
# the distribution touchHLE.exe sits next to the cmds, so rewrite the binary
# path so end-user double-clicks actually work.
function Rewrite-LauncherCmd([string]$srcPath, [string]$destPath) {
    $text = Get-Content -Raw -Path $srcPath
    $rewritten = $text -replace [regex]::Escape('"target\release\touchHLE.exe"'), '"%~dp0touchHLE.exe"'
    if ($rewritten -eq $text) {
        Write-Host ("  WARN: launcher {0} didn't contain the expected target\release\touchHLE.exe reference; copying verbatim" -f (Split-Path -Leaf $srcPath)) -ForegroundColor Yellow
    }
    Set-Content -Path $destPath -Value $rewritten -NoNewline -Encoding ascii
}
# End-user-facing names (with spaces, so they read as buttons rather than
# dev artefacts). Repo cmds keep the short snake_case for in-repo use.
Rewrite-LauncherCmd (Join-Path $RepoRoot 'run_ios3.cmd') (Join-Path $staging 'Run Chaos Ring.cmd')
Rewrite-LauncherCmd (Join-Path $RepoRoot 'run_ios4.cmd') (Join-Path $staging 'Run Chaos Ring HD.cmd')

# Licensing
Copy-Item (Join-Path $RepoRoot 'LICENSE')                       (Join-Path $staging 'LICENSE.MPL2.txt')
Copy-Item (Join-Path $RepoRoot 'dev-scripts\gpl-3.0.txt')       (Join-Path $staging 'LICENSE.GPL3.txt')

# Documentation
Copy-Item (Join-Path $RepoRoot 'README.md') $staging

# touchHLE_apps/: empty placeholder with a README explaining where the user
# is expected to drop their own .ipa files. The IPAs themselves are never
# bundled (they are copyrighted).
$appsDir = Join-Path $staging 'touchHLE_apps'
New-Item -ItemType Directory -Path $appsDir | Out-Null

@'
Put your Chaos Rings .ipa file here.

The run scripts expect these exact filenames:
  - CHAOS_RINGS_2.2.0_ios_3.0.ipa       (iPhone build; "Run Chaos Ring.cmd" uses this)
  - CHAOS_RINGS_for_iPad_2.3.0_ios_4.3.ipa  (iPad/HD build; "Run Chaos Ring HD.cmd" uses this)

If your IPA has a different name, either rename it or edit the .cmd file to
point at your filename. Both builds work -- the iPhone version is lighter on
memory, the iPad version has higher-res assets.

Only use legally obtained software with this emulator.
'@ | Set-Content -Path (Join-Path $appsDir 'README.txt') -Encoding utf8

# End-user playbook.
$runningMd = @'
# Running Chaos Rings in this build of touchHLE

## 1. Drop your IPA in `touchHLE_apps/`

This release does not include the game. You must supply your own legally-
obtained `.ipa` file. Two versions are supported:

| File the launcher expects                | What it is              | Launcher                  |
|-------------------------------------------|--------------------------|---------------------------|
| `CHAOS_RINGS_2.2.0_ios_3.0.ipa`           | iPhone build (2.2.0)    | `Run Chaos Ring.cmd`      |
| `CHAOS_RINGS_for_iPad_2.3.0_ios_4.3.ipa`  | iPad / HD build (2.3.0) | `Run Chaos Ring HD.cmd`   |

If your `.ipa` is named differently, rename it to one of the above, or open
the matching `.cmd` file in Notepad and change the filename inside.

## 2. Launch

Double-click `Run Chaos Ring.cmd` (or `Run Chaos Ring HD.cmd`). The game
window opens. The window is resizable -- drag a corner, or hit the OS
maximize button. The game's view is letterboxed to keep its aspect ratio
inside whatever size you choose.

You can pass extra touchHLE flags after the script name from a terminal:

```
"Run Chaos Ring.cmd" --fullscreen
"Run Chaos Ring.cmd" --scale-hack=2 --print-fps
```

See `OPTIONS_HELP.txt` for all flags.

## 3. Where your save goes

Your save lives at:

```
touchHLE_sandbox\com.square-enix.ChaosRings\Documents\data.dat
```

(or `com.square-enix.ChaosRingsHD\` for the iPad build.)

You can back this folder up with normal file-copy tools -- there's no DRM
on the save file. Save corruption is rare but possible; backing up after
big progress milestones is a good habit.

## 4. Controls

Default touchHLE controls (full list in `OPTIONS_HELP.txt`):

- **Mouse:** left-click to tap/hold/drag the simulated touch screen
- **Game controller:** right analog stick moves a virtual cursor; press
  the stick or right shoulder button to tap. Left analog stick simulates
  device tilt.
- **Right mouse button held:** simulates device tilt via mouse position

## 5. Known limitations

- Hardware character skinning isn't implemented in our GLES1-on-GL2 layer,
  so animated character meshes render in their bind pose. Static and 2D
  geometry are unaffected.
- The iPad/HD build (`Run Chaos Ring HD.cmd`) uses more memory than the iPhone
  build. If you hit issues, try the iPhone version first.

## 6. If something crashes

Two files in this folder are produced on crash:

- `touchHLE_log.txt` -- full run log; usually contains a `Panic at <file>:<line>`
  line near the end that pinpoints the failure.
- `touchHLE_crash.txt` -- only written for hard Windows access violations;
  contains CPU register state at the moment of crash.

Both are safe to share when reporting issues.
'@

$runningMd | Set-Content -Path (Join-Path $staging 'RUNNING.md') -Encoding utf8

Write-Host "Staged $(((Get-ChildItem $staging -Recurse -File) | Measure-Object).Count) files in $staging"

# ---- 3. Zip ----------------------------------------------------------------

Section 'Creating zip'
$outAbs = if ([IO.Path]::IsPathRooted($Output)) { $Output } else { Join-Path $RepoRoot $Output }
$outDir = Split-Path -Parent $outAbs
if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
if (Test-Path $outAbs) { Remove-Item $outAbs -Force }

# Compress-Archive zips the contents of the directory, not the directory
# itself, when the source ends with \* — but it preserves names when given
# just the directory. We want the zip to contain a top-level
# ChaosRing-Windows-x64/ folder so users unzipping it land in a clean
# directory rather than scattering files into cwd.
Compress-Archive -Path $staging -DestinationPath $outAbs -CompressionLevel Optimal

$zipInfo = Get-Item $outAbs
Write-Host "`n[OK] Built: $($zipInfo.FullName)" -ForegroundColor Green
Write-Host ("  Size: {0:N0} bytes ({1:N1} MB)" -f $zipInfo.Length, ($zipInfo.Length / 1MB))
