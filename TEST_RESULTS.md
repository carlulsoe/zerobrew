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

## ELF Binary Patching: ✅ FULLY IMPLEMENTED

The Linux ELF patching (equivalent to macOS `install_name_tool` + `codesign`) is complete:

| Feature | macOS | Linux |
|---------|-------|-------|
| Path patching | `install_name_tool -change` | `patchelf --set-rpath` |
| ID patching | `install_name_tool -id` | (not needed for ELF) |
| Interpreter | N/A | `patchelf --set-interpreter` |
| Signing | `codesign` | N/A |
| Parallel | rayon | rayon |

### What Gets Patched
1. **RPATH/RUNPATH** - Library search paths in ELF binaries
2. **Interpreter** - Dynamic linker path (`/lib/ld-linux-aarch64.so.1`)
3. **Placeholders** - `@@HOMEBREW_CELLAR@@` → actual cellar path
4. **Version mismatches** - Fixes wrong version references in paths

## Package Installation Tests

All packages tested work correctly:

### Simple packages
| Package | Dependencies | Status |
|---------|-------------|--------|
| jq | oniguruma | ✅ Works |
| tree | none | ✅ Works |
| ripgrep | pcre2 | ✅ Works |
| fd | none | ✅ Works |

### Complex package: ffmpeg (9 dependencies)
```
$ sudo ./target/release/zb install ffmpeg
==> Installing ffmpeg...
==> Resolving dependencies (9 packages)...
    dav1d 1.5.3
    lame 3.100
    libvpx 1.15.2
    opus 1.6.1
    sdl2 2.32.10
    svt-av1 4.0.0
    x264 r3222
    x265 4.1
    ffmpeg 8.0.1
==> Installed 9 packages in 7.39s

$ ffmpeg -version
ffmpeg version 8.0.1 Copyright (c) 2000-2025 the FFmpeg developers

$ ffmpeg -f lavfi -i testsrc=duration=1 -c:v libx264 test.mp4
✅ Video encoding works!
```

### RPATH verification
```
$ patchelf --print-rpath .../ffmpeg
/opt/zerobrew/prefix/Cellar/ffmpeg/8.0.1_2/lib:/opt/zerobrew/prefix/opt/dav1d/lib:...

$ patchelf --print-interpreter .../ffmpeg
/lib/ld-linux-aarch64.so.1

$ ldd .../ffmpeg | grep "not found"
(no output - all libraries resolve correctly)
```

## What Works
- ✅ Platform detection (arm64_linux bottles selected correctly)
- ✅ Bottle downloads from Linuxbrew
- ✅ Tarball extraction (xz, gzip, zstd)
- ✅ Dependency resolution
- ✅ Package installation to store
- ✅ Symlink creation
- ✅ Package uninstallation
- ✅ Reflink copy with ext4 fallback (FICLONE ioctl)
- ✅ **ELF binary patching (interpreter + RPATH)**
- ✅ **Complex packages with many shared libraries**

## Requirements

- **patchelf** must be installed for binaries to work
  ```bash
  # From releases:
  curl -L https://github.com/NixOS/patchelf/releases/download/0.18.0/patchelf-0.18.0-aarch64.tar.gz | tar xz
  sudo cp bin/patchelf /usr/local/bin/
  
  # Or via nix:
  nix-shell -p patchelf
  ```
- Without patchelf, packages install but binaries won't run (code gracefully skips patching)

## Platform Support

| Architecture | Interpreter | Status |
|-------------|-------------|--------|
| aarch64 | `/lib/ld-linux-aarch64.so.1` | ✅ Tested |
| x86_64 | `/lib64/ld-linux-x86-64.so.2` | Implemented |

## Implementation Details

### File: `zb_io/src/materialize.rs`

```rust
#[cfg(target_os = "linux")]
fn patch_homebrew_placeholders_linux(...) {
    // 1. Check patchelf availability
    // 2. Find ELF files by magic bytes (0x7f 'E' 'L' 'F')
    // 3. Parallel process with rayon
    // 4. For each ELF:
    //    - Read RPATH with patchelf --print-rpath
    //    - Replace @@HOMEBREW_CELLAR@@ and @@HOMEBREW_PREFIX@@
    //    - Fix version mismatches in paths
    //    - Apply with patchelf --set-rpath
    //    - Read interpreter with patchelf --print-interpreter
    //    - Set to system loader with patchelf --set-interpreter
}
```

## Notes

- First build requires `libssl-dev` for native TLS
- Cached store entries from before patchelf may need clearing
- Some very complex packages may need additional testing
