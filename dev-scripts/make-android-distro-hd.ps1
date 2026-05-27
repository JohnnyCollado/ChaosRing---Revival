<#
.SYNOPSIS
    Build a second wrapper APK for the Chaos Rings iPad / HD release
    (CHAOS_RINGS_for_iPad_2.3.0_ios_4.3.ipa), installable side-by-side
    with the existing 2.2.0 wrapper.

.DESCRIPTION
    Same machinery as make-android-distro.ps1, but with these inputs
    swapped in for one build only and restored on exit:

      - drawable-nodpi/icon.png: HD character icon (extracted + rounded
        from ChaosRingsHD60@3x.png inside the 2.3.0 IPA via
        defry_ios_png.py + a 22% corner mask)
      - build.gradle.kts       : applicationId "org.touchhle.android"
        -> "org.touchhle.android.hd" so the package is distinct from
        the 2.2.0 wrapper, and the launcher label "Chaos Ring" ->
        "Chaos Ring HD"
      - src/paths.rs, MainActivity.java, DocumentsProvider.kt: data
        folder "/sdcard/ChaosRing" -> "/sdcard/ChaosRingsHD" so saves
        and the IPA live in a separate directory from the 2.2.0 build.

    The IPA itself is no longer bundled inside the APK; the user is
    expected to place CHAOS_RINGS_for_iPad_2.3.0_ios_4.3.ipa under
    /sdcard/ChaosRingsHD/touchHLE_apps/ on the device.

    Output: dist\ChaosRing-HD-Android-AArch64.zip (and the raw APK
    next to it). Original sources are restored even on Ctrl+C or
    build failure via try/finally.

.PARAMETER Output
    Override the output zip path. Default:
    dist\ChaosRing-HD-Android-AArch64.zip.
#>

[CmdletBinding()]
param(
    [string]$Output = 'dist\ChaosRing-HD-Android-AArch64.zip'
)

$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

function Section([string]$msg) {
    Write-Host "`n=== $msg ===" -ForegroundColor Cyan
}

# ---- Locate inputs -------------------------------------------------------

$iconHdSrc        = Join-Path $RepoRoot 'dist\icon-hd-work\icon_hd.png'
$iconDest         = Join-Path $RepoRoot 'android\app\src\main\res\drawable-nodpi\icon.png'
$buildGradle      = Join-Path $RepoRoot 'android\app\build.gradle.kts'
$pathsRs          = Join-Path $RepoRoot 'src\paths.rs'
$mainActivity     = Join-Path $RepoRoot 'android\app\src\main\java\org\touchhle\android\MainActivity.java'
$documentsProvider= Join-Path $RepoRoot 'android\app\src\main\java\org\touchhle\android\DocumentsProvider.kt'

if (-not (Test-Path $iconHdSrc)) { throw "Missing HD icon: $iconHdSrc (run dist/icon-hd-work setup first)" }

# Files whose contents we patch and then restore. Listed as (path, snapshotName).
$patchTargets = @(
    @{ Path = $buildGradle;       Snap = 'build.gradle.kts' },
    @{ Path = $pathsRs;           Snap = 'paths.rs' },
    @{ Path = $mainActivity;      Snap = 'MainActivity.java' },
    @{ Path = $documentsProvider; Snap = 'DocumentsProvider.kt' }
)

# ---- Snapshot current state ----------------------------------------------

Section 'Snapshotting current sources'

$swapDir = Join-Path $RepoRoot 'dist\hd-build-swap'
if (Test-Path $swapDir) { Remove-Item $swapDir -Recurse -Force }
New-Item -ItemType Directory -Path $swapDir | Out-Null

# Back up the launcher icon.
$iconBackup = Join-Path $swapDir 'icon.png'
Copy-Item $iconDest $iconBackup
Write-Host "  snap: $iconDest -> $iconBackup"

# Back up each text file we'll patch.
foreach ($t in $patchTargets) {
    $dest = Join-Path $swapDir $t.Snap
    Copy-Item $t.Path $dest
    Write-Host ("  snap: {0} -> {1}" -f $t.Path, $dest)
}

try {
    # ---- Swap in HD inputs ------------------------------------------------

    Section 'Swapping in 2.3.0 / HD inputs'

    # Replace launcher icon with the HD one.
    Copy-Item $iconHdSrc $iconDest -Force
    Write-Host "  icon: $iconDest"

    # Patch build.gradle.kts in-place: append .hd suffix to applicationId
    # and rename the launcher label to "Chaos Ring HD".
    $g = Get-Content $buildGradle -Raw
    $patchedG = $g `
        -replace 'applicationId = "org\.touchhle\.android"', 'applicationId = "org.touchhle.android.hd"' `
        -replace '"Chaos Ring"', '"Chaos Ring HD"'
    if ($patchedG -eq $g) {
        throw 'build.gradle.kts patch produced no changes - source may have shifted; refusing to build.'
    }
    Set-Content -Path $buildGradle -Value $patchedG -NoNewline
    Write-Host '  gradle: applicationId -> org.touchhle.android.hd, label -> "Chaos Ring HD"'

    # Patch the data folder constant in the three files that reference it.
    # All three currently contain the literal string "/sdcard/ChaosRing" and
    # nothing else that matches that prefix, so a plain replace is safe.
    foreach ($f in @($pathsRs, $mainActivity, $documentsProvider)) {
        $orig = Get-Content $f -Raw
        $patched = $orig.Replace('/sdcard/ChaosRing', '/sdcard/ChaosRingsHD')
        if ($patched -eq $orig) {
            throw "Data-folder patch produced no changes in $f - source may have shifted; refusing to build."
        }
        Set-Content -Path $f -Value $patched -NoNewline
        Write-Host ("  patched: {0} (/sdcard/ChaosRing -> /sdcard/ChaosRingsHD)" -f $f)
    }

    # ---- Invoke the main build -------------------------------------------

    Section 'Building APK via make-android-distro.ps1'

    & (Join-Path $PSScriptRoot 'make-android-distro.ps1') `
        -Output 'dist\ChaosRing-HD-Android-AArch64.zip'

    if ($LASTEXITCODE -ne 0) {
        throw "Inner build script failed (exit $LASTEXITCODE)"
    }

    # ---- Stage / rename the output ---------------------------------------

    # The inner script writes to dist/ChaosRing-Android-AArch64/ and zips
    # that staging dir, then names the zip per -Output. Rename the staging
    # dir to match the HD output name so files inside don't get clobbered
    # by a future 2.2.0 build.
    $stagingDefault = Join-Path $RepoRoot 'dist\ChaosRing-Android-AArch64'
    $stagingHd      = Join-Path $RepoRoot 'dist\ChaosRing-HD-Android-AArch64'
    if (Test-Path $stagingDefault) {
        if (Test-Path $stagingHd) { Remove-Item $stagingHd -Recurse -Force }
        Rename-Item $stagingDefault (Split-Path -Leaf $stagingHd)
        # Rename the inner APK so the install command picks it up correctly.
        $oldApk = Join-Path $stagingHd 'ChaosRing.apk'
        $newApk = Join-Path $stagingHd 'ChaosRingHD.apk'
        if (Test-Path $oldApk) { Rename-Item $oldApk (Split-Path -Leaf $newApk) }
        Write-Host "  staged: $stagingHd"

        # The inner script wrote a RUNNING.md aimed at the iPhone wrapper
        # (org.touchhle.android, /sdcard/ChaosRing/). Replace it with the
        # HD variant so end users see the correct package and data folder.
        @'
# Running Chaos Rings HD on Android

## 1. Install the APK

Transfer `ChaosRingHD.apk` to your Android device and tap to install (you
may need to allow "Install from unknown sources" first).

Or from a computer with adb:

```
adb install ChaosRingHD.apk
```

The package ID is `org.touchhle.android.hd`. It installs cleanly alongside
the iPhone wrapper (`org.touchhle.android`) -- you can keep both on the
device with separate saves.

## 2. Grant "All files access" on first launch

The wrapper reads and writes its data folder at `/sdcard/ChaosRingsHD/`,
which on Android 11+ requires the "All files access" special permission.

The first time you open Chaos Ring HD it detects the missing permission,
shows a brief toast, and forwards you to the system settings page for
"All files access". Flip the toggle on for Chaos Ring HD, press back,
then re-launch the app from the launcher.

(On Android 10 and earlier the app instead asks for the legacy
`WRITE_EXTERNAL_STORAGE` runtime permission; same idea, smaller dialog.)

## 3. Place your .ipa file

The HD wrapper looks for `.ipa` and `.app` bundles under:

```
/sdcard/ChaosRingsHD/touchHLE_apps/
```

Drop your legally-obtained iPad / HD Chaos Rings IPA there:

- `CHAOS_RINGS_for_iPad_2.3.0_ios_4.3.ipa` (iPad / HD build)

To get a file there:

- After the permission has been granted (step 2), use the in-app
  "File manager" button on the picker to open the system file picker at
  `/sdcard/ChaosRingsHD/`.
- Or push from a computer with adb:

```
adb push CHAOS_RINGS_for_iPad_2.3.0_ios_4.3.ipa /sdcard/ChaosRingsHD/touchHLE_apps/
```

## 4. Launch

Open the app again. With "All files access" granted and an IPA in
`/sdcard/ChaosRingsHD/touchHLE_apps/`, the wrapper boots straight into
Chaos Rings HD without showing the picker.

## 5. Saves

Saves are kept under your data folder at:

```
/sdcard/ChaosRingsHD/touchHLE_sandbox/com.square-enix.ChaosRingsHD/Documents/data.dat
```

The iPhone wrapper writes to `/sdcard/ChaosRing/...` instead -- the two
folders are completely separate, so HD and iPhone saves never collide.

## 6. Known limitations

- Hardware character skinning is tolerated, not implemented; animated
  meshes render in bind pose.
- The iPad / HD build uses more memory than the iPhone build. On older
  or low-RAM devices the iPhone wrapper (`org.touchhle.android`) is the
  safer choice.
- The Windows-specific crash handler does nothing on Android; panics
  on Android go through Rust's default panic-on-stderr path, captured
  in logcat.
'@ | Set-Content -Path (Join-Path $stagingHd 'RUNNING.md') -Encoding utf8
        Write-Host "  rewrote RUNNING.md for the HD wrapper"
    }

    # Re-zip the staging dir so the published archive carries the renamed
    # folder, the renamed APK, and the HD-specific RUNNING.md (the inner
    # script's zip captured the pre-rename / iPhone state).
    $outAbs = if ([IO.Path]::IsPathRooted($Output)) { $Output } else { Join-Path $RepoRoot $Output }
    $outDir = Split-Path -Parent $outAbs
    if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
    if (Test-Path $outAbs) { Remove-Item $outAbs -Force }
    Compress-Archive -Path $stagingHd -DestinationPath $outAbs -CompressionLevel Optimal

    if (Test-Path $outAbs) {
        $zipInfo = Get-Item $outAbs
        Write-Host "`n[OK] HD wrapper built: $($zipInfo.FullName)" -ForegroundColor Green
        Write-Host ("  Size: {0:N0} bytes ({1:N1} MB)" -f $zipInfo.Length, ($zipInfo.Length / 1MB))
    }
}
finally {
    # ---- Restore original sources ----------------------------------------

    Section 'Restoring original sources'

    Copy-Item $iconBackup $iconDest -Force
    Write-Host "  restored: $iconDest"

    foreach ($t in $patchTargets) {
        $src = Join-Path $swapDir $t.Snap
        if (Test-Path $src) {
            Copy-Item $src $t.Path -Force
            Write-Host ("  restored: {0}" -f $t.Path)
        }
    }

    Remove-Item $swapDir -Recurse -Force -ErrorAction SilentlyContinue
}
