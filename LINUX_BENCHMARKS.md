# Linux Benchmark Results

Benchmarks comparing zerobrew (`zb`) vs Linuxbrew on Linux (aarch64).

## System Info

| Component | Value |
|-----------|-------|
| **OS** | Linux 6.12.47+rpt-rpi-2712 (aarch64) |
| **CPU** | Raspberry Pi 5 (4 cores @ 2.4 GHz, Cortex-A76) |
| **RAM** | 8 GB |
| **Storage** | 1.8 TB ext4 (USB SSD) |
| **Kernel** | Debian-based (Raspberry Pi OS) |

## Benchmark Methodology

- `HOMEBREW_NO_AUTO_UPDATE=1` set for fair comparison
- Fresh installs (packages uninstalled before each test)
- Warm cache (bottles already downloaded locally for brew)
- zerobrew uses 48-way parallel downloads by default

## Results: Simple Packages

| Package | Linuxbrew | zerobrew | Speedup |
|---------|-----------|----------|---------|
| **jq** (+ oniguruma) | 6.19s | 0.45s | **13.8x** |
| **ripgrep** (+ pcre2) | 10.36s | 0.68s | **15.2x** |
| **fd** | 5.73s | 0.36s | **15.9x** |
| **tree** | 4.75s | 1.26s | **3.8x** |
| **sqlite** (+ readline) | - | 0.39s | - |

**Average speedup: ~12x faster**

### Notes

- Linuxbrew sqlite was already installed, so no direct comparison
- Larger speedup for packages with more dependencies (parallel downloads shine)
- tree is smaller, so the speedup is less dramatic

## Results: Complex Packages (zerobrew only)

| Package | Dependencies | Time | Verified |
|---------|-------------|------|----------|
| **ffmpeg** | 9 packages | 2.69s | ✅ Works |
| **node** | 17 packages | 16.22s | ✅ Works |
| **python@3.12** | 7 packages | 9.47s | ✅ Works |
| **imagemagick** | 29 packages | 21.10s | ⚠️ See below |

### Verification

```bash
$ /opt/zerobrew/prefix/bin/ffmpeg -version
ffmpeg version 8.0.1 Copyright (c) 2000-2025 the FFmpeg developers

$ /opt/zerobrew/prefix/bin/node --version
v25.4.0

$ /opt/zerobrew/prefix/bin/python3.12 --version
Python 3.12.12

$ /opt/zerobrew/prefix/bin/convert --version
Version: ImageMagick 7.1.2-13 Q16-HDRI aarch64
```

## Issues Found

### 1. ImageMagick Missing Dependency

ImageMagick failed to run initially:
```
error while loading shared libraries: libxml2.so.16: cannot open shared object file
```

**Workaround:** Manually install `libxml2`:
```bash
zb install libxml2
```

This is a dependency resolution bug — `libxml2` should be included in ImageMagick's dependency chain.

### 2. Download Corruption Retries

During node installation, some bottles showed corruption warnings:
```
Corrupted download detected for sqlite, retrying (2/3)...
Corrupted download detected for zstd, retrying (2/3)...
```

The retry mechanism worked correctly, and packages installed successfully. This may be related to parallel download race conditions or network issues.

## Conclusions

1. **Performance:** zerobrew is ~12x faster than Linuxbrew for typical package installs
2. **Complex packages:** Work well, with large dependency chains (29 packages for imagemagick) installing in ~21 seconds
3. **Reliability:** Retry mechanism handles download issues gracefully
4. **Dependencies:** One missing transitive dependency found (libxml2 for imagemagick)

## Recommendations

- [ ] Fix imagemagick dependency resolution to include libxml2
- [ ] Investigate download corruption (may be bottle hosting or network issue)
- [ ] Consider adding runtime library path validation

---

*Benchmarked on 2025-01-26*
