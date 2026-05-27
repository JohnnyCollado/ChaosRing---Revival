/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * Parts of this file are derived from SDL 2's Android project template, which
 * has a different license. Please see vendor/SDL/LICENSE.txt for details.
 */
package org.touchhle.android;

import android.Manifest;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.Settings;
import android.util.Log;
import android.widget.Toast;

import org.libsdl.app.SDLActivity;

import java.io.File;
import java.util.ArrayList;
import java.util.Arrays;

/**
 * Wrapper over SDLActivity that points touchHLE at /sdcard/ChaosRing/ as its
 * main data folder. On first launch we ensure the directory exists and that
 * the app has the necessary storage permission; if it doesn't, we send the
 * user to Settings rather than starting SDL with a path it can't read or
 * write. Once a usable directory is in place we look for the first .ipa/.app
 * bundle inside /sdcard/ChaosRing/touchHLE_apps/ and pass it to SDL_main as
 * argv[1], skipping touchHLE's built-in picker.
 */
public class MainActivity extends SDLActivity {

    private static final String TAG = "ChaosRing";
    private static final String DATA_DIR = "/sdcard/ChaosRing";
    private static final String APPS_SUBDIR = "touchHLE_apps";
    private static final int REQ_LEGACY_STORAGE = 1001;

    private String autoLaunchPath = null;

    @Override
    protected String[] getLibraries() {
        return new String[]{
            "SDL2",
            "touchHLE"
        };
    }

    @Override
    protected String[] getArguments() {
        if (autoLaunchPath == null) {
            Log.w(TAG, "getArguments() called with autoLaunchPath=null; falling back to picker");
            return new String[0];
        }
        // Match the flags the Windows launcher uses for Chaos Rings — see
        // run_ios3.cmd. --disable-direct-memory-access forces every guest
        // memory access through the callback path, avoiding spurious access
        // violations from JIT'd direct page-table reads.
        String[] args = new String[]{ autoLaunchPath, "--disable-direct-memory-access" };
        Log.i(TAG, "getArguments() => " + Arrays.toString(args));
        return args;
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        Log.i(TAG, "MainActivity.onCreate() entered");

        if (!ensureStorageAccess()) {
            // ensureStorageAccess has already shown UI / launched a settings
            // intent. Don't kick off SDL: super.onCreate is what spins up the
            // native thread, and the native side would crash trying to write
            // to /sdcard/ChaosRing without permission.
            Log.w(TAG, "Storage access not granted; finishing without starting SDL");
            finish();
            return;
        }

        try {
            autoLaunchPath = findAppToAutoLaunch();
            Log.i(TAG, "findAppToAutoLaunch returned: " + autoLaunchPath);
        } catch (Throwable t) {
            Log.e(TAG, "Failed to look up app; falling back to picker", t);
        }

        super.onCreate(savedInstanceState);
    }

    /**
     * Returns true if /sdcard/ChaosRing/ is usable. If the app doesn't have
     * sufficient permission to write there, this method shows a Toast and
     * launches the appropriate Settings page so the user can grant it.
     */
    private boolean ensureStorageAccess() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            if (!Environment.isExternalStorageManager()) {
                Toast.makeText(this,
                    "Chaos Ring needs \"All files access\" to read its data folder at " + DATA_DIR,
                    Toast.LENGTH_LONG).show();
                try {
                    Intent intent = new Intent(
                        Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                        Uri.parse("package:" + getPackageName()));
                    startActivity(intent);
                } catch (Exception e) {
                    Log.e(TAG, "Couldn't open per-app all-files-access settings", e);
                    startActivity(new Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION));
                }
                return false;
            }
        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            // API 23-29: runtime permission required.
            if (checkSelfPermission(Manifest.permission.WRITE_EXTERNAL_STORAGE)
                    != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(
                    new String[]{ Manifest.permission.WRITE_EXTERNAL_STORAGE },
                    REQ_LEGACY_STORAGE);
                // requestPermissions is async; the user will need to relaunch
                // after granting. Bail out of this onCreate for now.
                return false;
            }
        }
        // API 21-22: WRITE_EXTERNAL_STORAGE is granted at install time, nothing
        // to check at runtime.

        File dataDir = new File(DATA_DIR);
        if (!dataDir.exists() && !dataDir.mkdirs()) {
            Log.e(TAG, "Could not create data dir " + dataDir);
            Toast.makeText(this,
                "Could not create " + DATA_DIR + ". Check storage permissions.",
                Toast.LENGTH_LONG).show();
            return false;
        }
        if (!dataDir.canWrite()) {
            Log.e(TAG, "Data dir is not writable: " + dataDir);
            Toast.makeText(this,
                DATA_DIR + " is not writable. Check storage permissions.",
                Toast.LENGTH_LONG).show();
            return false;
        }
        return true;
    }

    /**
     * Look in /sdcard/ChaosRing/touchHLE_apps/ for a bundle and return the
     * path of the first one (sorted by name), or null if there isn't one yet.
     */
    private String findAppToAutoLaunch() {
        File appsDir = new File(DATA_DIR, APPS_SUBDIR);
        if (!appsDir.isDirectory()) {
            // Create it so the user has somewhere obvious to drop the IPA.
            if (!appsDir.mkdirs()) {
                Log.w(TAG, "Could not create " + appsDir);
            }
            return null;
        }
        File[] entries = appsDir.listFiles();
        if (entries == null) return null;

        ArrayList<String> bundles = new ArrayList<>();
        for (File f : entries) {
            String lower = f.getName().toLowerCase();
            if (lower.endsWith(".ipa") || lower.endsWith(".app")) {
                bundles.add(f.getAbsolutePath());
            }
        }
        if (bundles.isEmpty()) return null;
        bundles.sort(String::compareTo);
        return bundles.get(0);
    }
}
