<p align="center">
  <img src="assets/icon.png" width="320" alt="SILO" />
</p>

<h1 align="center">SILO</h1>

<p align="center">
  A console-style game launcher for Windows and Linux. Controller first, distraction free, and it never loses your saves.
</p>

<p align="center">
  <a href="https://github.com/antnjhn/SILO/releases/latest">
    <img src="https://img.shields.io/github/v/release/antnjhn/SILO?style=flat-square&color=7c4dff" alt="Latest release" />
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

SILO turns your PC into a console dashboard. Sit back with a gamepad, browse a rail of your games with their own artwork, launch with one press, and let SaveGuard handle your save files. Everything runs locally and nothing leaves the machine.

---

## Features

**First run setup** - Four steps on one flat card: where your games live, the SteamGridDB key that makes artwork automatic, the store libraries, then the scan results. You can skip any step and finish later in Settings.

**Controller first** - Full gamepad support from boot to launch. Browse, step into a game, launch, edit, and reach Settings without a keyboard.

**Library rail** - Games sit in a vertical rail with their own logos and wallpapers crossfading behind them. Browsing shows the focused game's stats; stepping into it brings out LAUNCH, EDIT, FAVORITE, BACKUP and RESTORE.

**Import from Steam, Epic and GOG** - Read the games those stores already have installed, with artwork attached.
- **Windows:** registry and filesystem scanning
- **Linux:** Steam library folders (native and Flatpak) plus Heroic Games Launcher for Epic and GOG

**Automatic artwork** - SteamGridDB logos, heroes and grids. A sweep fills in anything missing at launch and after a game is added, with a progress toast so a slow first run is visible rather than silent. Name matching ignores accents and filler words, so `POKEMON EMERALD` still finds `Pokémon Emerald Version`. The art picker also browses SteamGridDB by hand, and logos can be replaced from disk.

**SaveGuard** - Save folders are detected automatically. SILO backs them up every time a game exits, keeps a rolling history, and restores transactionally. A restore can never leave you worse off. Manual named backups and a Backup Manager are in the details panel and Settings.

**Playtime tracking** - Sessions, total hours and last played, tracked locally. The Statistics page charts overall activity or a single game, weekly or monthly.

**Stay organized** - Favorites, tags, live search, and a category wheel for All Games, Recently Played, Favorites and your own tags.

**Folder discovery** - Point SILO at your game folders and it watches them at startup. It always asks before adding a game, and remembers the ones you turned down.

**Back it all up** - Export the whole library, metadata, images and save backups, to a single file, then import it on any machine.

**Uninstaller integration** - Run a game's own uninstaller or delete its folder in one step, with a confirmation before anything is destroyed.
- **Windows:** `unins000.exe`, `uninstall.exe`
- **Linux:** native uninstall scripts, Flatpak, Steam, Wine and Proton prefixes

**Tray and close behaviour** - Choose what closing the window does: ask, minimise to the tray, or exit completely.

**Frameless fullscreen** - No window chrome and no taskbar bleed. SILO fills the display and gets out of the way.

---

## Install

**Windows (recommended):** download `SILO-Setup-<version>.exe` from the [Releases](https://github.com/antnjhn/SILO/releases/latest) page. It is SILO's own installer: per user, no admin prompt, a guided four page setup, and an uninstaller that leaves your library in `%APPDATA%` alone.

**Windows (Tauri bundle):** `SILO_<version>_x64-setup.exe` is the stock NSIS package produced by `npm run build`.

**Linux:** download the Debian package or the AppImage.

| File | Type | Platform |
|------|------|----------|
| `SILO-Setup-<version>.exe` | SILO custom installer | Windows |
| `SILO_<version>_x64-setup.exe` | Tauri NSIS installer | Windows |
| `SILO_<version>_amd64.deb` | Debian package | Ubuntu, Debian, Mint, etc. |
| `SILO_<version>_amd64.AppImage` | AppImage | Any Linux distro |

Windows may show a SmartScreen warning because the binary is unsigned. Click **More info**, then **Run anyway**. That is expected for software without a code-signing certificate.

---

## First run

SILO shows setup once, while the library is empty:

1. **Where are your games?** Add the folders your games are installed in, usually `C:\Games`. Only listed folders are scanned.
2. **Artwork, automatically.** Paste a free SteamGridDB API key. Create one at [steamgriddb.com](https://www.steamgriddb.com/profile/preferences/api). Without a key, games are added without artwork.
3. **Bring in your libraries.** Pick Steam, Epic or GOG to import what that store already has installed. Nothing is imported unless you pick a store.
4. **Scan.** SILO looks through the folders you listed and shows what it found. Press Finish.

Every step can be skipped, and every one of them exists again in Settings.

---

## Controls

### Controller

| Input | Browsing the rail | Inside the game |
|-------|-------------------|-----------------|
| D-pad up / down | Move between games, up from the first reveals the top bar | Walk the action dock |
| D-pad right | Step into the focused game | Move right in the dock |
| D-pad left | Open the category wheel | Move left in the dock |
| A | Open the full details view | Activate the focused action |
| B | Tap: back. Hold: launch | Tap: step back out |
| X | Remove the focused game | Remove the focused game |
| Y | Add a game | Hold: edit the focused game |
| LB | Category wheel | Category wheel |
| START | Show or hide the top bar | Show or hide the top bar |

### Keyboard

| Key | Action |
|-----|--------|
| Up / Down | Move between games |
| Right | Step into the focused game |
| Left | Back out, category wheel, or close the filter panel |
| Enter | Open the full details view, or activate the focused action |
| C | Category wheel |
| `/` or Ctrl+K | Search |
| Escape | Close the details view or the open overlay |

---

## Save management

Save locations are detected when you add a game, and re-checked when a game runs. From the details panel:

| Action | What it does |
|--------|--------------|
| `BACKUP` | Takes a named snapshot of the current save state |
| `RESTORE` | Lists snapshots labelled `AUTO` or `MANUAL`. Pick one to restore or delete |

Automatic snapshots run every time a game exits. Settings also has a Backup Manager that lists every snapshot in the library and can clear them.

If a save folder is guessed wrong, the details panel can relink it, and the save migration flow can bring saves across from another folder or machine.

---

## Where your data lives

Everything stays local.

**Windows:**
```
%APPDATA%\com.silo.launcher\
├── games.json          # library metadata
├── games.json.bak      # previous good copy, written on save
├── settings.json       # preferences, including the SteamGridDB key
├── discovery.json      # scan folders and ignored games
├── wallpapers\         # wallpapers, logos and grids
└── saves\<game-id>\    # compressed save snapshots

%LOCALAPPDATA%\com.silo.launcher\logs\   # silo.log
```

**Linux:**
```
~/.local/share/com.silo.launcher/
├── games.json
├── games.json.bak
├── settings.json
├── discovery.json
├── wallpapers/
└── saves/<game-id>/

~/.local/state/com.silo.launcher/logs/   # silo.log
```

Settings has **Open Logs Folder** for when something needs a closer look. Nothing is sent anywhere.

---

## Build from source

### Prerequisites

- Node.js (LTS) and npm
- Rust stable (the crate targets 1.77.2 or newer)
- **Windows:** Windows 10 or 11 with WebView2, and the MSVC build tools
- **Linux:** WebKitGTK and the other Tauri system libraries

```bash
# Ubuntu / Debian / Mint
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev pkg-config libglib2.0-dev libgtk-3-dev libxdo-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev

# Arch
sudo pacman -S webkit2gtk-4.1 libayatana-appindicator librsvg patchelf openssl pkg-config glib2 gtk3 libxdo soup3 javascriptcoregtk-4.1

# Fedora
sudo dnf install webkit2gtk4.1-devel libayatana-appindicator-gtk3-devel librsvg2-devel patchelf openssl-devel pkgconfig glib2-devel gtk3-devel libxdo-devel soup3-devel javascriptcoregtk4.1-devel
```

### Dev and release builds

```bash
git clone https://github.com/antnjhn/SILO.git
cd SILO
npm install

npm run dev     # tauri dev, live reload
npm run build   # tauri build, release binary and bundles
```

The release binary lands at `src-tauri/target/release/silo.exe` (`silo` on Linux). Tauri's own bundles land in `src-tauri/target/release/bundle/`.

The renderer is plain ES modules served straight from `src/renderer`, so there is no frontend build step and no bundler in the loop.

### Custom Windows installer

SILO's own installer lives in `installer/` and is independent of the Tauri bundle. It packages the existing release binary, so build the app first.

```powershell
cd installer
.\build.ps1
```

Output: `installer/dist/SILO-Setup-<version>.exe`. The script finds `makensis.exe` in the Tauri cache or in `Program Files`, and accepts `-Makensis <path>` and `-AppSource <path>` if either needs pointing somewhere else. The installer is per user and never touches `%APPDATA%\com.silo.launcher`.

### Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml   # Rust unit and integration tests
node --test src/renderer/js/gamepad.test.mjs      # gamepad state machine
```

---

## Project layout

```
src/renderer/            frontend, no build step
├── index.html           all markup, one document, pages inside #page-host
├── style.css            layout, rail, panels, toasts
├── tokens.css           colour and spacing tokens, Inter @font-face
├── assets/              icon, bundled Inter, store logos
└── js/
    ├── app.js           boot, keyboard and window wiring
    ├── library.js       rail, categories, details panel, action dock
    ├── gamepad.js       polling state machine and input mapping
    ├── onboarding.js    first run setup
    ├── discovery.js     folder scanning and the add prompt
    ├── modals.js        add and edit game, metadata picker
    ├── artpicker.js     SteamGridDB browser
    ├── settings.js      settings page
    ├── stats.js         statistics page
    ├── backups.js       SaveGuard UI and Backup Manager
    ├── pages.js         page switcher
    ├── background.js    the single animated background layer
    ├── utils.js         toasts, including the determinate progress toast
    └── api.js           typed wrappers over the Rust commands

src-tauri/src/
├── lib.rs               app setup, command registration, startup work
├── commands.rs          library, launch, files, backup and window commands
├── metadata.rs          SteamGridDB search, art selection, the art sweep
├── saveguard.rs         save detection, snapshots, restore
├── library_import.rs    Steam, Epic and GOG import
├── discovery.rs         scan folders and discovery state
├── uninstaller.rs       uninstaller discovery and execution
├── game_launch.rs       launching, process monitoring
├── gaming_session_*.rs  session tracking and recovery
├── display.rs           resolution and display information
├── settings.rs          settings persistence
├── tray.rs              tray icon and close behaviour
└── window_lifecycle.rs  fullscreen, focus, window state

installer/               SILO's custom NSIS installer and uninstaller
```

---

## Stack

| Layer | Technology |
|-------|------------|
| Shell | [Tauri 2](https://v2.tauri.app/) |
| Backend | Rust: process management, filesystem work, save detection, HTTP to SteamGridDB |
| Frontend | Vanilla HTML, CSS and JavaScript modules |
| Type | Inter, bundled locally |

The interface is flat by design: no gradients, glows or blur, one animated background layer, and a single accent colour.

---

## Linux support

### Supported launchers and runtimes

- **Steam:** native Linux games and Proton titles
- **Heroic Games Launcher:** Epic Games Store and GOG
- **Lutris:** games managed by Lutris, added by path
- **Flatpak:** Steam, Heroic and other sandboxed launchers
- **Wine and Proton:** standalone prefixes and Proton-GE
- **Native:** AppImages, shell scripts and ELF binaries

### Game discovery

| Source | Method |
|--------|--------|
| Steam | Parses `libraryfolders.vdf` and `appmanifest_*.acf` from `~/.local/share/Steam`, `~/.steam/steam` and Flatpak Steam |
| Epic and GOG via Heroic | Reads `~/.config/heroic/legendaryConfig/legendary/installed.json` and `~/.config/heroic/gog_store/installed.json` |
| Flatpak paths | `~/.var/app/com.valvesoftware.Steam/...`, `~/.var/app/com.heroicgameslauncher.hgl/...` |

### SaveGuard on Linux

SaveGuard watches:

- `~/.local/share/` and `~/.config/`
- `~/Documents` and `~/Saved Games`
- Steam Proton `compatdata` directories
- Wine prefixes under `~/.wine/drive_c/users/...`

### Known limitations

- Some Windows-only games still need manual Wine or Proton configuration
- SteamGridDB artwork needs a free API key, same as Windows
- The AppImage needs FUSE (`libfuse2`) to run
- SILO's custom installer is Windows only. On Linux, use the `.deb` or the AppImage

---

## Troubleshooting

**No artwork.** Add a SteamGridDB API key in Settings, then restart SILO. The launch sweep fills in anything missing and shows its progress in a toast.

**A game is missing from the import.** Store imports only read what that store has installed. Add the game by path instead, or add its folder to the scan folders.

**Saves are not detected.** Open the game's details, relink the save folder, and launch the game once so SILO can confirm the paths.

**The window will not close.** Settings, **When closing SILO**, controls whether the window hides to the tray instead of exiting. Quitting from the tray always shuts SILO down.

---

## License

[![MIT License](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)

### Original software

The SILO source code and project documentation are licensed under the [MIT License](LICENSE), Copyright (c) 2026 Anton John.

### Branding

The name "SILO", the SILO logo and all associated branding are all rights reserved and are not covered by the MIT License. See [TRADEMARKS.md](TRADEMARKS.md) for the brand exclusions and usage guidelines.

### Third-party assets

Third-party dependencies, fonts and artwork are distributed under their own licenses. Inter is licensed under the SIL Open Font License 1.1. Everything else is listed in `Cargo.lock` and `package-lock.json`.
