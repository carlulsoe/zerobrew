# Migrating from Homebrew to Zerobrew

This guide covers migrating from Homebrew to Zerobrew. Zerobrew is designed as a drop-in replacement for most Homebrew workflows.

## Quick Start

```bash
# 1. Install zerobrew
curl -sSL https://raw.githubusercontent.com/lucasgelfond/zerobrew/main/install.sh | bash

# 2. Export your current Homebrew packages
brew bundle dump --file=~/.Brewfile

# 3. Install packages with zerobrew
zb bundle --file ~/.Brewfile

# 4. Update your shell configuration
```

## Shell Configuration

Replace your Homebrew shell initialization with Zerobrew:

**Before (Homebrew):**
```bash
eval "$(/opt/homebrew/bin/brew shellenv)"
```

**After (Zerobrew):**
```bash
eval "$(zb shellenv)"
```

Add this to your `~/.bashrc`, `~/.zshrc`, or equivalent.

## Command Mapping

Most commands work identically between Homebrew and Zerobrew:

| Homebrew | Zerobrew | Notes |
|----------|----------|-------|
| `brew install pkg` | `zb install pkg` | Same syntax |
| `brew uninstall pkg` | `zb uninstall pkg` | Same syntax |
| `brew upgrade` | `zb upgrade` | Same syntax |
| `brew upgrade pkg` | `zb upgrade pkg` | Same syntax |
| `brew search query` | `zb search query` | Same syntax |
| `brew info pkg` | `zb info pkg` | Same syntax |
| `brew list` | `zb list` | Same syntax |
| `brew outdated` | `zb outdated` | Same syntax |
| `brew pin pkg` | `zb pin pkg` | Same syntax |
| `brew unpin pkg` | `zb unpin pkg` | Same syntax |
| `brew deps pkg` | `zb deps pkg` | Same syntax |
| `brew uses pkg` | `zb uses pkg` | Same syntax |
| `brew leaves` | `zb leaves` | Same syntax |
| `brew link pkg` | `zb link pkg` | Same syntax |
| `brew unlink pkg` | `zb unlink pkg` | Same syntax |
| `brew cleanup` | `zb cleanup` | Same syntax |
| `brew autoremove` | `zb autoremove` | Same syntax |
| `brew doctor` | `zb doctor` | Same syntax |
| `brew tap user/repo` | `zb tap user/repo` | Same syntax |
| `brew untap user/repo` | `zb untap user/repo` | Same syntax |
| `brew services` | `zb services` | Same syntax |
| `brew bundle` | `zb bundle` | Same syntax |

## Environment Variables

Zerobrew sets Homebrew-compatible environment variables for maximum compatibility:

```bash
HOMEBREW_PREFIX="/opt/zerobrew/prefix"
HOMEBREW_CELLAR="/opt/zerobrew/prefix/Cellar"
```

This means most scripts expecting Homebrew paths will work unchanged.

## Storage Locations

| Purpose | Homebrew (macOS) | Zerobrew |
|---------|------------------|----------|
| Prefix | `/opt/homebrew` | `/opt/zerobrew/prefix` |
| Cellar | `/opt/homebrew/Cellar` | `/opt/zerobrew/prefix/Cellar` |
| Cache | `~/Library/Caches/Homebrew` | `~/.cache/zerobrew` |
| Database | N/A (filesystem) | `/opt/zerobrew/db/zerobrew.db` |
| Store | N/A | `/opt/zerobrew/store` |

## Brewfile Migration

Zerobrew supports Brewfile syntax for declarative package management:

```bash
# Export from Homebrew
brew bundle dump --file=Brewfile

# Install with Zerobrew
zb bundle install --file Brewfile
```

**Supported Brewfile entries:**
- `tap "user/repo"` - Third-party repositories
- `brew "formula"` - Package installation
- `brew "formula", args: ["--HEAD"]` - Installation with options

**Not currently supported:**
- `cask "app"` - macOS GUI applications
- `mas "app"` - Mac App Store apps
- `vscode "extension"` - VS Code extensions

## Third-Party Taps

Zerobrew supports Homebrew taps:

```bash
# Add a tap
zb tap user/repo

# Install from tap
zb install user/repo/formula

# List taps
zb tap

# Remove a tap
zb untap user/repo
```

Zerobrew parses Ruby formula files directly, so most tap formulas work out of the box.

## Services

Service management works similarly:

```bash
# List services
zb services list

# Start a service
zb services start redis

# Stop a service
zb services stop redis

# Enable auto-start
zb services enable redis

# View logs
zb services log redis
```

On Linux, Zerobrew uses systemd user services. On macOS, it uses launchd.

## Building from Source

For packages without bottles or when you need custom builds:

```bash
# Build from source
zb install --build-from-source pkg

# Build HEAD version
zb install --HEAD pkg
```

## Differences from Homebrew

### What's Faster

- **Cold installs**: ~2x faster on average due to parallel downloads and streaming extraction
- **Warm installs**: ~8x faster due to content-addressable store (instant reinstalls)
- **Linking**: Copy-on-write clonefile on macOS/APFS, reflinks on Linux/btrfs

### What's Different

1. **Storage model**: Packages stored by SHA256 hash, deduplicated across versions
2. **Database**: SQLite for package tracking (faster queries)
3. **No Cask support**: GUI apps not supported (use `mas` or direct downloads)
4. **No analytics**: Privacy-focused, no usage data collected

### What's Missing

- Cask (GUI apps) - Use native installers or `mas` for Mac App Store
- Formula creation tools - Use Homebrew's tooling
- `brew edit` - Edit formulas manually if needed

## Running Both

You can run Homebrew and Zerobrew side-by-side during migration:

1. Keep your Homebrew installation
2. Install Zerobrew
3. Update PATH to prefer Zerobrew: `export PATH="/opt/zerobrew/prefix/bin:$PATH"`
4. Gradually migrate packages
5. Uninstall Homebrew when ready: `/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/uninstall.sh)"`

## Troubleshooting

### Check System Health

```bash
zb doctor
```

This diagnoses common issues like missing dependencies, broken symlinks, and permission problems.

### Package Not Found

If a package exists in Homebrew but not Zerobrew:

1. Check if it's in a tap: `zb tap homebrew/core` (usually not needed)
2. Try searching: `zb search <name>`
3. Check for versioned variants: `zb search <name>@`

### Linux-Specific Issues

On Linux, ensure `patchelf` is installed:

```bash
# Debian/Ubuntu
sudo apt install patchelf

# Fedora/RHEL
sudo dnf install patchelf

# Arch
sudo pacman -S patchelf
```

### Permission Issues

Zerobrew installs to `/opt/zerobrew` by default. The install script sets up permissions, but if you have issues:

```bash
zb init
```

## Complete Migration Checklist

- [ ] Install Zerobrew
- [ ] Export Homebrew packages: `brew bundle dump --file=~/.Brewfile`
- [ ] Install packages: `zb bundle --file ~/.Brewfile`
- [ ] Update shell config to use `zb shellenv`
- [ ] Verify packages work: `zb doctor`
- [ ] Migrate services: `zb services list`
- [ ] Test your development workflow
- [ ] (Optional) Uninstall Homebrew

## Getting Help

- Issues: https://github.com/lucasgelfond/zerobrew/issues
- Run `zb --help` for command reference
- Run `zb <command> --help` for command-specific help
