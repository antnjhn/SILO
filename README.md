<p align="center">
  <img src="assets/icon.png" width="360" alt="SILO" />
</p>

<h1 align="center">SILO</h1>

<p align="center">
  A console-style game launcher for Windows — controller-first, distraction-free, and it never loses your saves.
</p>

<p align="center">
  <a href="https://github.com/antnjhn/SILO/releases/latest">
    <img src="https://img.shields.io/github/v/release/antnjhn/SILO?style=flat-square&color=7c4dff" alt="Latest Release" />
  </a>
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="License" />
  <img src="https://img.shields.io/badge/platform-Windows-0078d4?style=flat-square" alt="Platform" />
  <img src="https://img.shields.io/badge/built_with-Tauri_2-ffc131?style=flat-square" alt="Built with Tauri 2" />
</p>

<p align="center">
  <img src="assets/ss-1.png" width="49%" alt="SILO library" />
  <img src="assets/ss-2.png" width="49%" alt="SILO game details" />
</p>

---

SILO turns your PC into a console dashboard. Kick back with a gamepad, browse a library that looks the way you want it to, launch with one press — and never worry about losing a save again. Everything runs locally. Nothing leaves your machine.

---

## Features

**Controller-first** — Full Xbox gamepad support. Browse, launch, and manage your whole library without touching a keyboard.

**Import from Steam, Epic & GOG** — Pull in your installed titles with a few clicks, ready to launch.

**Beautiful, and yours** — Per-game wallpapers that crossfade as you navigate, plus logos and custom typography. Grab art from **Steam and SteamGridDB** (logos, heroes, and grids) right from the picker.

**SaveGuard** — Save locations are detected automatically. SILO backs them up every time a game exits, keeps a rolling history, and restores transactionally — a restore can never leave you worse off.

**Playtime tracking** — Sessions, total hours, and last played, tracked locally.

**Stay organized** — Favorites, tags, live search, and sorting make a big library feel small.

**Back it all up** — Export your whole library (metadata, images, and save backups) to a single `.zip` and import it anywhere.

**Uninstaller integration** — SILO detects `unins000.exe` and can uninstall or delete a game folder in one step.

**Frameless fullscreen UI** — No window chrome, no taskbar bleed. It fills the screen and gets out of the way.

---

## Install

Download **`SILO_0.2.1_x64-setup.exe`** (the release's only asset) from the [Releases](https://github.com/antnjhn/SILO/releases/latest) page and run it.

| File | Type |
|------|------|
| `SILO_0.2.1_x64-setup.exe` | NSIS installer |

> Windows may show a SmartScreen warning because the binary is unsigned — click **More info** → **Run anyway**. That's expected for indie software without a code-signing certificate.

---

## Build from source

**Prerequisites:** Node.js (LTS), Rust, Windows 10 or 11.

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
| `RESTORE` | Lists available backups labeled `AUTO` or `MANUAL` — pick one to restore or delete |

Automatic backups run every time a game exits.

---

## Data storage

Everything stays local. Nothing leaves your machine.

```
%APPDATA%\com.silo.launcher\            # app data
├── games.json       # library metadata
├── settings.json    # preferences (e.g. SteamGridDB API key)
├── wallpapers/      # background images & logos
└── backups/         # compressed save snapshots

%LOCALAPPDATA%\com.silo.launcher\logs\  # diagnostic logs
```

---

## Stack

| Layer | Technology |
|-------|------------|
| Shell | [Tauri 2](https://v2.tauri.app/) |
| Backend | Rust — process management, filesystem ops, save detection |
| Frontend | Vanilla HTML / CSS / JS |

---

## License

[![MIT License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

MIT. Do whatever you want with it.
