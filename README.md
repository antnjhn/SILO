
<p align="center">
  <img src="assets/icon.png" width="180" alt="SILO logo" />
</p>

<h1 align="center">SILO</h1>

<p align="center">
  <strong>Your game library. One place. Your control.</strong>
  <br />
  A controller-first game library and launcher for Windows and Linux.
</p>

<p align="center">
  <a href="https://github.com/antnjhn/SILO/releases/latest">
    <img src="https://img.shields.io/github/v/release/antnjhn/SILO?style=for-the-badge&color=8b5cf6" alt="Latest release" />
  </a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux-64748b?style=for-the-badge" alt="Platform" />
  <img src="https://img.shields.io/badge/built%20with-Tauri%202-ffc131?style=for-the-badge" alt="Tauri 2" />
  <img src="https://img.shields.io/badge/license-MIT-64748b?style=for-the-badge" alt="MIT license" />
</p>

<p align="center">
  <a href="https://github.com/antnjhn/SILO/releases">Download</a>
  ·
  <a href="#features">Features</a>
  ·
  <a href="#getting-started">Getting Started</a>
  ·
  <a href="#building-from-source">Build from Source</a>
</p>

---

<p align="center">
  <img src="assets/ss-1.png" width="49%" alt="SILO game library" />
  <img src="assets/ss-2.png" width="49%" alt="SILO game details" />
</p>

## What is SILO?

SILO is a desktop game launcher designed to make your PC feel more like a console.

Bring your games into one library, browse them with a controller, launch them from a fullscreen interface, and keep track of your playtime. Give every game its own artwork and background, while SaveGuard takes care of backing up and restoring save files.

**Your game library should be yours.** Your library, metadata, artwork and backups are stored locally on your machine.

## Features

### A Library Built for the Couch

- **Controller-first navigation** — Browse your library, navigate menus, open game details and launch games using a gamepad.
- **Console-style interface** — A fullscreen, frameless dashboard with a vertical game rail and animated game backgrounds.
- **Game details** — Access launch, edit, favorite, backup and restore actions from a game's detail view.
- **Playtime statistics** — Track sessions, total hours and last-played dates. View activity by week or month.
- **Organization** — Use favorites, tags, search and categories to keep your library manageable.

### One Library for Your Games

- **Steam** — Import installed games from your Steam libraries.
- **Epic Games Store and GOG** — Import installed games through Heroic Games Launcher.
- **Custom games** — Add games using their executable or installation path.
- **Folder discovery** — Scan selected directories for games and review discoveries before adding them.

Windows uses registry and filesystem discovery. Linux supports native and Flatpak Steam libraries, Heroic, and manually added games.

### Artwork That Makes Every Game Feel at Home

SILO integrates with [SteamGridDB](https://www.steamgriddb.com/) to find game logos, hero images, backgrounds and grid artwork.

Artwork can be selected manually, replaced from local files, or filled in through automatic metadata matching. A free SteamGridDB API key is required.

### SaveGuard — Your Saves Matter

SaveGuard is SILO's save backup and restore system.

- Detects potential save locations when games are added and launched.
- Creates automatic backups when a game exits.
- Keeps a rolling history of snapshots.
- Supports manual, named backups.
- Restores selected snapshots through a transactional restore process.
- Includes a Backup Manager for reviewing and managing stored snapshots.
- Supports relinking save directories when detection needs correction.

### Built Around Your Machine

- Local library and settings storage.
- Import and export library data, metadata, artwork and save backups.
- Configurable window-close and system-tray behaviour.
- Game uninstaller detection and execution.
- Fullscreen interface designed to stay out of the way.

---

## Getting Started

### Download

Visit the [latest SILO release](https://github.com/antnjhn/SILO/releases/latest) and download the package for your operating system.

| Platform | Package |
|---|---|
| Windows | `SILO-Setup-<version>.exe` |
| Windows | `SILO_<version>_x64-setup.exe` |
| Linux | `SILO_<version>_amd64.deb` |
| Linux | `SILO_<version>_amd64.AppImage` |

The custom SILO installer is the recommended Windows option. The standard Tauri NSIS installer is also available.

**Windows note:** Windows SmartScreen may display a warning because the application binary is unsigned. Review the downloaded release before choosing whether to run it.

**Linux note:** The AppImage requires FUSE support, including `libfuse2` on distributions that need it.

### First Launch

SILO includes a first-run setup to help configure your library.

1. **Game folders** — Select the directories SILO should scan.
2. **Artwork** — Add your SteamGridDB API key to enable automatic artwork.
3. **Game stores** — Choose which installed Steam, Epic and GOG libraries to import.
4. **Scan** — Review the discovered games and finish setup.

You can skip steps and configure them later through Settings.

## Controls

SILO supports both gamepad and keyboard navigation.

### Gamepad

| Input | Action |
|---|---|
| D-pad Up / Down | Navigate the game rail |
| D-pad Right | Enter the focused game |
| D-pad Left | Open the category wheel |
| A | Open details / activate action |
| B | Go back; hold to launch from the rail |
| X | Remove the focused game |
| Y | Add a game; hold to edit the focused game |
| LB | Open category wheel |
| START | Toggle the top bar |

### Keyboard

| Key | Action |
|---|---|
| Arrow keys | Navigate the library |
| Right | Enter the focused game |
| Left | Navigate back or open category wheel |
| Enter | Open details / activate action |
| C | Open category wheel |
| `/` or `Ctrl + K` | Search |
| Escape | Close the current view or overlay |

## Data and Privacy

SILO stores its library data, settings, artwork and save snapshots locally.

It does not require an account or a cloud library service. SteamGridDB is contacted when you use its artwork integration; the service requires your API key.

Default application data locations:

**Windows**
```text
%APPDATA%\com.silo.launcher\
%LOCALAPPDATA%\com.silo.launcher\logs\
```

**Linux**
```text
~/.local/share/com.silo.launcher/
~/.local/state/com.silo.launcher/logs/
```

Use **Open Logs Folder** in Settings to access application logs.

## Linux Support

SILO supports several Linux game environments:

| Environment | Support |
|---|---|
| Steam | Native games and Proton titles |
| Heroic | Epic Games Store and GOG |
| Flatpak | Steam and Heroic installations |
| Lutris | Games added by installation path |
| Wine / Proton | Prefix-based game configurations |
| Native applications | AppImages, shell scripts and ELF binaries |

Some Windows-only games may require manual Wine or Proton configuration. SteamGridDB artwork requires an API key on Linux as well.

## Building from Source

### Requirements

- Node.js LTS and npm
- Rust stable (1.77.2 or newer)
- Windows 10/11: WebView2 and MSVC build tools
- Linux: Tauri system dependencies, including WebKitGTK

Install the Linux dependencies:

```bash
# Ubuntu / Debian / Linux Mint
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev pkg-config libglib2.0-dev libgtk-3-dev libxdo-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev

# Arch Linux
sudo pacman -S webkit2gtk-4.1 libayatana-appindicator librsvg patchelf openssl pkg-config glib2 gtk3 libxdo soup3 javascriptcoregtk-4.1

# Fedora
sudo dnf install webkit2gtk4.1-devel libayatana-appindicator-gtk3-devel librsvg2-devel patchelf openssl-devel pkgconfig glib2-devel gtk3-devel libxdo-devel soup3-devel javascriptcoregtk4.1-devel
```

### Clone and Run

```bash
git clone https://github.com/antnjhn/SILO.git
cd SILO
npm install

# Development
npm run dev

# Production build
npm run build
```

Build artifacts are generated under `src-tauri/target/release/` and `src-tauri/target/release/bundle/`.

The renderer is plain ES modules served directly from `src/renderer`; no separate frontend bundling step is required.

### Custom Windows Installer

The custom NSIS installer is maintained separately from the standard Tauri bundle. Build the application first, then run:

```powershell
cd installer
.\build.ps1
```

The installer is generated under `installer/dist/`.

### Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml
node --test src/renderer/js/gamepad.test.mjs
```

## Technology

| Component | Technology |
|---|---|
| Desktop framework | [Tauri 2](https://v2.tauri.app/) |
| Backend | Rust |
| Frontend | HTML, CSS and JavaScript ES modules |
| Artwork | [SteamGridDB](https://www.steamgriddb.com/) |
| Typography | Inter |

## Project Structure

```text
SILO/
├── src/
│   └── renderer/       # Frontend interface and JavaScript modules
├── src-tauri/
│   └── src/            # Rust backend and desktop commands
├── installer/          # Custom Windows installer
├── assets/             # Repository screenshots and assets
├── docs/               # Project documentation
├── package.json
└── README.md
```

## License

SILO's source code and project documentation are licensed under the [MIT License](LICENSE).

Copyright (c) 2026 Anton John.

**Branding:** The SILO name, logo and associated branding are reserved and are not covered by the MIT License. See [TRADEMARKS.md](TRADEMARKS.md) for details.

Third-party dependencies and assets remain subject to their respective licenses.
