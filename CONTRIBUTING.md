# Contributing to Network Manager

Thank you for your interest in contributing to Network Manager!

The app is made for Fedora with the GNOME desktop (tested on Fedora 44), so
Fedora + GNOME is the reference platform for all changes.

## Development Setup

### Prerequisites

- Fedora's current Rust toolchain and a C/C++ compiler
- WebKitGTK 4.1, GTK 3, GLib and OpenSSL development libraries
- Python 3 and gettext for the embedded UI catalogs

**Fedora:**
```bash
sudo dnf install cargo rust gcc-c++ python3 gettext pkgconf-pkg-config gtk3-devel webkit2gtk4.1-devel glib2-devel gettext-devel openssl-devel
```

### Development Tools

Install required Rust tools:
```bash
rustup component add rustfmt clippy
```

### Building

```bash
cargo build
```

### Running

```bash
cargo run
```

## Code Quality

### Before Submitting

Run these checks before submitting a PR:

```bash
# Format code
cargo fmt

# Run linter (with warnings as errors)
cargo clippy -- -D warnings

# Run tests
cargo test

# Check for security advisories (optional)
cargo audit
```

The project has no hosted CI, so please run these locally.

## Code Style

### Naming Conventions

- **Modules**: `snake_case` (e.g., `dashboard_page.rs`)
- **Types**: `PascalCase` (e.g., `Profile`, `NetworkAction`)
- **Functions**: `snake_case` verbs (e.g., `get_profile()`, `apply_action()`)
- **Constants**: `SCREAMING_SNAKE_CASE` (e.g., `APP_ID`, `CONFIG_DIR_NAME`)

### Architecture

The Cargo workspace builds `src-tauri/` (the installed `network-manager`
executable) and `core/` (the shared Rust services). The webview loads the
bundled pages in `desktop/`. The former GTK4 entry point and widgets remain
in the source tree for reference and are not part of the build.

```
src/
├── core.rs              # Shared library entry point
├── storage.rs           # Local data persistence
├── tray.rs              # StatusNotifierItem system tray
├── network_utils.rs     # Linux sysfs network detection
├── autostart.rs         # XDG autostart management
├── models/              # Domain models
│   ├── actions/         # Network, system, automation actions
│   ├── profile.rs       # Profile data model
│   ├── config.rs        # App configuration
│   ├── error.rs         # Error types
│   ├── rules.rs         # Auto-switch rules engine
│   └── validation.rs    # Input validation
└── ui/                  # GTK widgets and pages
    ├── main_window.rs   # Main application window
    ├── pages/           # Application pages
    └── widgets/         # Reusable UI components
```

### Key Rules

1. **No panics in production code**: Use `Result<T, E>` instead of `unwrap()`/`expect()`
2. **Handle lock poisoning**: Use `.unwrap_or_else(|e| e.into_inner())` for RwLock
3. **Graceful degradation**: Handle missing displays, D-Bus failures, etc.
4. **Async for I/O**: Use async/await for D-Bus operations

## Duplication Policy

Before adding new code, check for existing similar implementations:

1. Search for similar function names
2. Check if a trait could be extracted
3. Parameterize instead of copy-paste

### Duplication Checklist

After each change, ask:
- [ ] Does this logic exist elsewhere in the codebase?
- [ ] Could this be a shared function or trait?
- [ ] Are there similar UI patterns that could be components?

## Translations

Every user-visible string goes through `tr()` (or `ntr()` for plurals),
imported with `use crate::tr;`:

- Call it bare, `tr("Save")`, with a string literal. xgettext does not
  extract `crate::tr(...)`, variables or `format!` arguments.
- Put values in named placeholders:
  `tr("Apply '{name}'?").replace("{name}", name)`.
- Plurals: `ntr("{n} profile", "{n} profiles", n).replace("{n}", &n.to_string())`.
- Never compare or parse displayed text in code; it changes with the language.

After adding or changing strings, run `./scripts/update-translations.sh`. It
regenerates `po/cd-network-manager.pot` and merges it into every catalog listed
in `po/LINGUAS`; then translate the new entries in `po/*.po`.

`cargo build` embeds the webview catalogs in the executable. The RPM also
installs gettext catalogs for Rust services under `/usr/share/locale`.

## D-Bus Integration

This application talks to NetworkManager through its command-line tools and
D-Bus. Key considerations:

1. **Async operations**: All D-Bus calls should be async
2. **Error handling**: Handle connection failures gracefully
3. **Timeouts**: Use appropriate timeouts for operations
4. **Polkit**: Privilege escalation uses PolicyKit

## Testing

### Unit Tests

Add tests for:
- Domain logic in `models/`
- Validation functions
- Profile serialization/deserialization

### Integration Tests

For D-Bus integration, use mock services or test on real systems.

### Test Matrix

Test on these configurations before release:

| Distribution | Desktop | Display Server | Notes |
|-------------|---------|----------------|-------|
| Fedora 44   | GNOME   | Wayland        | Primary target |

## Packaging

### Building Packages

```bash
# RPM package for Fedora (output: dist/rpm/)
./scripts/package-rpm.sh

# DEB package (output: dist/deb/)
./scripts/package-deb.sh

# AppImage (output: dist/appimage/)
./scripts/package-appimage.sh
```

The RPM is built with `rpmbuild` from `packaging/rpm/network-manager.spec`
(install `rpm-build` first).

## Questions?

Open an issue for:
- Bug reports
- Feature requests
- Questions about the codebase
