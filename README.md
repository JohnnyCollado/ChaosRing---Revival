# ChaosRing — touchHLE patched to actually play *Chaos Rings*

A fork of [**touchHLE**](https://touchhle.org/) — the high-level emulator for
early iPhone OS apps — patched until Square Enix's *Chaos Rings* (2010)
loads, plays, fights battles, and saves on Windows.

This is **not a general touchHLE improvement** in the upstream sense. It is
one game's compatibility surface, dragged kicking and screaming into a state
where the game runs. Several of the fixes are useful for any iOS 3.0-era
app; others are stubs sufficient for this specific binary.

The journey, the fixes, and how to run it are below. The original touchHLE
license, attributions, and platform info are at the bottom — none of that
changed.

---

## Quick start

Build once (~10 minutes on first build for the C++ dependencies), then:

```
run_ios3.cmd            # iPhone build (2.2.0, iOS 3.0)
run_ios4.cmd            # iPad / HD build (2.3.0, iOS 4.3)
```

Both versions load saves, play battles, and write saves cleanly. Same
fixes apply to both — the games share the C++ codebase, only the assets
differ.

Saves live in `touchHLE_sandbox\com.square-enix.ChaosRings\Documents\data.dat`.

---

## The journey

We started with no `.git`, empty `vendor/` submodules, no Boost, and a
half-remembered build doc. Several hours later, the game launches, loads a
save, plays a battle, and writes a new save. Between those two states is a
trail of dead ends, a lot of stack traces, and one very satisfying
breakthrough at the end.

### 0. Dependency archaeology

The repo was extracted from a zip — no `.git`, so the submodule SHAs were
gone. We cloned each vendor (`dynarmic`, `openal-soft`, `SDL`, `stb`)
manually, then pinned each to the SHA that upstream `touchHLE/trunk`
references via the GitHub API. Then downloaded Boost 1.81.0 (the CI's
version) into `vendor/boost/`.

Git on Windows had its usual SSL-cert-bundle problem; we used per-command
`-c http.sslBackend=schannel` instead of touching the global config. Cargo
also needed `CARGO_HTTP_CHECK_REVOKE=false` because the corporate-style
revocation check couldn't reach OCSP.

A clean release build came together in ~2.5 minutes after dynarmic and
openal-soft had compiled once via CMake.

### 1. The first crash: ARC

Launched the iPad iOS 4.3 build first. Instant null-page access at `0x0`,
right after the binary loaded. The smoking gun in the dyld warnings:

```
Warning: unhandled non-lazy symbol "_objc_retain" at 0xb4550 in "ChaosRingsHD"
```

The binary was compiled with **ARC** — every Objective-C strong-pointer
assignment compiles down to a direct call to `objc_retain` / `objc_release`
/ etc. touchHLE didn't implement any of them. The unresolved symbol slot
stayed `0`, the binary later called through it, → branch to address 0 →
"null-page access at 0x0".

Added the full ARC runtime to `src/objc.rs`: 17 functions covering
`objc_retain`, `objc_release`, `objc_autorelease`,
`objc_autoreleaseReturnValue`, `objc_retainAutoreleasedReturnValue`,
`objc_retainAutorelease`, `objc_storeStrong`, `objc_retainBlock`,
`objc_unsafeClaimAutoreleasedReturnValue`, and the weak family
(`storeWeak`, `loadWeak`, `loadWeakRetained`, `destroyWeak`, `copyWeak`,
`moveWeak`, `initWeak`).

The strong family delegates to touchHLE's existing `retain`/`release`/
`autorelease` Obj-C message helpers. The weak family is degraded (stores
without zeroing-on-dealloc), but most games don't depend on the zeroing
semantics.

### 2. Stack canary + dyld stub binder

After ARC, the next unresolved relocations were `___stack_chk_guard` and
`dyld_stub_binder`. Stack canary is a global `int` for stack-protector
prologues; dyld_stub_binder is the lazy-binding helper.

Added [`src/libc/stack_chk.rs`](src/libc/stack_chk.rs) exporting
`___stack_chk_guard` as a 4-byte constant `0xC0DEFACE` and `__stack_chk_fail`
as a panic. Added a `dyld_stub_binder` stub that panics if invoked — it
should never be, because touchHLE handles lazy binding via SVC instead.

### 3. Block runtime

Next unresolved symbols were `__NSConcrete{Global,Stack,Malloc}Block`,
`_Block_copy`, `_Block_release`. Clang Block ABI — iOS 4+ uses these for
closures.

Added a minimal block runtime: the three `_NSConcrete*Block` "isa"
sentinels as 4-byte allocations, `_Block_copy` with stack→heap promotion
(reads `Block_size` from the descriptor, memcpy, runs `copy_helper` if
`BLOCK_HAS_COPY_DISPOSE` is set), and `_Block_release` as a no-op (leaks).
For one play session the leak doesn't matter; for a long-running app, this
would need refcount + dispose-helper invocation.

Also added a `pub fn invoke_block_no_args` helper for hosts that want to
call a block — reads the `invoke` pointer at offset 12 of the layout, calls
it as a `GuestFunction` with the block as r0.

### 4. The OES skinning extension

The game tries to enable `GL_OES_matrix_palette` (`0x8840`) and friends —
hardware vertex skinning, used for animating character meshes. touchHLE's
GLES1-on-GL2 layer has no idea what these enums are and panics.

Made `glEnable` / `glDisable` / `glEnableClientState` /
`glDisableClientState` tolerate the OES skinning enums (`0x8840`–`0x8843`,
`0x86AD`, `0x8844`) by warning + skipping. Also skip the underlying
`gl21::*` call so spurious `GL_INVALID_ENUM` doesn't leak through the
guest's `glGetError`. Animated meshes render in bind pose; static
geometry is unaffected.

### 5. NSOperationQueue: the deadlock

Game now allocates an `NSOperationQueue` and submits a `CRoperation` (a
custom NSOperation subclass — there are only two app classes in the whole
binary, this one and a `CRView`). touchHLE didn't have NSOperationQueue.

Added a synchronous `NSOperation` / `NSBlockOperation` / `NSInvocationOperation`
/ `NSOperationQueue` stub. `addOperation:` immediately ran `[op start]`.

Game then deadlocked silently. Cranked up message-dispatch tracing
(`touchHLE_log.txt` filled with thousands of lines) and the smoking gun
appeared:

```
Dispatching sleepForTimeInterval: for 0x30010130
Dispatching sleepForTimeInterval: for 0x30010130
Dispatching sleepForTimeInterval: for 0x30010130
   ... ad infinitum ...
```

`CRoperation.main` loops:

```objc
- (void)main {
    [self kickOffWork];
    while (!self.done) {
        [NSThread sleepForTimeInterval:0.001];
    }
}
```

It expects to run on a background thread while the main thread sets
`self.done` via UI events. With a synchronous stub, both "threads" are the
same thread — main is blocked inside `addOperation:` forever.

Made `NSOperationQueue.addOperation:` spawn a real `pthread_create` (which
in touchHLE means a corosensei coroutine, but it's a separate one with its
own scheduling). `waitUntilFinished` polls `isFinished` with host-side
sleep.

### 6. mmap offset assertion

Game now spun up the worker thread, started reading audio files, and
panicked at:

```
assertion `left == right` failed
  left: 88059904
 right: 0
   at src\libc\sys\mman.rs:53
```

`mmap` had `assert_eq!(offset, 0)` even though the code immediately below
correctly did `lseek(fd, offset, SEEK_SET) + read(...)` to handle non-zero
offsets. The assertion was a leftover. Removed it.

### 7. The game ran

It actually ran. Title screen, intro, gameplay, a battle. Then:

### 8. The post-battle crash

After winning a battle, the process dies with a hard Windows access
violation:

```
Faulting module: VCRUNTIME140.dll
Exception code: 0xC0000005
Fault offset:   0x000000000000f237
```

`memcpy` in the C runtime. No Rust panic in the log. Just gone.

This was the dead-end that ate the rest of the afternoon. We tried, in
order:

1. **Bumping the corosensei coroutine stack from 1 MB to 8 MB.** The
   default `DefaultStack` only `MEM_COMMIT`s 4 KB regardless of reserved
   size and grows on demand via `_chkstk` and guard pages. We thought SEH
   unwinding was running off the committed region.

2. **Writing a custom `Stack` provider that pre-commits the entire 8 MB
   upfront** with a single guard page at the bottom for overflow
   detection. The whole new module is [`src/coroutine_stack.rs`](src/coroutine_stack.rs).

3. **Wrapping dynarmic's emulation entry in C++ `try/catch`** in
   [`src/cpu/dynarmic_wrapper/lib.cpp`](src/cpu/dynarmic_wrapper/lib.cpp)
   — any thrown exception would be reported and `abort()`ed cleanly.

4. **Instrumenting `MemoryRead*` / `MemoryWrite*` callbacks** to log the
   guest PC + failing address.

5. **Passing `--disable-direct-memory-access`** to force every JIT
   memory access through the safe callback path instead of the direct
   page-table read.

6. **Reverse-engineering the binary.** Extracted the Mach-O, scanned
   strings, found that the entire app surface is two Obj-C classes
   (`CRoperation`, `CRView`) wrapping a C++ codebase organised into
   `Classes/Fld/`, `Classes/Puzzle/`, `Classes/CmpLib/`, `Classes/Base/`.
   No save manager class to intercept.

7. **Installing a `SetUnhandledExceptionFilter` + vectored exception
   handler** ([`src/crash_handler.rs`](src/crash_handler.rs)) to capture
   register state, stack memory, and symbol-resolved stack frames on hard
   crashes. Output goes to `touchHLE_crash.txt`.

None of these fixed the crash. All of them made the crash *visible* in
detail — but the actual stack frames at the time of the AV were all
inside Microsoft's exception-unwinding machinery (`_CxxFrameHandler3`,
`RtlUnwindEx`, `set_se_translator`, `is_exception_typeof`), not anything
identifiable as the crash origin.

### 9. The actual root cause

Eventually noticed something: `pub fn main` (the Windows desktop entry
point) didn't install a panic hook. Only `SDL_main` (the Android entry)
did. So **every Rust panic on Windows printed to stderr and was lost**
when the cmd window closed.

Installed the same panic-hook in `pub fn main`. Re-ran. The log
immediately revealed:

```
Panic at src\frameworks\foundation\ns_locale.rs:162:5: assertion `left == right` failed
  left: 2
 right: 5
```

That's it. That's the whole "SEH cascade through corosensei" we had been
chasing for hours. Every "post-battle crash", every "save-load crash",
every "startup crash with a save present" — **all the same Rust panic**,
which propagated up through corosensei's `extern "C"` `coroutine_func`
(`noexcept` on MSVC) and detonated as Windows SEH unwinding through a
`noexcept` boundary, which manifests as the AV-in-VCRUNTIME we kept seeing.

The actual bug was in touchHLE's `NSLocale.initWithLocaleIdentifier:`:

```rust
assert_eq!(2, str.len());          // panics on "en_US"
assert!(str.to_lowercase().eq(&str)); // panics on "ja_JP"
assert!(!str.contains('_'));        // panics on "en_US"
```

The game passes `"en_US"` or `"ja_JP"`. The asserts hardcoded "2-char
lowercase, no separator" — fine for an ISO 639 language code, wrong for
real locale identifiers.

Replaced the asserts with a real parser: take the prefix up to the first
`_`, `-`, or `@`; lowercase it; that's the language code; store the full
original identifier as the locale identifier value.

### 10. Convergence

With the panic hook on, each remaining missing piece became a one-line
diagnosis and a small fix:

- **`-[NSDateFormatter setLocale:]`** — game configures date formatters
  with a locale; touchHLE didn't have the setter. Added it plus
  `setTimeZone:` / `setDateStyle:` / `setTimeStyle:` as no-ops.
- **`-[NSDateFormatter stringFromDate:]` with no format set** —
  unwrapped a `None`. Added a fallback `"yyyy-MM-dd HH:mm:ss"` for guests
  using `setDateStyle:` (which we ignore) instead of `setDateFormat:`.
- **`+[NSCalendar currentCalendar]`** — entire `NSCalendar` class was
  missing. Added a minimal stub plus `NSDateComponents` with all the
  standard properties. `components:fromDate:` extracts Gregorian
  components via the existing `CFAbsoluteTimeGetGregorianDate`.

After NSCalendar, the post-battle save flow ran clean.

---

## Summary of what changed

| Area | File | What |
|---|---|---|
| ARC runtime | [src/objc.rs](src/objc.rs) | 17 functions: `objc_retain`/`release`/`autorelease`/`storeStrong`/`retainBlock`/weak family |
| Block runtime | [src/objc.rs](src/objc.rs) | `_Block_copy` (stack→heap), `_Block_release`, `_NSConcrete{Global,Stack,Malloc}Block` |
| Stack canary | [src/libc/stack_chk.rs](src/libc/stack_chk.rs) (new) | `___stack_chk_guard` + `__stack_chk_fail` |
| GLES skinning tolerance | [src/gles/gles1_on_gl2.rs](src/gles/gles1_on_gl2.rs) | Tolerate `GL_OES_matrix_palette` family |
| NSOperationQueue (async) | [src/frameworks/foundation/ns_operation.rs](src/frameworks/foundation/ns_operation.rs) (new) | NSOperation / NSBlockOperation / NSInvocationOperation / NSOperationQueue with real-pthread `addOperation:` |
| mmap non-zero offset | [src/libc/sys/mman.rs](src/libc/sys/mman.rs) | Remove overly-strict offset==0 assertion |
| Full NSFileManager attrs | [src/frameworks/foundation/ns_file_manager.rs](src/frameworks/foundation/ns_file_manager.rs) | Return 14 attribute keys instead of 3 |
| Pre-committed coroutine stack | [src/coroutine_stack.rs](src/coroutine_stack.rs) (new) | All coroutine stacks `MEM_COMMIT`ed upfront for Windows SEH safety |
| dynarmic try/catch | [src/cpu/dynarmic_wrapper/lib.cpp](src/cpu/dynarmic_wrapper/lib.cpp) | Catch C++ exceptions at the emulation boundary |
| Crash handler | [src/crash_handler.rs](src/crash_handler.rs) (new) | Captures Windows hard-crash state to `touchHLE_crash.txt` |
| Panic hook on desktop | [src/lib.rs](src/lib.rs) | `pub fn main` installs the panic hook (previously only `SDL_main` did) |
| NSLocale identifier parsing | [src/frameworks/foundation/ns_locale.rs](src/frameworks/foundation/ns_locale.rs) | Real-world identifier parsing instead of 2-char assertion |
| NSDateFormatter setters | [src/frameworks/foundation/ns_date_formatter.rs](src/frameworks/foundation/ns_date_formatter.rs) | `setLocale:`/`setTimeZone:`/`setDateStyle:`/`setTimeStyle:` + default format fallback |
| NSCalendar + NSDateComponents | [src/frameworks/foundation/ns_calendar.rs](src/frameworks/foundation/ns_calendar.rs) (new) | Minimal stubs sufficient for save-timestamp use |

---

## Building from source

Prereqs (mostly inherited from upstream touchHLE):

- Rust toolchain (`rustup default stable`)
- CMake 3.20+
- A C/C++ toolchain — MSVC on Windows
- The vendored Boost extracted to `vendor/boost/` (`boost_1_81_0` works)
- Submodules populated in `vendor/dynarmic`, `vendor/SDL`, `vendor/openal-soft`,
  `vendor/stb`

Then:

```
cargo build --release
```

If cargo fails fetching crates with an SSL revocation error on Windows,
set:

```
$env:CARGO_HTTP_CHECK_REVOKE = "false"
```

before the build. (Underlying issue: schannel can't reach the OCSP server.
Doesn't affect cert chain validation.)

## Building distro packages

Two PowerShell scripts in `dev-scripts/` produce portable zips you can hand
to other people without making them install a Rust toolchain.

### Windows distro

```
dev-scripts\make-windows-distro.ps1
```

Produces `dist\ChaosRing-Windows-x64.zip` (~16 MB). Contains `touchHLE.exe`,
`touchHLE_dylibs/`, `touchHLE_fonts/`, both `.cmd` launchers, license files,
and an empty `touchHLE_apps/` directory with a README explaining where to
drop the user's `.ipa`. Pass `-SkipBuild` to reuse an existing
`target/release/touchHLE.exe`.

### Android distro

```
dev-scripts\make-android-distro.ps1
```

Cross-compiles touchHLE for `aarch64-linux-android`, then runs the
upstream Gradle project in `android/` to build a release APK. Produces
`dist\ChaosRing-Android-AArch64.zip` containing the APK, licenses, and a
`RUNNING.md` for end users.

Prereqs for the Android build (the script verifies each one and bails with
a clear message if anything is missing):

- JDK 11+ (`java` on PATH)
- Android SDK with `ANDROID_HOME` set
- **Android NDK r25c specifically** (version `25.2.9519653`) — newer NDKs
  (27+) ship Clang 17 which fails to compile dynarmic's vendored `fmt`
  library with "call to consteval function" errors. Install via Android
  Studio's SDK Manager (SDK Tools → "Show Package Details" → NDK (Side by
  side) → check 25.2.9519653), or `sdkmanager --install "ndk;25.2.9519653"`.
  The script auto-detects and aborts early with this message if a newer
  NDK is the only one present.
- `cargo install cargo-ndk@3.5.4`
- `rustup target add aarch64-linux-android`
- Gradle 8.11.1+ — the script downloads it locally to `gradle/` if it's
  not already on PATH

End users install the APK and drop their `.ipa` into
`Android/data/org.touchhle.android/files/touchHLE_apps/`. The
`RUNNING.md` inside the zip explains the details.

---

## Known limitations

- **OES matrix-palette skinning is tolerated, not implemented.** Animated
  character meshes render in their bind pose. Static geometry is fine.
- **Block runtime leaks.** `_Block_release` is a no-op. Fine for short
  sessions; not for long-running apps.
- **Weak references don't zero on dealloc.** `objc_storeWeak` stores a
  raw pointer. Most games don't notice; some might.
- **NSOperationQueue ignores dependencies and `maxConcurrentOperationCount`.**
  One pthread per operation, FIFO from the OS scheduler. Good enough for
  Chaos Rings' single-CRoperation pattern.
- **NSCalendar is Gregorian-only and ignores time zones.** Calendar-system
  conversions and DST won't work correctly.
- **iPad iOS 4.3 build (`run_ios4.cmd`) is more memory-intensive** than
  the iPhone iOS 3.0 build (larger HD assets, more audio buffers).
  Confirmed playable end-to-end on this fork; just heavier on RAM.

---

## Original touchHLE info

**touchHLE** is a high-level emulator for iPhone OS apps, written in Rust.
This repository is a fork that's been tweaked to play one specific game on
Windows. All the heavy lifting is upstream's — the entire emulator, the
Objective-C runtime, dyld, every framework, the dynarmic integration, the
GLES1-on-GL2 layer.

For the full upstream README — platform support matrix, input methods,
development status, contributor list, and so on — see
<https://github.com/touchHLE/touchHLE> and <https://touchhle.org/>.

### Disclaimer

This project is not affiliated with or endorsed by Apple Inc in any way.
iPhone, iOS, iPod, iPod touch, and iPad are trademarks of Apple Inc.

Only use touchHLE (or this fork) to emulate software you have obtained
legally.

### Licenses

- touchHLE itself (and our patches) are under the **Mozilla Public License,
  version 2.0**.
- Binaries distributed from this fork are under the **GNU General Public
  License version 3 or later** (inherited from upstream, due to dependency
  license compatibility concerns).
- Bundled dynamic libraries (`touchHLE_dylibs/`) and fonts
  (`touchHLE_fonts/`) have their own licenses — see those directories.

### Thanks

To the touchHLE project contributors, especially
[hikari\_no\_yume](https://hikari.noyu.me/) who started it; to the authors
of [dynarmic](https://github.com/merryhime/dynarmic), [SDL](https://libsdl.org/),
[openal-soft](https://github.com/kcat/openal-soft),
[Symphonia](https://github.com/pdeljanov/Symphonia),
[corosensei](https://github.com/Amanieu/corosensei), and the dozens of
other libraries this emulator is built on; and to the Rust project
generally. None of this would exist without their work.
