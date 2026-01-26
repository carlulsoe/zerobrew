# Linux Support Test Results

**Date:** 2026-01-26  
**Platform:** Raspberry Pi 5 (aarch64-unknown-linux-gnu)  
**OS:** Raspberry Pi OS (Debian trixie)  
**Rust:** 1.93.0

## Build Status: ✅ SUCCESS

```
cargo build --release
Finished `release` profile [optimized] in 2m 48s
```

Required dependency: `libssl-dev` (via apt)

## Package Installation Tests

### jq (with dependency: oniguruma)
- **Download:** ✅ SUCCESS
- **Extraction:** ✅ SUCCESS  
- **Dependency resolution:** ✅ SUCCESS (correctly pulled oniguruma)
- **Binary execution:** ❌ FAILS (ELF patching needed)

```
$ sudo ./target/release/zb install jq
==> Installing jq...
==> Resolving dependencies (2 packages)...
    oniguruma 6.9.10
    jq 1.8.1
==> Downloading and installing...
==> Installed 2 packages in 1.28s
```

### tree (no dependencies)
- **Download:** ✅ SUCCESS
- **Extraction:** ✅ SUCCESS
- **Binary execution:** ❌ FAILS (ELF patching needed)

### Uninstall
- **jq:** ✅ SUCCESS
- **oniguruma:** ✅ SUCCESS
- **tree:** ✅ SUCCESS

## Root Cause of Execution Failures

All Linuxbrew bottles contain placeholder paths that must be patched:

```
$ readelf -l /opt/zerobrew/prefix/Cellar/jq/1.8.1/bin/jq | grep interpreter
[Requesting program interpreter: @@HOMEBREW_PREFIX@@/lib/ld.so]
```

The `@@HOMEBREW_PREFIX@@` placeholder needs to be replaced with the actual path (`/opt/zerobrew/prefix`) using `patchelf`.

### Affected binary attributes:
1. **Interpreter (PT_INTERP):** `@@HOMEBREW_PREFIX@@/lib/ld.so` → should be `/lib/ld-linux-aarch64.so.1`
2. **RPATH:** Contains placeholder paths → should be updated to actual lib paths

## What Works
- ✅ Platform detection (arm64_linux bottles selected correctly)
- ✅ Bottle downloads from Linuxbrew
- ✅ Tarball extraction
- ✅ Dependency resolution
- ✅ Package installation to store
- ✅ Symlink creation
- ✅ Package uninstallation
- ✅ Reflink copy with ext4 fallback

## What Needs Future Work
- ❌ ELF binary patching via patchelf (required for all Linuxbrew bottles)
- ❌ Automatic interpreter/rpath fixup post-install

## Recommendation

The Linux bottle selection and reflink support work correctly. To make packages actually runnable, zerobrew needs to:

1. Install/bundle `patchelf`
2. After extracting bottles, run:
   ```bash
   patchelf --set-interpreter /lib/ld-linux-aarch64.so.1 <binary>
   patchelf --set-rpath '$ORIGIN/../lib:/opt/zerobrew/prefix/lib' <binary>
   ```

This is a known limitation of Linuxbrew bottles and affects all tools that consume them without running Homebrew's relocation logic.
