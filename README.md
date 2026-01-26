## Install

```bash
curl -sSL https://raw.githubusercontent.com/lucasgelfond/zerobrew/main/install.sh | bash
```

After install, run the export command it prints, or restart your terminal.

# zerobrew

A faster, modern Mac package manager.

![zb demo](zb-demo.gif)

zerobrew applies [uv](https://github.com/astral-sh/uv)'s model to Mac packages. Packages live in a content-addressable store (by sha256), so reinstalls are instant. Downloads, extraction, and linking run in parallel with aggressive HTTP caching. It pulls from Homebrew's CDN, so you can    swap `brew` for `zb` with your existing commands. 

This leads to dramatic speedups, up to 5x cold and 20x warm. Full benchmarks [here](benchmark-results.txt).

| Package | Homebrew | ZB (cold) | ZB (warm) | Cold Speedup | Warm Speedup |
|---------|----------|-----------|-----------|--------------|--------------|
| **Overall (top 100)** | 452s | 226s | 59s | **2.0x** | **7.6x** |
| ffmpeg | 3034ms | 3481ms | 688ms | 0.9x | 4.4x |
| libsodium | 2353ms | 392ms | 130ms | 6.0x | 18.1x |
| sqlite | 2876ms | 625ms | 159ms | 4.6x | 18.1x |
| tesseract | 18950ms | 5536ms | 643ms | 3.4x | 29.5x | 

## Using `zb`

### Basic Commands

```bash
zb install jq             # install a package
zb install wget git       # install multiple packages
zb uninstall jq           # uninstall a package
zb list                   # list installed packages
zb info jq                # show info about a package
zb search json            # search for packages
```

### Upgrading

```bash
zb outdated               # list packages with newer versions
zb upgrade                # upgrade all outdated packages
zb upgrade jq             # upgrade a specific package
zb pin jq                 # pin a package to prevent upgrades
zb unpin jq               # unpin a package
```

### Dependencies

```bash
zb deps jq                # show dependencies
zb deps --tree jq         # dependency tree view
zb uses zlib              # show what depends on a package
zb leaves                 # list packages not depended on by others
zb autoremove             # remove orphaned dependencies
```

### Taps (Third-Party Repositories)

```bash
zb tap                    # list taps
zb tap user/repo          # add a tap
zb untap user/repo        # remove a tap
zb install user/repo/pkg  # install from a tap
```

### Services

```bash
zb services list          # list all services
zb services start redis   # start a service
zb services stop redis    # stop a service
zb services restart redis # restart a service
```

### Maintenance

```bash
zb cleanup                # remove old versions and cache
zb gc                     # garbage collect unused store entries
zb doctor                 # diagnose common issues
zb reset                  # reset zerobrew (delete all data)
```

### Linking

```bash
zb link jq                # create symlinks for a package
zb unlink jq              # remove symlinks (keeps package installed)
```

## Why is it faster?

- **Content-addressable store**: packages are stored by sha256 hash (at `/opt/zerobrew/store/{sha256}/`). Reinstalls are instant if the store entry exists.
- **APFS clonefile** (macOS) / **reflink** (Linux): materializing from store uses copy-on-write (zero disk overhead).
- **Parallel downloads**: deduplicates in-flight requests, races across CDN connections.
- **Streaming execution**: downloads, extractions, and linking happen concurrently.

## Linux Support

zerobrew also works on Linux (x86_64 and aarch64). It downloads Homebrew's Linux bottles and patches ELF binaries for your system.

### Requirements

- **patchelf**: Required to patch ELF binary rpaths and interpreters
  ```bash
  # Debian/Ubuntu
  sudo apt install patchelf
  
  # Fedora/RHEL
  sudo dnf install patchelf
  
  # Arch
  sudo pacman -S patchelf
  ```

### Filesystem Notes

- **btrfs/xfs**: Full reflink (copy-on-write) support — materialization is instant
- **ext4/others**: Falls back to regular copy — still fast, but uses disk space

### Linux Caveats

- Homebrew's Linux bottles are built for specific glibc versions. Very old distros may have compatibility issues.
- Some packages may need additional system libraries not bundled in bottles.

## Notes on LLMs

I spent a lot of time thinking through this architecture, testing, and debugging. I also used Claude Opus 4.5 to write much of the code here. I am a big believer in language models for coding, especialy when they are given a precise spec and work with human input! See some of the discussion about this [on Reddit](https://www.reddit.com/r/rust/comments/1qn2aev/zerobrew_is_a_rustbased_520x_faster_dropin/) that convinced me it was worth adding to the README. A lot of people I respect, [including the developers of uv](https://x.com/charliermarsh/status/2007117912801427905) are doing similar sorts of development, I don't think this is a particularly crazy practice in 2026. 


## Storage layout

```
/opt/zerobrew/
├── store/          # content-addressable (sha256 keys)
├── prefix/
│   ├── Cellar/     # materialized packages
│   ├── bin/        # symlinked executables
│   └── opt/        # symlinked package directories
├── cache/          # downloaded bottle blobs
├── db/             # sqlite database
└── locks/          # per-entry file locks
```

## Build from source 

```bash
cargo build --release
cargo install --path zb_cli
```

## Benchmarking

```bash
./benchmark.sh                                # 100-package benchmark
./benchmark.sh --format html -o results.html  # html report
./benchmark.sh --format json -o results.json  # json output
./benchmark.sh -c 20 --quick                  # quick test (22 packages)
./benchmark.sh -h                             # show help
```

## Migrating from Homebrew

See [MIGRATION.md](MIGRATION.md) for a complete guide to migrating from Homebrew to Zerobrew.

Quick start:
```bash
# Export your Homebrew packages
brew bundle dump --file=~/.Brewfile

# Install with Zerobrew
zb bundle --file ~/.Brewfile

# Update shell config
eval "$(zb shellenv)"
```

## Status

Zerobrew is feature-complete for common workflows. It supports bottle installs, upgrades, taps, services (systemd/launchd), and source builds. See the [ROADMAP.md](ROADMAP.md) for details.

Some formulas may need more work - please submit issues / PRs! 

