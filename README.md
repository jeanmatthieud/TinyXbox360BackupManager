<img alt="TinyXbox360BackupManager Logo" width="128" src="assets/TinyXbox360BackupManager-256x256.png" align="left">

### `TinyXbox360BackupManager`<br><sub><sup>:star: A tiny game backup manager for the Xbox 360</sup></sub>

[![release: vX.X.X](https://img.shields.io/github/v/release/jeanmatthieud/TinyXbox360BackupManager)](https://github.com/jeanmatthieud/TinyXbox360BackupManager/releases/latest)
[![license: GPL-3.0](https://img.shields.io/github/license/jeanmatthieud/TinyXbox360BackupManager)](https://github.com/jeanmatthieud/TinyXbox360BackupManager/blob/main/COPYING)<br />
<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>

<br>

> [!CAUTION]
> TinyXbox360BackupManager is intended strictly for legal game backup use and is not affiliated with or endorsed by Microsoft.
> Use of TinyXbox360BackupManager for pirated or unauthorized copies of games is strictly prohibited.

> [!WARNING]
> This project has just started. The modifications and adaptations from the upstream project are being made with the help of IA.

<img align="center" alt="App Screenshot" src="assets/screenshot.png">

## Put your game backups on your Xbox 360 — the easy way

You have a modded **Xbox 360** (running the [Aurora](https://phoenix.xboxunity.net) dashboard) and some game backups on your computer.
This app copies them onto your Xbox360 (or a USB drive) in the format the console understands — **you don't need to know how any of it works.**

- :package: **Drop in your game file, it does the rest** — it figures out the type and prepares it automatically.
- :compass: **It guides you** — it detects your plugged-in USB drives or remote Xbox360, and once connected, looks at what's already there to suggest where to put your games.
- :framed_picture: **Covers appear on their own** — box art is downloaded for you.
- :arrows_counterclockwise: **Three ways in** — send games to your Xbox360 over Ethernet / Wi‑Fi (FTP), to a USB drive plugged into your PC, or straight onto the **console's own hard drive** connected to your computer (experimental, advanced users).
- :feather: **Tiny and self-contained** — one small app, nothing else to install.

It also handles **original Xbox** games, just like the Wii plays GameCube games.
Inspired by [TinyWiiBackupManager](https://github.com/mq1/TinyWiiBackupManager).

## :rocket: Get started in 3 steps

1. **Open the app and pick where your games go.** Click the **connect icon** (bottom-left), then choose **Local USB drive**, **Xbox 360 internal hard drive** or **Remote Xbox360 over network**. For a USB drive, pick it from the list of detected removable drives (FAT32 only); for the console's own drive — experimental, and reserved for advanced users — connect it to your computer and pick it from the list (see [Using the console's hard drive](#electric_plug-using-the-consoles-hard-drive)); for the remote Xbox360, enter its IP + Aurora login — the defaults `xboxftp` / `xboxftp` usually just work.
2. **Confirm the setup.** A short **Content analysis** window opens, looks at your drive/console, and pre-fills where games should be stored. In most cases you can just click **Confirm** — it remembers your choice, so you only do this once.
3. **Add your games.** Click the **+** button (or drag & drop your files). The app converts/copies each one to the right place. That's it — your games show up in the grid with their covers.

> :bulb: Aurora needs to be told which folders to scan. The app checks this for you in the **Toolbox** and, if something is missing, shows you exactly what to add.

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

## :sparkles: What it can do

- **Send games to a console (Aurora over FTP) or a USB drive / local folder.** The game list reflects what's really on the target.
- **Accepts many kinds of input** and picks the right processing automatically: Xbox 360 ISOs, original Xbox ISOs, Arcade (XBLA) archives, install / expansion discs, and bare STFS packages.
- **Multi-disc games with an install disc** (e.g. GTA V) and **Expansion Installer discs** (e.g. GTA IV: The Complete Edition) are handled — just provide both ISOs; DLC and title updates get installed to the right place so unlocks work out of the box.
- **Covers** from [XboxUnity](https://www.xboxunity.net) and [MobCats](https://github.com/MobCat/MobCats-original-xbox-game-list) (with a local cache).
- **Cross-platform**, native, no dependencies to install:
  - :window: Windows 10+ | x86 (32-bit), x64 (64-bit), arm64 (Qualcomm Snapdragon etc.)
  - :apple: macOS 10.14+ | x86_64 (Intel), arm64 (Apple Silicon/M1+)
  - :penguin: Linux (glibc 2.31+) | x86 (32-bit), x86_64 (64-bit), arm64 (Raspberry Pis etc.)

> [!WARNING]
> Title updates for original Xbox (OG) games are not yet supported. Homebrew management is intentionally out of scope for now.

<br>

---

## :gear: How it works (technical details)

Everything below is optional reading — the app does it for you.

### Input types and processing

Provide an **ISO** image or an **Arcade (XBLA) game** (a `.7z`/`.zip` archive or a bare STFS package); the app detects the input and converts/installs it:

| Detected input type | Processing | Goes to |
|---|---|---|
| Xbox 360 game ISO (`default.xex`) | Conversion to **GOD** (Games on Demand)<br />*or*<br />**Extraction** of the content | GOD folder → `<TitleID>/00007000/`<br />*or*<br />Extracted-XEX folder → `<Game Name>/` |
| Original Xbox game ISO (`default.xbe`) | **Extraction** of the content | Extracted-XBE folder → `<Game Name>/` |
| Install / **Expansion Installer** disc (no executable; contains DLC and title updates) | Extraction and merge of the `Content` folder | GOD folder → `<TitleID>/<type>/` |
| **Arcade (XBLA) game** (`.7z`/`.zip` archive) | Extraction, verification of the **Arcade** STFS package (title, TitleID) | GOD folder → `<TitleID>/000D0000/` |
| STFS package (`LIVE`/`CON `/`PIRS`, usually no extension) | Installed as-is per its content type | GOD folder → `<TitleID>/<type>/` |

**DLC and title updates** bundled with an Arcade game in an archive are installed too (respectively in `00000002/` and `000B0000/`), so unlocks work out of the box. They can also be added individually as bare STFS packages.

For Xbox 360 games, the choice between **GOD conversion** and **XEX extraction** is not automatic — it's a user setting. On the Settings page, a dropdown lets you pick the target format.

### Storage folders (guided setup + `.txbm.json`)

The app installs and scans games in **three configurable folders**, one per storage format:

| Format | Default folder | Content |
|---|---|---|
| **GOD** (converted Xbox 360) | `Content/0000000000000000` | `<TitleID>/…` containers |
| **XBE** (extracted original Xbox) | `Games Xbox` | one sub-folder per game (`default.xbe`) |
| **XEX** (extracted Xbox 360) | `Games Xbox360` | one sub-folder per game (`default.xex`) — reserved for the planned 360-extraction feature |

When you connect a target, the **Content analysis** window resolves these folders in this order:

1. a `.txbm.json` file already saved on the target (your confirmed choice — takes priority);
2. the drive's / console's own **Aurora scan paths** (read from Aurora's databases), used to detect where your games already live;
3. built-in defaults (the table above).

You confirm (or adjust) the three folders, and the app writes a small **`.txbm.json`** at the root of the target so it never has to ask again. On a USB drive the paths are stored **relative to the drive**, so it keeps working if the drive is later mounted elsewhere.

### Targets

- **USB drive / local folder (FAT32):** plugged-in removable drives are auto-detected and listed, with non-FAT32 ones flagged as unsupported; games are then written directly in the correct format.
- **Xbox360 over FTP (Aurora):** the game list is read from the console, added games are converted locally then pushed to the console; deletion is done remotely. Only **one FTP connection at a time** is used, as required by the console's FTP server.
- **Console hard drive (FATX360), experimental:** the drive is opened as a raw device and its `data` partition — the one the console calls `Hdd1` — is read and written directly, so folders, the game list, the `.txbm.json` configuration and Aurora's own databases are the very same ones the console sees. Added games are converted locally then copied across. The drive is opened **read-only unless a write is actually taking place**, and only one session at a time. See [Using the console's hard drive](#electric_plug-using-the-consoles-hard-drive) for the disk permissions this needs.

### Aurora scan paths

Aurora only lists games from folders it is told to scan. The **Toolbox** page reads Aurora's configured scan paths and compares them with the folders this app uses:

- for a **console over FTP**, and for a **USB key that carries an Aurora install**, each storage folder is flagged as scanned by Aurora or not;
- if a folder isn't scanned yet, the app shows the exact path to add in *Aurora → Settings → Content → Manage Paths* (Scan Depth 3+), then a rescan.

## :electric_plug: Using the console's hard drive

> [!WARNING]
> **Experimental — for advanced users.** This target writes to the console's FATX filesystem
> directly, through a young implementation, and it needs raw disk access: privileges that let a
> mistake reach any disk on your computer. The steps below assume you are comfortable with a
> terminal, with device names, and with the idea of backing the drive up first.
> Prefer the USB drive or the network target if any of that is not the case.

You can take the hard drive out of your Xbox 360, connect it to your computer (SATA port or a USB adapter) and manage your games on it directly — no network, no USB key.

> [!CAUTION]
> **Never let your computer format, initialise or "repair" this drive.** Your operating system does not understand FATX, the filesystem the Xbox 360 uses, and will offer to fix it. Accepting erases every game on it.
> - :window: Windows: *"You need to format the disk before you can use it"* → **Cancel**. In Disk Management the drive shows as *Not initialized*: leave it that way.
> - :apple: macOS: *"The disk you inserted was not readable by this computer"* → **Ignore**, never *Initialize*.
> - :penguin: Linux: the drive simply does not appear in your file manager. That is the expected behaviour.
>
> Back the drive up before your first write.

An Xbox 360 drive has no partition table, so your system never mounts it and it is invisible to the usual drive lists. The app reads and writes the raw device instead — **and that needs privileges**. Here is how to grant them on each system.

### :window: Windows

Raw disk access (`\\.\PhysicalDriveN`) is reserved to administrators. Right-click `TinyXbox360BackupManager-vX.X.X-windows-x64.exe` and choose **Run as administrator** (or set it once in *Properties → Compatibility → Run this program as an administrator*).

Nothing else to configure: the drive shows up in the app's picker as soon as the app is elevated. Launched normally, the app cannot even read enough to tell your disks apart — they all appear greyed out as *"access denied"*.

### :apple: macOS

Whole disks (`/dev/diskN`) belong to `root`, so the app has to be started from Terminal as an administrator:

```sh
sudo /Applications/TinyXbox360BackupManager.app/Contents/MacOS/TinyXbox360BackupManager
```

Two consequences worth knowing:

- The app then runs as `root`, so the settings and cover cache it uses (`~/Library/Application Support/net.jeanm.TinyXbox360BackupManager`) may be created or rewritten with root ownership. If a later normal launch behaves oddly, hand them back:
  `sudo chown -R "$USER" ~/Library/Application\ Support/net.jeanm.TinyXbox360BackupManager`
- Double-clicking the app in Finder will *not* give it disk access: launched that way, every disk appears greyed out as *"access denied"*. Only the Terminal command above works.

### :penguin: Linux

Block devices belong to `root:disk` and are not readable by your account. Do **not** run the AppImage with `sudo`: on Wayland a root process cannot connect to your desktop session, and it would write its settings into `/root`. Grant access to the *device* instead, and keep running the app as yourself.

First find the drive — an Xbox 360 disk shows no partitions and no filesystem:

```sh
lsblk -dno NAME,SIZE,MODEL
```

#### Option A — one command, for the time of a session

```sh
sudo setfacl -m u:$USER:rw /dev/sdb        # replace sdb with your drive
./TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage
```

This grants read/write on that one device, to you alone. The permission disappears when the drive is unplugged or the machine reboots, which makes it the safest way to try things out — but you have to repeat it every time.

`setfacl` comes from the `acl` package, present on most desktop distributions (`sudo apt install acl` on Debian/Ubuntu/Pop!_OS if it is missing).

#### Option B — a udev rule, applied automatically

```sh
sudo tee /etc/udev/rules.d/99-xbox360-hdd.rules >/dev/null <<'EOF'
# A whole disk with neither a partition table nor a recognised filesystem:
# what an Xbox 360 drive looks like to Linux. Grant read/write to one user.
ACTION=="add|change", SUBSYSTEM=="block", ENV{DEVTYPE}=="disk", \
  ENV{ID_PART_TABLE_TYPE}!="?*", ENV{ID_FS_TYPE}!="?*", \
  RUN+="/usr/bin/setfacl -m u:YOUR_USERNAME:rw /dev/%k"
EOF
sudo udevadm control --reload
```

Replace `YOUR_USERNAME`, and check that `setfacl` really lives at that path (`command -v setfacl`) since udev runs with a minimal `PATH`. Then unplug and replug the drive; `getfacl /dev/sdb` shows whether it worked.

**What this rule can and cannot do.** Linux has no idea what FATX is — `blkid`, which is what feeds udev, has no prober for it — and an Xbox 360 disk carries no partition table either. So the rule cannot ask for *"an Xbox 360 disk"*; the closest it can get is *"a whole disk the system recognises nothing on"*. In practice:

- Your system and data disks are **not** affected: they have a partition table or a known filesystem, which the two `!="?*"` conditions exclude.
- But a **blank or freshly-wiped disk or USB stick** matches too, and your user would get read/write on it. The consequences are limited — access is granted to one named user, on a disk that holds nothing — yet it is a genuinely wider rule than it looks.
- Access is granted to **one user**, not to everyone, and only as an ACL on the device node; nothing about the system's own permissions changes.

If that trade-off bothers you, Option A stays the tightest choice.

## :hammer_and_wrench: Compilation

```sh
cargo build --release
```

Build prerequisites on Linux (Debian/Ubuntu/Pop!_OS):

```sh
sudo apt-get install -y build-essential pkg-config libfontconfig1-dev
```

The binary is generated in `target/release`.

## :computer: Technologies

Pure Rust, no runtime external dependencies:

- [Slint](https://slint.dev) — graphical interface
- [iso2god-rs](https://github.com/iliazeus/iso2god-rs) — ISO → GOD conversion
- [xdvdfs](https://crates.io/crates/xdvdfs) — reading/extraction of XDVDFS images ([extract-xiso](https://github.com/XboxDev/extract-xiso) equivalent)
- [suppaftp](https://crates.io/crates/suppaftp) — FTP client
- [XboxUnity](https://www.xboxunity.net) — Xbox360 covers and title updates
- [MobCats](https://github.com/MobCat/MobCats-original-xbox-game-list) — Xbox covers

## :scroll: License

GPL-3.0-only. Based on the work of Manuel Quarneti (TinyWiiBackupManager), iliazeus (iso2god-rs) and antangelo (xdvdfs).
