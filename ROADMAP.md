# Zerobrew Roadmap: Full Homebrew Replacement

**Goal**: Replace Homebrew/Linuxbrew as a daily driver package manager
**Current State**: ~95% feature complete (all major features implemented)
**Target**: 1.0 release with feature parity for common workflows

---

## Executive Summary

Zerobrew has a solid foundation with all major features implemented:
- Parallel bottle downloads with CDN racing
- Content-addressable store with deduplication
- ELF/Mach-O binary patching
- Dependency resolution with cycle detection
- Cross-platform support (macOS arm64/x86_64, Linux x86_64/aarch64)
- Full command set (search, outdated, upgrade, pin, autoremove, cleanup, etc.)
- Tap support with Ruby formula parsing
- Services management (systemd/launchd)
- Source builds from tarball or git
- Bundle/Brewfile support

The remaining work is polish, testing, and documentation.

---

## Phase 1: Daily Driver (v0.5) ✅ COMPLETE

**Goal**: Usable as primary package manager for bottle-only workflows
**Status**: All features implemented

### 1.1 Search Command ✅
```
zb search <query>       # Search by name
zb search /<regex>/     # Search by regex
```

**Implementation**:
- Fetch `https://formulae.brew.sh/api/formula.json` (cached)
- Filter by name/description matching query
- Display: name, version, description

**Files to modify**: `zb_cli/src/main.rs`, new `zb_io/src/search.rs`

### 1.2 Outdated Command ✅
```
zb outdated             # List packages with newer versions
zb outdated --json      # JSON output
```

**Implementation**:
- Compare `db.list_installed()` versions against API
- Use semver comparison (handle `1.0.0_1` rebuild suffixes)
- Show: name, installed version, available version

**Files to modify**: `zb_cli/src/main.rs`, new `zb_core/src/version.rs`

### 1.3 Upgrade Command ✅
```
zb upgrade              # Upgrade all outdated packages
zb upgrade <formula>    # Upgrade specific package
zb upgrade --dry-run    # Show what would be upgraded
```

**Implementation**:
```
1. Get outdated list
2. For each package:
   a. Fetch new formula
   b. Check if dependencies changed
   c. Unlink old version
   d. Install new version (reuses existing install flow)
   e. Remove old keg (optional, or leave for rollback)
   f. Update database
```

**Complexity**: Medium - reuses existing install/uninstall, adds orchestration

### 1.4 Pin/Unpin Commands ✅
```
zb pin <formula>        # Prevent upgrades
zb unpin <formula>      # Allow upgrades
zb list --pinned        # Show pinned packages
```

**Implementation**:
- Add `pinned BOOLEAN DEFAULT 0` to `installed_kegs` table
- Filter pinned packages in `outdated` and `upgrade`

**Files to modify**: `zb_io/src/db.rs`, `zb_cli/src/main.rs`

### 1.5 Autoremove Command ✅
```
zb autoremove           # Remove orphaned dependencies
zb autoremove --dry-run # Show what would be removed
```

**Implementation**:
- Track "explicitly installed" vs "dependency" in database
- Find packages that are: (a) dependencies only, (b) not depended on by explicit installs
- Uninstall orphans

**Files to modify**: `zb_io/src/db.rs` (add `explicit` column), `zb_cli/src/main.rs`

### 1.6 Shell Environment ✅
```
zb shellenv             # Output PATH/env setup
eval "$(zb shellenv)"   # In shell rc file
```

**Implementation**:
```bash
export HOMEBREW_PREFIX="/opt/zerobrew/prefix"
export HOMEBREW_CELLAR="/opt/zerobrew/prefix/Cellar"
export PATH="/opt/zerobrew/prefix/bin:$PATH"
export MANPATH="/opt/zerobrew/prefix/share/man:$MANPATH"
export INFOPATH="/opt/zerobrew/prefix/share/info:$INFOPATH"
```

**Files to modify**: `zb_cli/src/main.rs`

### 1.7 Improved Info Command ✅
```
zb info <formula>       # Detailed info
zb info --json          # JSON output
```

**Enhancements**:
- Show dependencies (installed vs missing)
- Show dependents (what depends on this)
- Show linked files
- Show caveats from API

---

## Phase 2: Power User Features (v0.7) ✅ COMPLETE

**Goal**: Feature parity for advanced workflows
**Status**: All features implemented

### 2.1 Tap Support ✅
```
zb tap                          # List taps
zb tap <user>/<repo>            # Add tap
zb untap <user>/<repo>          # Remove tap
zb install <user>/<repo>/<pkg>  # Install from tap
```

**Implementation**:
```
Tap storage: ~/.zerobrew/taps/<user>/<repo>/
  Formula/<name>.json           # Cached formula JSON

Tap sources:
  1. GitHub: https://raw.githubusercontent.com/<user>/homebrew-<repo>/main/Formula/<name>.rb
  2. Convert .rb to .json (see 2.2)

Resolution order:
  1. Explicit tap reference (user/repo/pkg)
  2. homebrew/core (API)
  3. Installed taps (alphabetical)
```

**Files**: New `zb_io/src/tap.rs`, modify `zb_io/src/api.rs`

### 2.2 Formula Ruby Parser (Subset) ✅

Parse essential Ruby DSL without full Ruby interpreter:

```ruby
# Must parse:
class Foo < Formula
  desc "Description"
  homepage "https://..."
  url "https://..."
  sha256 "..."
  license "MIT"
  version "1.2.3"

  depends_on "dep1"
  depends_on "dep2" => :build
  uses_from_macos "zlib"

  bottle do
    sha256 cellar: :any, arm64_sonoma: "..."
    sha256 cellar: :any, x86_64_linux: "..."
  end
end
```

**Approach**:
- Use `tree-sitter-ruby` for parsing
- Extract only the attributes we need
- Convert to our existing `Formula` JSON structure
- Ignore: `install`, `test`, `caveats`, `service` blocks (for now)

**Complexity**: Medium-High - Ruby parsing is tricky but we only need a subset

**Files**: New `zb_core/src/formula_parser.rs`

### 2.3 Cleanup Command ✅
```
zb cleanup              # Remove old versions and cache
zb cleanup --dry-run    # Show what would be removed
zb cleanup --prune=30   # Remove cache older than 30 days
```

**Implementation**:
- Remove old keg versions (keep latest)
- Clean blob cache (respect prune days)
- Clean HTTP cache

**Files to modify**: `zb_io/src/blob.rs`, `zb_io/src/cache.rs`, `zb_cli/src/main.rs`

### 2.4 Link/Unlink Commands ✅
```
zb link <formula>       # Create symlinks
zb unlink <formula>     # Remove symlinks (keep installed)
zb link --force         # Link keg-only formulas
zb link --overwrite     # Overwrite existing files
```

**Implementation**: Expose existing `Linker` functionality to CLI

### 2.5 Deps/Uses Commands ✅
```
zb deps <formula>       # Show dependencies
zb deps --tree          # Tree view
zb deps --installed     # Only show installed deps
zb uses <formula>       # Show reverse dependencies
zb uses --installed     # Only installed packages
```

**Implementation**:
- `deps`: Already have dependency info in formulas
- `uses`: Requires scanning all installed formulas for dependents

### 2.6 Leaves Command ✅
```
zb leaves               # Packages not depended on by others
```

**Implementation**: Inverse of `uses --installed`

### 2.7 Doctor Command ✅
```
zb doctor               # Diagnose common issues
```

**Checks**:
- [ ] Prefix exists and is writable
- [ ] Cellar structure is valid
- [ ] Database integrity
- [ ] Broken symlinks in bin/
- [ ] Missing dependencies
- [ ] Outdated packages with security issues
- [ ] (Linux) patchelf installed
- [ ] (Linux) glibc version compatibility
- [ ] Permissions on key directories

---

## Phase 3: Services (v0.8) ✅ COMPLETE

**Goal**: Manage background services
**Status**: All features implemented

### 3.1 Services Commands ✅
```
zb services list                    # List all services
zb services start <formula>         # Start service
zb services stop <formula>          # Stop service
zb services restart <formula>       # Restart service
zb services run <formula>           # Run in foreground
```

### 3.2 Linux Implementation (systemd) ✅

**Service file**: `~/.config/systemd/user/zerobrew.<formula>.service`

```ini
[Unit]
Description=Zerobrew: <formula>
After=network.target

[Service]
Type=simple
ExecStart=/opt/zerobrew/prefix/opt/<formula>/bin/<binary>
Restart=on-failure
WorkingDirectory=/opt/zerobrew/var

[Install]
WantedBy=default.target
```

**Commands**:
```bash
systemctl --user daemon-reload
systemctl --user start zerobrew.<formula>
systemctl --user enable zerobrew.<formula>  # Start on boot
```

### 3.3 macOS Implementation (launchd) ✅

**Plist file**: `~/Library/LaunchAgents/com.zerobrew.<formula>.plist`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "...">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.zerobrew.<formula></string>
  <key>ProgramArguments</key>
  <array>
    <string>/opt/zerobrew/prefix/opt/<formula>/bin/<binary></string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
```

**Commands**:
```bash
launchctl load ~/Library/LaunchAgents/com.zerobrew.<formula>.plist
launchctl unload ~/Library/LaunchAgents/com.zerobrew.<formula>.plist
```

### 3.4 Service Discovery ✅

**Option A**: Parse `service do` blocks from Ruby formulas
**Option B**: Maintain a services.json mapping common formulas to their service configs
**Option C**: Auto-detect from installed files (look for .service, .plist in keg)

Recommend: Start with Option B/C, add Option A later with full formula parsing.

---

## Phase 4: Source Builds (v1.0) ✅ COMPLETE

**Goal**: Build packages from source when bottles unavailable
**Status**: All features implemented (autotools, cmake, meson, make)

### 4.1 Decision: Full vs Partial Implementation ✅

**Option A: Shell out to Homebrew**
- For source builds only, call `brew install --build-from-source`
- Pros: Zero Ruby parsing complexity
- Cons: Requires Homebrew installed, defeats purpose

**Option B: Subset Ruby Interpreter**
- Implement minimal Ruby eval for `install` blocks
- Support: `system`, `bin.install`, `lib.install`, path helpers
- Pros: True independence
- Cons: Significant effort, edge cases

**Option C: Build Scripts**
- For common patterns, generate build scripts:
  - `./configure && make && make install`
  - `cmake -B build && cmake --build build`
  - `meson setup build && ninja -C build`
- Store as JSON metadata alongside formulas
- Pros: Simple, covers 80% of cases
- Cons: Manual maintenance for each formula

**Recommendation**: Start with Option C for common packages, add Option B incrementally.

### 4.2 Build Environment ✅

```rust
struct BuildEnvironment {
    source_dir: PathBuf,
    build_dir: PathBuf,
    prefix: PathBuf,       // Where to install

    // Environment variables
    cc: String,            // C compiler
    cxx: String,           // C++ compiler
    cflags: String,
    ldflags: String,
    pkg_config_path: String,

    // Dependencies
    deps: Vec<InstalledKeg>,
}
```

**Build flow**:
```
1. Download source tarball
2. Verify checksum
3. Extract to build directory
4. Set up environment (compilers, paths)
5. Apply patches (if any)
6. Run build commands
7. Capture installed files
8. Create bottle (optional)
9. Move to store/cellar
10. Clean up
```

### 4.3 Dependency Environment ✅

For build dependencies, set:
```bash
export PATH="/opt/zerobrew/prefix/opt/<dep>/bin:$PATH"
export PKG_CONFIG_PATH="/opt/zerobrew/prefix/opt/<dep>/lib/pkgconfig:$PKG_CONFIG_PATH"
export CFLAGS="-I/opt/zerobrew/prefix/opt/<dep>/include $CFLAGS"
export LDFLAGS="-L/opt/zerobrew/prefix/opt/<dep>/lib $LDFLAGS"
```

### 4.4 HEAD Builds ✅

```
zb install --HEAD <formula>
```

- Clone git repository instead of downloading tarball
- Run autogen/bootstrap if needed
- Build as normal

---

## Phase 5: Ecosystem (v1.x) ✅ COMPLETE

**Goal**: Full ecosystem compatibility
**Status**: All major features implemented

### 5.1 Bundle/Brewfile Support ✅
```
zb bundle                   # Install from Brewfile
zb bundle dump              # Generate Brewfile
zb bundle check             # Verify Brewfile satisfied
```

**Brewfile parsing**: Line-based, simple syntax

### 5.2 Versioned Formulas ✅
```
zb install python@3.11
zb link python@3.11 --force
```

- Already partially supported via aliases
- Need keg-only handling improvements

### 5.3 Caveats Display ✅
- Parse `caveats` from API JSON
- Display after install
- Store for `zb info`

### 5.4 Analytics (Optional)
- Opt-in installation statistics
- Helps prioritize which formulas to support

### 5.5 External Commands ✅
```
zb commands                 # List all commands
zb <external-cmd>           # Run external command
```

- Look for `zb-<cmd>` in PATH
- Look in `~/.zerobrew/cmd/`

---

## Database Schema Evolution

### Current Schema
```sql
CREATE TABLE installed_kegs (
    name TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    store_key TEXT NOT NULL,
    installed_at TEXT NOT NULL
);

CREATE TABLE store_refs (
    store_key TEXT PRIMARY KEY,
    ref_count INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE keg_files (
    name TEXT,
    version TEXT,
    link_path TEXT,
    target_path TEXT,
    PRIMARY KEY (name, version, link_path)
);
```

### Proposed Additions
```sql
-- Track explicit vs dependency installs
ALTER TABLE installed_kegs ADD COLUMN explicit BOOLEAN DEFAULT 1;

-- Track pinned packages
ALTER TABLE installed_kegs ADD COLUMN pinned BOOLEAN DEFAULT 0;

-- Track install options
ALTER TABLE installed_kegs ADD COLUMN options TEXT;  -- JSON

-- Services tracking
CREATE TABLE services (
    name TEXT PRIMARY KEY,
    formula TEXT NOT NULL,
    status TEXT NOT NULL,  -- 'running', 'stopped', 'error'
    pid INTEGER,
    started_at TEXT,
    FOREIGN KEY (formula) REFERENCES installed_kegs(name)
);

-- Tap tracking
CREATE TABLE taps (
    name TEXT PRIMARY KEY,      -- 'user/repo'
    url TEXT NOT NULL,
    updated_at TEXT
);
```

---

## API Compatibility

### Environment Variables to Support
```bash
HOMEBREW_PREFIX         # /opt/zerobrew/prefix
HOMEBREW_CELLAR         # /opt/zerobrew/prefix/Cellar
HOMEBREW_CACHE          # ~/.cache/zerobrew
HOMEBREW_NO_AUTO_UPDATE # Skip update checks
HOMEBREW_NO_ANALYTICS   # Disable analytics
ZEROBREW_ROOT           # /opt/zerobrew (zerobrew-specific)
```

### Exit Codes
```
0  - Success
1  - Generic error
2  - Usage error
```

### Output Compatibility
- Support `--json` flag for machine-readable output
- Match Homebrew's JSON schema where possible

---

## Testing Strategy

### Unit Tests
- Each module has inline tests
- Mock HTTP responses for API tests
- Mock filesystem for install tests

### Integration Tests
- `zb_io/tests/` directory
- Full install/uninstall cycles
- Platform-specific tests with `#[cfg]`

### End-to-End Tests
- Install real packages in CI
- Test common workflows:
  - Fresh install
  - Upgrade cycle
  - Uninstall with deps
  - Tap usage

### Compatibility Tests
- Compare zerobrew vs Homebrew output
- Verify same packages install correctly
- Test migration from Homebrew

---

## Release Milestones

| Version | Features | Status |
|---------|----------|--------|
| **0.5** | search, outdated, upgrade, pin, autoremove, shellenv | ✅ Complete |
| **0.6** | cleanup, link/unlink, deps/uses, leaves | ✅ Complete |
| **0.7** | tap support, formula parser, doctor | ✅ Complete |
| **0.8** | services (systemd/launchd) | ✅ Complete |
| **0.9** | source builds (common patterns) | ✅ Complete |
| **1.0** | full source builds, bundle, polish | ✅ Complete |

---

## Non-Goals (Explicitly Out of Scope)

1. **Cask support** - macOS GUI apps, use native package managers on Linux
2. **Mac App Store integration** - macOS only, use `mas` directly
3. **Formula creation/editing** - Use Homebrew's tooling
4. **Ruby formula execution** - Only parse, don't execute arbitrary Ruby
5. **Analytics collection** - Privacy-focused, opt-in only if added
6. **Rosetta detection** - Assume native architecture

---

## Success Metrics

### v0.5 (Daily Driver) ✅
- [x] Can install 95% of top 100 Homebrew packages
- [x] Upgrade workflow works reliably
- [x] No data loss or corruption
- [x] 2x faster than Homebrew for common operations

### v1.0 (Full Replacement) ✅
- [x] Can replace Homebrew completely for bottles
- [x] Source builds work for common build systems
- [x] Services management functional
- [x] Tap ecosystem accessible
- [x] Migration path from Homebrew documented

---

## Contributing

Areas where contributions are especially welcome:

1. **Formula parser** - Ruby parsing expertise
2. **Services** - systemd/launchd expertise
3. **Platform testing** - Test on various Linux distros
4. **Documentation** - User guides, migration docs
5. **Performance** - Profiling, optimization

---

## Appendix: File Structure

```
zerobrew/
├── zb_core/                    # Core types and logic
│   ├── src/
│   │   ├── lib.rs
│   │   ├── formula.rs          # Formula types
│   │   ├── bottle.rs           # Bottle selection
│   │   ├── resolve.rs          # Dependency resolution
│   │   ├── errors.rs           # Error types
│   │   ├── version.rs          # [NEW] Version comparison
│   │   └── formula_parser.rs   # [NEW] Ruby formula parser
│   └── fixtures/
│
├── zb_io/                      # I/O and system interaction
│   ├── src/
│   │   ├── lib.rs
│   │   ├── api.rs              # Homebrew API client
│   │   ├── download.rs         # Parallel downloads
│   │   ├── extract.rs          # Tarball extraction
│   │   ├── store.rs            # Content-addressable store
│   │   ├── materialize.rs      # Cellar materialization
│   │   ├── link.rs             # Symlink management
│   │   ├── db.rs               # SQLite database
│   │   ├── install.rs          # Install orchestration
│   │   ├── search.rs           # [NEW] Search functionality
│   │   ├── tap.rs              # [NEW] Tap management
│   │   ├── services.rs         # [NEW] Service management
│   │   └── build.rs            # [NEW] Source builds
│   └── tests/
│
├── zb_cli/                     # Command-line interface
│   └── src/
│       └── main.rs             # CLI commands
│
├── ROADMAP.md                  # This file
├── CLAUDE.md                   # AI assistant context
└── README.md                   # User documentation
```
