# Aurora import format — notes

How the app hands Aurora the assets of each game (cover, icon, banner,
background, description) through Aurora's **import folder**. Implemented in
`crates/core/src/aurora_import.rs` (the folder) and
`crates/core/src/marketplace.rs` (the sources); the GUI side lives in the
Aurora card of the Device status page and the job queue.

An earlier draft wrote Aurora's `.asset` files into `Data/GameData` itself. It
was dropped before merging: that path is keyed by Aurora's own database row
(`<TitleId>_<DbId>`), so it needs `content.db` and an Aurora scan to have
happened first, encodes textures itself, and cannot carry metadata at all.

| | `.asset` files (dropped) | Import folder (built) |
|---|---|---|
| Key | `<TitleId>_<DbId>`: Aurora must have scanned the game first | `<TitleID>` only |
| Encoding | Ours (DXT5 / ARGB) | Aurora's: we drop a PNG/JPG |
| Metadata | Not possible without writing `content.db` | Plain text files |
| Control | Full: we see what exists and never overwrite | Opaque: Aurora decides |
| Effect | Immediate | When the user runs the import in Aurora |

## The format

Source: <https://consolemods.org/wiki/Xbox_360:Aurora_Import_Format>.
Everything lives in `Aurora\User\Import\<TitleID>\` and every item is optional.

| File | Content |
|---|---|
| `titlename.txt` | Title |
| `description.txt` | Description (most skins show 8 lines at most) |
| `publisher.txt`, `developer.txt` | Names |
| `releasedate.txt` | `YYYY-MM-DD` |
| `genre.txt` | One or more genres from the list below, comma-separated |
| `cover.{jpg,png,dds}` | 900x600 |
| `icon.{jpg,png,dds}` | 64x64 |
| `banner.{jpg,png,dds}` | 420x95 |
| `background.{jpg,png,dds}` | 1280x720 |
| `screenshotN.{jpg,png,dds}` | 1000x562, `N` = 1–20 (or `01`–`20`) |

Accepted genres (anything else shows as "Not Available"): Other, Action &
Adventure, Family, Fighting, Music, Platformer, Racing & Flying, Role Playing,
Shooter, Strategy & Simulation, Sports & Recreation, Board & Card Games,
Classics, Puzzle & Trivia.

Screenshots are the risky item: the wiki warns that importing them always adds
new entries, whatever the file names, and can corrupt the existing ones.

## Aurora's behaviour, as tested

Tested on a console on 2026-10-06, first with flat-colour samples for BioShock 2
(54540861: `A` a full set, `B` cover + title only, `ghost` a TitleID with no
game), then with the app's real output for a library of 20 games.

- The import is **global and manual**: *Settings > Assets > Import* goes
  through every folder at once. Scanning a new game does not consume its
  folder — Aurora tries its own internet download instead.
- **Unscanned title**: its folder is silently ignored and left in place, and no
  `GameData` folder is created. So a folder can be dropped ahead of the scan and
  is picked up by the first import run after it.
- **Scanned title**: every image and every text field is imported and shown.
- **Files are never consumed**: the folder is intact after an import, and
  running the import again re-applies it without error.
- **Overwrite is per item**: importing `B` after `A` replaced the cover and the
  title, and left the icon, banner, background and description of `A` alone.
  What is present in the folder wins; what is absent is untouched.
- A cover is shown only when it measures **exactly 900x600**: XboxUnity's
  Borderlands cover (897x600) was imported but never displayed, while the
  nineteen 900x600 ones were. The app now resizes any other size.
- Text files are read as **UTF-8**, with or without a BOM (accents, `®`, `™`
  and `–` all came out right); UTF-16 shows as garbage.
- Progressive JPEGs (Gears of War's cover) display fine.

Side findings from the `GameData` backups taken around the sample tests:

- Aurora creates **empty placeholder assets**: before the import, `GC`, `GL`,
  `BK` and `SS` were all 2048 bytes (a bare header), next to `GameCoverInfo.bin`
  and `GameOfferInfo.bin`. A file being there says nothing; only a size above
  2048 bytes means artwork.
- After importing `A`: `GC` 657 408 bytes, `BK` 985 088, `GL` 83 968 (icon and
  banner share it), `SS` still 2048 (no screenshot in the sample).
- The **DbId follows the scan, not the game**: uninstalling and re-adding the
  game moved its folder from `54540861_00000004` to `54540861_00000034`.
  Artwork keyed by DbId does not survive a reinstall; an import folder, keyed by
  TitleID, does.

Not checked: whether Aurora accepts a space after the comma in `genre.txt`
(the app writes none).

## Sources (Xbox 360 and XBLA)

Both addresses below derive from the TitleID alone, through the marketplace
product GUID `66acd000-77fe-1000-9115-d802<titleid>`:

- **Images**: `http://download.xbox.com/content/images/<guid>/1033/<file>` —
  the marketplace CDN, still answering. No API call needed.
- **Metadata**: `https://dbox.tools/api/marketplace/products/<guid>` — an
  archive of the marketplace catalogue, about 60 KB of JSON per title.

| Import file | Source | Field / file |
|---|---|---|
| `cover` | XboxUnity or its GitHub mirror (already in `covers.rs`) | `Large/<CoverID>.png` |
| `icon` | CDN, or the mirror's `Icons/<TitleID>.png` (same file) | `tile.png` |
| `banner` | CDN | `banner.png` (420x95) |
| `background` | CDN | `background.jpg` (1280x720) |
| `screenshotN` (not used) | CDN | `screenlg1.jpg` … `screenlgN.jpg` (1000x562) |
| `titlename.txt` | dbox.tools | the localization's `full_title`, else `default_title` |
| `description.txt` | dbox.tools | `reduced_description` / `full_description`, 46 locales |
| `publisher.txt` | dbox.tools | `publisher_name` |
| `developer.txt` | dbox.tools | `developer_name` |
| `releasedate.txt` | dbox.tools | `global_original_release_date`, cut to the date |
| `genre.txt` | dbox.tools | `categories`, mapped by dbox.tools' own category ids |

Checked against BioShock 2 (54540861), Halo 3, Black Ops II, GTA IV, Borderlands
and two XBLA titles (Super Meat Boy, Minecraft), then the 20-game library above.

## Limits

- **Screenshots** are not there for every title: none for BioShock 2, 9 to 11
  for Halo 3 (`screenlg1`, `screenlg2`… until the first 404). Not used anyway.
- **XBLA titles** come back as "Full Game - Super Meat Boy", in every language
  ("Version complète - ", "Vollversion - ", "完全版 - "), now and then twice
  ("Full Game - Full Game - Sonic The Hedgehog"). `reduced_title` never has it
  but is sometimes abbreviated ("PAC-MAN CE DX+"). The prefix is stripped on
  Arcade products (dbox.tools `product_type` 14) by comparing both titles, then
  by cutting at the first " - " (`marketplace::strip_full_game_prefix`).
- **Genres** match Aurora's list by name, except "Card & Board" (Aurora wants
  "Board & Card Games"). Kinect, Avatar and Educational have no equivalent and
  are dropped. The mapping is by dbox.tools' category ids (8 Shooter, 9 Action &
  Adventure…), which are its own numbering, not the marketplace's.
- **Release date**: for a game sold as Games on Demand it is the day it reached
  the marketplace, which can be years after the disc (Halo 3: 2010).
- **Original Xbox** games have no marketplace product (404 on both sides): they
  only get their cover (MobCat's database). dbox.tools knows their name
  (`/api/title_ids/<TitleID>`) but nothing else.
- **The marketplace box art** (`boxartlg.jpg`) is 219x300, front only: XboxUnity
  stays the source for the cover.
- **Extracted games** without a TitleID in the library are skipped.

## Risks

- `download.xbox.com` is plain HTTP (no HTTPS) and Microsoft can switch it off
  without notice. No mirror was found; the local cache softens the blow.
- dbox.tools is one person's project; its API calls itself "NOT finished" and
  makes no claim of accuracy. Everything is cached per TitleID (and per locale
  for the texts), and a miss is "no asset", never an error — only an unreachable
  source is, and then the title is skipped for a later run.

## What was built

Core:

- Import folder only: no `.asset` file is written and `content.db` is not read
  for this.
- A folder that exists is never touched again (it may be the user's own), and
  a title no source knows gets an empty one, which marks it as dealt with.
- Folders are left in place after an import. Detecting that an import happened
  would mean reading `GameData`, i.e. depending on Aurora's internals again.
  Instead the card offers to empty the import folder.
- Covers are brought to 900x600 (PNG) when they are any other size: stretched
  when already about that shape (897x600), fitted and centred on black
  otherwise (the portrait 300x420 fronts of original Xbox games).
- A folder is never written over: the import folder's listing must succeed
  (an unreadable one is an error, not an empty one), and a title whose write
  failed midway has its folder removed so a later run redoes it.
- `genre.txt` separates genres with a bare comma (`Shooter,Action & Adventure`).
- Titles and descriptions follow the language setting (ten marketplace locales
  in `marketplace::LOCALES`), with `en-us` as the fallback. The dbox.tools
  record is cached as served (`marketplace/<TitleID>/product.json`), so a
  language change costs no download; but a game already prepared is not
  redone after one.
- The covers cache is written atomically (`.tmp` then rename): after a scan,
  the cover download and the preparation job run side by side on it.
- `cargo run -p txbm-core --example aurora_import_check` runs it against a
  synthetic Aurora with real downloads; `cargo test -p txbm-core marketplace`
  covers the Arcade prefix.

GUI:

- Both writes (preparing, emptying the folder) are `QueuedJob`s, so they run
  one at a time with the other writes, with progress and cancel.
- Aurora card (Device status): the two steps, "Prepare the Aurora game assets of
  the N remaining games", "Delete the Aurora game assets to import" (confirmed),
  and in the footer "ready to import: N of M games" — M being the library's
  games with a TitleID, N those with a folder, from one listing of the import
  folder.
- Setting "Prepare Aurora game assets when adding games", **off by default**
  and marked experimental, with the language drop-down (its rows come from
  `marketplace::LOCALES`, the only list). When on, a successful
  addition sets `State::aurora_import_wanted`, and the next library rescan
  (which is what knows an archive's TitleID) queues the preparation once the
  queue is idle.
- The end-of-preparation notification is sticky: it carries the step left to
  do on the console.

## Known weak spots (from the code review, left as is)

- `covers::download_cover` does not tell "no cover" from "source unreachable":
  a XboxUnity outage marks a title as having no cover, for good (its folder
  exists). Fixing it means changing that function's contract.
- A CDN that stops answering with anything but a 404 (403 once Microsoft
  retires it, DNS failure) fails every Xbox 360 title, description included,
  and nothing is ever prepared. Deliberate for a passing outage, since a failed
  title is retried; to be changed if the CDN goes for good.
- The automatic preparation sweeps the whole library, not just the games added;
  by construction only the titles without a folder cost anything, so what gets
  replayed on every add is the titles whose sources failed.
- A new game added while a manual preparation is still running is only
  prepared at the next library rescan.
