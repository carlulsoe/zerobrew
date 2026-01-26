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

All packages tested work correctly with patchelf installed:

### jq (with dependency: oniguruma)
- **Download:** ✅ SUCCESS
- **Extraction:** ✅ SUCCESS  
- **Dependency resolution:** ✅ SUCCESS (correctly pulled oniguruma)
- **Binary execution:** ✅ SUCCESS

```
$ sudo ./target/release/zb install jq
==> Installing jq...
==> Resolving dependencies (2 packages)...
    oniguruma 6.9.10
    jq 1.8.1
==> Downloading and installing...
==> Installed 2 packages in 0.34s

$ jq --version
jq-1.8.1

$ echo '{"test":123}' | jq '.test'
123
```

### tree (no dependencies)
- **Download:** ✅ SUCCESS
- **Extraction:** ✅ SUCCESS
- **Binary execution:** ✅ SUCCESS

```
$ tree --version
tree v2.2.1 © 1996 - 2024 by Steve Baker, Thomas Moore, Francesc Rocher, Florian Sesser, Kyosuke Tokoro
```

### ripgrep (with dependency: pcre2)
- **Download:** ✅ SUCCESS
- **Binary execution:** ✅ SUCCESS

```
$ rg --version
ripgrep 15.1.0
```

### fd (no dependencies)
- **Download:** ✅ SUCCESS
- **Binary execution:** ✅ SUCCESS

```
$ fd --version
fd 10.3.0
```

### Uninstall
All packages uninstall correctly: ✅ SUCCESS

## What Works
- ✅ Platform detection (arm64_linux bottles selected correctly)
- ✅ Bottle downloads from Linuxbrew
- ✅ Tarball extraction (xz, gzip, zstd)
- ✅ Dependency resolution
- ✅ Package installation to store
- ✅ Symlink creation
- ✅ Package uninstallation
- ✅ Reflink copy with ext4 fallback
- ✅ ELF binary patching (interpreter + RPATH)

## Requirements

- **patchelf** must be installed for binaries to work
  - Download from: https://github.com/NixOS/patchelf/releases
  - Without patchelf, packages install but binaries won't run

## How ELF Patching Works

Linuxbrew bottles contain placeholder paths that zerobrew patches at install time:

**Before patching:**
```
[Requesting program interpreter: @@HOMEBREW_PREFIX@@/lib/ld.so]
```

**After patching:**
```
[Requesting program interpreter: /lib/ld-linux-aarch64.so.1]
```

The patching code:
1. Detects ELF binaries by magic bytes
2. Uses patchelf to read and modify RPATH entries
3. Replaces `@@HOMEBREW_CELLAR@@` and `@@HOMEBREW_PREFIX@@` with actual paths
4. Sets the interpreter to the system dynamic linker

## Platform Support

| Architecture | Interpreter |
|-------------|-------------|
| aarch64 | `/lib/ld-linux-aarch64.so.1` |
| x86_64 | `/lib64/ld-linux-x86-64.so.2` |

## Notes

- First build requires `libssl-dev` for native TLS
- Cached store entries from before patchelf was installed will need to be cleared (`rm -rf /opt/zerobrew/store/*`)
- Some complex packages with many dynamic library dependencies may need additional RPATH fixes
