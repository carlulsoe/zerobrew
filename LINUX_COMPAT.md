# Linux Compatibility for zerobrew

## Overview

This document analyzes what's needed to port zerobrew from macOS to Linux. The good news: it's quite feasible! The codebase is already well-structured with `#[cfg(target_os = "macos")]` guards, and most Linux changes are additive.

## Current macOS-Specific Code

### 1. APFS clonefile (`zb_io/src/materialize.rs`)

**Location:** Lines 408-440

```rust
#[cfg(target_os = "macos")]
fn try_clonefile_dir(src: &Path, dst: &Path) -> io::Result<()> {
    // Uses macOS clonefile() syscall for zero-copy directory cloning
}
```

**What it does:** APFS clonefile creates copy-on-write clones of directories instantly, with zero disk overhead until files are modified.

**Linux equivalent:** 
- **btrfs/XFS:** Use `ioctl_ficlone` (FICLONE ioctl) for file-level reflinks
- **ext4:** No reflink support — fall back to regular copy (already implemented!)

**Implementation approach:**
```rust
#[cfg(target_os = "linux")]
fn try_reflink_dir(src: &Path, dst: &Path) -> io::Result<()> {
    // Walk directory, use ioctl(FICLONE) per file on btrfs/XFS
    // Falls back gracefully if filesystem doesn't support reflinks
}
```

**Effort:** Medium. The fallback (hardlink → copy) already exists, so Linux will work immediately — reflinks are a performance optimization.

### 2. Mach-O Binary Patching (`zb_io/src/materialize.rs`)

**Location:** Lines 127-320

```rust
#[cfg(target_os = "macos")]
fn patch_homebrew_placeholders(...) { ... }

#[cfg(target_os = "macos")]
fn codesign_and_strip_xattrs(...) { ... }
```

**What it does:**
- Patches `@@HOMEBREW_CELLAR@@` and `@@HOMEBREW_PREFIX@@` placeholders in Mach-O binaries using `install_name_tool`
- Strips quarantine xattrs (`com.apple.quarantine`, `com.apple.provenance`)
- Ad-hoc codesigns binaries

**Linux equivalent:**
- **ELF patching:** Use `patchelf` to modify rpaths/interpreter paths
- **No xattrs:** Linux doesn't have quarantine xattrs
- **No codesigning:** Linux doesn't require ad-hoc signing

**Implementation approach:**
```rust
#[cfg(target_os = "linux")]
fn patch_homebrew_placeholders_linux(keg_path: &Path, cellar_dir: &Path) -> Result<(), Error> {
    // Use patchelf to fix:
    // - RPATH (runpath): points to correct library locations
    // - Interpreter: ensures correct ld-linux.so path
    // Replace @@HOMEBREW_CELLAR@@ and @@HOMEBREW_PREFIX@@ in rpaths
}
```

**Effort:** Medium. May need to shell out to `patchelf` (like macOS shells out to `install_name_tool`), or use the `goblin` crate for pure Rust ELF parsing.

### 3. Bottle Selection (`zb_core/src/bottle.rs`)

**Location:** Lines 8-50

```rust
pub fn select_bottle(formula: &Formula) -> Result<SelectedBottle, Error> {
    let macos_tags = ["arm64_tahoe", "arm64_sequoia", "arm64_sonoma", "arm64_ventura"];
    // ... only selects macOS bottles
}
```

**What it does:** Selects ARM64 macOS bottles, explicitly excluding Linux.

**Linux equivalent:** Select Linux bottles based on architecture:

```rust
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const LINUX_TAGS: &[&str] = &["arm64_linux"];

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const LINUX_TAGS: &[&str] = &["x86_64_linux"];
```

**Verified:** Homebrew API includes Linux bottles (confirmed with `jq` formula):
- `arm64_linux` ✓
- `x86_64_linux` ✓

**Effort:** Easy. Just conditional compilation with the right tags.

### 4. Default Paths (`zb_core/src/context.rs`)

**Location:** Line 72

```rust
paths: Paths::from_root(PathBuf::from("/opt/zerobrew")),
```

**Issue:** `/opt/zerobrew` is fine for Linux too, but:
- Linuxbrew traditionally uses `/home/linuxbrew/.linuxbrew`
- User installs might prefer `~/.local/zerobrew`

**Implementation approach:**
```rust
impl Context {
    pub fn from_defaults() -> Self {
        #[cfg(target_os = "macos")]
        let default_root = PathBuf::from("/opt/zerobrew");
        
        #[cfg(target_os = "linux")]
        let default_root = PathBuf::from("/opt/zerobrew"); // or respect $ZEROBREW_ROOT
        
        // ...
    }
}
```

**Effort:** Trivial. Current path works fine on Linux.

## Summary of Changes

| Component | macOS | Linux | Effort |
|-----------|-------|-------|--------|
| Clonefile | APFS clonefile | FICLONE ioctl (btrfs/XFS) or fallback | Medium |
| Binary patching | install_name_tool + codesign | patchelf | Medium |
| Bottle selection | arm64_* macOS tags | arm64_linux / x86_64_linux | Easy |
| Xattr stripping | com.apple.quarantine | N/A (skip) | Trivial |
| Default paths | /opt/zerobrew | /opt/zerobrew | Already works |
| Symlinks | std::os::unix::fs::symlink | Same (Unix) | Already works |

## Implementation Plan

### Phase 1: Basic Linux Support (MVP)

1. **Modify `select_bottle()`** to detect Linux and select appropriate bottles
2. **Add `#[cfg(target_os = "linux")]` stubs** for macOS-only functions (no-ops initially)
3. **Test with simple packages** like `jq` (no complex dependencies)

This gets zerobrew working on Linux with copy-based installs (no reflinks).

### Phase 2: Linux Binary Patching

1. **Implement ELF patching** using `patchelf` or `goblin` crate
2. **Handle Linuxbrew placeholders** in ELF rpaths
3. **Test with packages** that have shared library dependencies

### Phase 3: Performance Optimizations

1. **Implement reflink support** for btrfs/XFS filesystems
2. **Add filesystem detection** to choose optimal copy strategy
3. **Benchmark** against Linuxbrew

## Testing Strategy

```bash
# On a Linux machine (or VM):
cargo build --release --target x86_64-unknown-linux-gnu

# Test simple package
./target/release/zb install jq

# Test package with dependencies
./target/release/zb install ripgrep
```

## Potential Issues

### 1. Glibc Compatibility
Linuxbrew bottles are built against specific glibc versions. Older systems might have issues.

### 2. Missing Dependencies
Some bottles assume Linuxbrew-provided libraries. May need to fall back to system packages or warn users.

### 3. ARM64 Linux Coverage
Not all packages have `arm64_linux` bottles. May need graceful fallback messages.

## Implementation Status

### ✅ Completed (this branch)

1. **Bottle selection** (`zb_core/src/bottle.rs`)
   - Added `get_platform_tags()` with conditional compilation for macOS/Linux × ARM64/x86_64
   - Added `is_compatible_fallback_tag()` for fallback selection
   - Supports `arm64_linux` and `x86_64_linux` bottles

2. **Reflink support** (`zb_io/src/materialize.rs`)
   - Added `try_reflink_copy_dir()` using FICLONE ioctl
   - Graceful fallback when filesystem doesn't support reflinks
   - Preserves permissions and symlinks

### ⏳ Not Implemented (future work)

1. **ELF binary patching**
   - Linuxbrew bottles may have placeholders that need patching
   - Would require `patchelf` or `goblin` crate
   - Many simple packages work without this

## Conclusion

Linux support for zerobrew is **achievable with moderate effort**. The codebase is already structured for cross-platform support with clean `#[cfg]` guards.

### What's done:
- ✅ Bottle selection for Linux (implemented)
- ✅ Reflink optimization (implemented)

### What's left:
- ⏳ ELF binary patching (4-8 hours, needed for complex packages)
- ⏳ Testing on various Linux distros
- ⏳ CI/CD for Linux builds

The code in this branch should allow zerobrew to work on Linux for simple packages (like `jq`, `ripgrep`) that don't need rpath patching.

---

*Implementation by Claude, January 2026*
*Branch: `linux-compat`*
