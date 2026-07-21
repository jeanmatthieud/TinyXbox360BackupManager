<img alt="TinyXbox360BackupManager Logo" width="128" src="assets/TinyXbox360BackupManager-256x256.png" align="left">

### `TinyXbox360BackupManager`<br><sub><sup>:star: A tiny game backup manager for the Xbox 360</sup></sub>

[![release: vX.X.X](https://img.shields.io/github/v/release/jeanmatthieud/TinyXbox360BackupManager)](https://github.com/jeanmatthieud/TinyXbox360BackupManager/releases/latest)
[![license: GPL-3.0](https://img.shields.io/github/license/jeanmatthieud/TinyXbox360BackupManager)](https://github.com/jeanmatthieud/TinyXbox360BackupManager/blob/main/COPYING)

<br>

> [!CAUTION]
> TinyXbox360BackupManager is intended strictly for legal homebrew use and is not affiliated with or endorsed by Microsoft.
> Use of TinyXbox360BackupManager for pirated or unauthorized copies of games is strictly prohibited.

> [!WARNING]
> This project has just started. The modifications and adaptations from the upstream project are being made via AI.

<img align="center" alt="App Screenshot" src="assets/screenshot.png">

Game backup manager for modified **Xbox 360** (using [Aurora](https://phoenix.xboxunity.net) dashboard), cross-platform (Linux, Windows, macOS), with no dependencies to install: everything is built into the binary.

Inspired by [TinyWiiBackupManager](https://github.com/mq1/TinyWiiBackupManager): just as the Wii plays Wii and GameCube games, the Xbox 360 plays Xbox 360 and original Xbox games.

## :sparkles: Features

- **Lightweight & Fast**: Native app written in Rust and Slint, no Electron!
- **Cross-Platform**:
  - :window: Windows 7+ | x86 (32-bit), x64 (64-bit), arm64 (Qualcomm Snapdragon etc.)
  - :apple: macOS 10.14+ | x86_64 (Intel), arm64 (Apple Silicon/M1+)
  - :penguin: Linux (glibc 2.31+) | x86 (32-bit), x86_64 (64-bit), arm64 (Raspberry PIs etc.)

#### :video_game: Game Management

Provide an **ISO** image or an **Arcade (XBLA) game** — as a `.7z`/`.zip` archive or a bare STFS package — and the application does the rest:

| Detected input type | Processing | Destination |
|---|---|---|
| Xbox 360 game ISO (`default.xex`) | Conversion to **GOD** (Games on Demand) | `Content/0000000000000000/<TitleID>/00007000/` |
| Original Xbox game ISO (`default.xbe`) | **Extraction** of the content | `Games/<Game Name>/` |
| Install / **Expansion Installer** disc (no executable; contains DLC and title updates) | Extraction and merging of the `Content` folder | `Content/0000000000000000/<TitleID>/<type>/` |
| **Arcade (XBLA) game** (`.7z`/`.zip` archive) | Extraction, verification of the **Arcade** package (STFS metadata: title, TitleID) | `Content/0000000000000000/<TitleID>/000D0000/` |
| STFS package (`LIVE`/`CON `/`PIRS`, usually no extension) | Installed as-is according to its content type | `Content/0000000000000000/<TitleID>/<type>/` |

**DLC and title updates** found alongside the Arcade game in an archive are installed too (respectively in `00000002/` and `000B0000/`), so unlocks work out of the box. They can also be added individually as bare STFS packages.

The library is managed **directly on your chosen target**:

- **USB drive / local folder (FAT32)**: games are written directly in the correct format.
- **FTP console (Aurora)**: the game list is read from the console, added ISOs are converted locally then pushed directly to the internal drive `Hdd1`, and deletion is done remotely (one connection at a time, as required by the console's FTP server).
- **Covers**: retrieved automatically from [XboxUnity](https://www.xboxunity.net) (local cache).

> [!WARNING]
> Title updates for original Xbox (OG) games are not yet supported.

Homebrew management is intentionally not currently supported.

<br>

## :arrow_down: Downloads

<table>
  <tr>
    <td width="9999px"><strong>:window: Windows</strong></td>
  </tr>
  <tr>
    <td>
      :arrow_right: <a href="https://github.com/jeanmatthieud/TinyXbox360BackupManager/releases/latest">Download standalone binary</a>
    </td>
  </tr>
</table>

<table>
  <tr>
    <td width="9999px"><strong>:apple: macOS</strong></td>
  </tr>
  <tr>
    <td>
      :arrow_right: <a href="https://github.com/jeanmatthieud/TinyXbox360BackupManager/releases/latest">Download universal DMG</a>
    </td>
  </tr>
</table>

<table>
  <tr>
    <td width="9999px"><strong>:penguin: Linux</strong></td>
  </tr>
  <tr>
    <td>
      :arrow_right: <a href="https://github.com/jeanmatthieud/TinyXbox360BackupManager/releases/latest">Download AppImage</a>
    </td>
  </tr>
</table>

<br>

## :rocket: Usage

The interface resembles TinyWiiBackupManager: sidebar (Games, Toolbox, Settings, About), grid or table view, 360/XBLA/OG filters, search, conversion queue, notifications, drag-and-drop.

1. Click the **hard drive** icon (at the bottom of the sidebar) then select the target: **USB drive / local folder**, or **FTP console** (IP + Aurora credentials, `xboxftp`/`xboxftp` port 21 by default, with connection test).
2. **Games** page: the list reflects the content of the target (local or remote). Click the **+** button (or drag and drop) to add ISOs, XBLA archives or STFS packages; the application detects the type of each input and starts GOD conversion, extraction or package installation, directly to the target (tracked in the conversion queue and status bar).
3. On the console: Aurora > Settings > Content Paths, add `Hdd1:\Content\0000000000000000\` (and `Hdd1:\Games\`), Scan Depth 3-4, then run a scan.

For **multi-disc games with an installation disc** (e.g., GTA V): the "Play" disc is converted to GOD, the installation disc is detected as a content disc and its `Content` folder is merged in the right place — simply provide both ISOs to the application. The same applies to **Expansion Installer** discs (e.g., GTA IV: The Complete Edition): the DLC and title updates they contain are installed to their respective folders.

## :hammer_and_wrench: Compilation

```sh
cargo build --release
```

Build prerequisites on Linux (Debian/Ubuntu/Pop!_OS):

```sh
sudo apt-get install -y build-essential pkg-config libfontconfig1-dev
```

The binary is generated in `target/release/TinyXbox360BackupManager`.

## :computer: Technologies

Pure Rust, no runtime external dependencies:

- [Slint](https://slint.dev) — graphical interface
- [iso2god-rs](https://github.com/iliazeus/iso2god-rs) — ISO → GOD conversion
- [xdvdfs](https://crates.io/crates/xdvdfs) — reading/extraction of XDVDFS images ([extract-xiso](https://github.com/XboxDev/extract-xiso) equivalent)
- [suppaftp](https://crates.io/crates/suppaftp) — FTP client
- [XboxUnity](https://www.xboxunity.net) — Xbox360 covers and title updates
- [MobCat](https://github.com/MobCat/MobCats-original-xbox-game-list) — Xbox covers

## :scroll: License

GPL-3.0-only. Based on the work of Manuel Quarneti (TinyWiiBackupManager), iliazeus (iso2god-rs) and antangelo (xdvdfs).
