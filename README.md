<p align="center">
  <img src="assets/icon.png" width="360" alt="SILO" />
</p>

<h1 align="center">SILO</h1>

<p align="center">
  A console-style game launcher for Windows and Linux. Controller-first, distraction-free, and it never loses your saves.
</p>

<p align="center">
  <a href="https://github.com/antnjhn/SILO/releases/latest">
    <img src="https://img.shields.io/github/v/release/antnjhn/SILO?style=flat-square&color=7c4dff" alt="Latest Release" />
  </a>
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License" />
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux-0078d4?style=flat-square" alt="Platform" />
  <img src="https://img.shields.io/badge/built_with-Tauri_2-ffc131?style=flat-square" alt="Built with Tauri 2" />
</p>

<p align="center">
  <img src="assets/ss-1.png" width="49%" alt="SILO library" />
  <img src="assets/ss-2.png" width="49%" alt="SILO game details" />
</p>

---

SILO turns your PC into a console dashboard. Kick back with a gamepad, browse a library that looks the way you want it to, launch with one press, and never worry about losing a save again. Everything runs locally. Nothing leaves your machine.

---

## Features

**Controller-first** - Full Xbox gamepad support. Browse, launch, and manage your whole library without touching a keyboard.

**Import from Steam, Epic & GOG** - Pull in your installed titles with a few clicks, ready to launch.
- **Windows**: Direct registry and filesystem scanning
- **Linux**: Reads Steam library folders (native and Flatpak), Heroic Games Launcher configs for Epic/GOG

**Beautiful, and yours** - Per-game wallpapers that crossfade as you navigate, plus logos and custom typography. Grab art from **Steam and SteamGridDB** (logos, heroes, and grids) right from the picker.

**SaveGuard** - Save locations are detected automatically. SILO backs them up every time a game exits, keeps a rolling history, and restores transactionally. A restore can never leave you worse off.

**Playtime tracking** - Sessions, total hours, and last played, tracked locally. Activity charts with weekly/monthly views in the Statistics hub.

**Stay organized** - Favorites, tags, live search, and sorting make a big library feel small. Category dots let you jump between All Games, Recently Played, Favorites, and your tags.

**Back it all up** - Export your whole library (metadata, images, and save backups) to a single `.zip` and import it anywhere.

**Uninstaller integration** - SILO detects uninstallers and can uninstall or delete a game folder in one step.
- **Windows**: `unins000.exe`, `uninstall.exe`
- **Linux**: Native uninstall scripts, Flatpak, Steam, Wine/Proton prefixes

**Frameless fullscreen UI** - No window chrome, no taskbar bleed. It fills the screen and gets out of the way.

---

## Install

**Windows:** Download `SILO_0.2.3_x64-setup.exe` (NSIS installer) from the [Releases](https://github.com/antnjhn/SILO/releases/latest) page and run it.

**Linux:** Download `SILO_0.2.3_amd64.deb` or `SILO_0.2.3_amd64.AppImage` from the [Releases](https://github.com/antnjhn/SILO/releases/latest) page.

| File | Type | Platform |
|------|------|----------|
| `SILO_0.2.3_x64-setup.exe` | NSIS installer | Windows |
| `SILO_0.2.3_amd64.deb` | Debian package | Ubuntu, Debian, Mint, etc. |
| `SILO_0.2.3_amd64.AppImage` | AppImage | Any Linux distro |

> Windows may show a SmartScreen warning because the binary is unsigned. Click **More info** then **Run anyway**. That's expected for indie software without a code-signing certificate.

---

## Build from source

### Windows

**Prerequisites:** Node.js (LTS), Rust, Windows 10 or 11.

```bash
git clone https://github.com/antnjhn/SILO.git
cd SILO
npm install
npm run build
```

### Linux

**Prerequisites:** Node.js (LTS), Rust, and WebKitGTK development libraries.

```bash
# Ubuntu/Debian/Mint
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev pkg-config libglib2.0-dev libgtk-3-dev libxdo-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev

# Arch Linux
sudo pacman -S webkit2gtk-4.1 libayatana-appindicator librsvg patchelf openssl pkg-config glib2 gtk3 libxdo soup3 javascriptcoregtk-4.1

# Fedora
sudo dnf install webkit2gtk4.1-devel libayatana-appindicator-gtk3-devel librsvg2-devel patchelf openssl-devel pkgconfig glib2-devel gtk3-devel libxdo-devel soup3-devel javascriptcoregtk4.1-devel
```

```bash
git clone https://github.com/antnjhn/SILO.git
cd SILO
npm install
npm run build
```

Compiled binaries output to `src-tauri/target/release/bundle/`.

---

## Save management

Save locations are detected automatically when you add a game. From the details panel:

| Action | What it does |
|--------|--------------|
| `BACKUP` | Takes a named snapshot of the current save state |
| `RESTORE` | Lists available backups labeled `AUTO` or `MANUAL`. Pick one to restore or delete |

Automatic backups run every time a game exits.

---

## Data storage

Everything stays local. Nothing leaves your machine.

**Windows:**
```
%APPDATA%\com.silo.launcher\            # app data
├── games.json       # library metadata
├── settings.json    # preferences (e.g. SteamGridDB API key)
├── wallpapers/      # background images & logos
└── backups/         # compressed save snapshots

%LOCALAPPDATA%\com.silo.launcher\logs\  # diagnostic logs
```

**Linux:**
```
~/.local/share/com.silo.launcher/       # app data (XDG_DATA_HOME)
├── games.json       # library metadata
├── settings.json    # preferences (e.g. SteamGridDB API key)
├── wallpapers/      # background images & logos
└── backups/         # compressed save snapshots

~/.local/state/com.silo.launcher/logs/  # diagnostic logs (XDG_STATE_HOME)
```

---

## Stack

| Layer | Technology |
|-------|------------|
| Shell | [Tauri 2](https://v2.tauri.app/) |
| Backend | Rust - process management, filesystem ops, save detection |
| Frontend | Vanilla HTML / CSS / JS |

---

## Linux support details

### Supported launchers and runtimes
- **Steam**: Native Linux games, Proton/Steam Play (Windows games via Proton)
- **Heroic Games Launcher**: Epic Games Store and GOG games
- **Lutris**: Games managed by Lutris (manual path selection)
- **Flatpak**: Steam, Heroic, and other Flatpak'd launchers
- **Wine/Proton**: Standalone Wine prefixes and Proton-GE
- **Native Linux**: AppImages, shell scripts, ELF binaries

### Game discovery on Linux
| Platform | Method |
|----------|--------|
| Steam | Parses `libraryfolders.vdf` and `appmanifest_*.acf` from `~/.local/share/Steam`, `~/.steam/steam`, Flatpak Steam |
| Epic/GOG (via Heroic) | Reads `~/.config/heroic/legendaryConfig/legendary/installed.json` and `~/.config/heroic/gog_store/installed.json` |
| Flatpak | `~/.var/app/com.valvesoftware.Steam/...`, `~/.var/app/com.heroicgameslauncher.hgl/...` |

### SaveGuard on Linux
SaveGuard watches these locations for save files:
- `~/.local/share/` and `~/.config/` (XDG directories)
- `~/Documents`, `~/Saved Games`
- Steam Proton `compatdata` directories
- Wine prefixes (`~/.wine/drive_c/users/...`)

### Known Linux limitations
- Xbox Mode (Windows+F11 hotkey) is Windows-only
- Some Windows-only games may require manual Wine/Proton configuration
- SteamGridDB artwork requires a free API key (same as Windows)
- AppImage requires FUSE (`libfuse2`) to run

---

## License

[![MIT License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

MIT. Do whatever you want with it.
