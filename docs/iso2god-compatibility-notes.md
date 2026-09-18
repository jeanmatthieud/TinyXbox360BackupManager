# Iso2God compatibility list vs. our install mechanisms

Cross-check of the compatibility / installation notes published by
[r4dius/Iso2God](https://github.com/r4dius/Iso2God#compatibility--installation-notes)
against what TinyXbox360BackupManager actually does. That list is not written
for the exact Iso2God build we depend on, but the output shape is the same, so
its "Instructions" column is a good proxy for the manual steps our automation
should be removing.

The conclusions below were checked against a local set of 31 Redump dumps
(zipped ISOs), by parsing the XDVDFS volume straight out of the deflate stream:
game-partition probe, root directory table, `Content/…` tree, then the STFS
header (`0x344` content type, `0x360` TitleID, `0x411`/`0x1691` names — the same
fields `crates/core/src/stfs.rs` reads) of every package found. Nothing was
extracted to disk. Observations from those dumps are marked **[measured]**.

A **second campaign** (2026-09-17) re-ran the same collection, grown to 33 usable
archives, through `crates/core/examples/mix_disc_survey.rs` — this time through
the app's own readers (`XdvdImage`, `stfs::inspect_reader`) rather than an
ad-hoc parser, so that any criterion it validates is one the app can actually
compute at run time. It also reads the *disc's own* `default.xex`, which the
first pass never did. Its findings are marked **[survey]** and are what settles
gap 1 below. Three *Borderlands Triple Pack* archives are excluded: they are
incomplete downloads.

## What our mechanisms already cover

### Discs that carry both an executable and a `Content/0000000000000000` tree

This is the shape behind most of the "No GOD — extract ISO and copy
`content\0000000000000000\<TitleID>\00000002` to HDD" rows: Battlefield 4 D1,
GTA V D1, Forza Motorsport 3 D2, Batman/CoD/Alien Isolation equivalents, and
also Battlefield 3 D1 and Halo 4 D2, which the Iso2God list does not mention.

`iso_info.rs` gives `Content/0000000000000000` priority over the executable and
returns `IsoKind::BundledContent`; `convert.rs` then extracts the disc and runs
`find_installable_packages_excluding(&tmp, &["$SystemUpdate"])`, filing every
package under the TitleID and content type read from its **own STFS header**.
So the "copy that folder to the HDD" step happens automatically, and the
matching lookups are case-insensitive (`XdvdImage::find_in`, `util::find_dir_ci`)
— which matters, since real dumps spell the folder `content` or `Content`
depending on the publisher. **[measured]**

### The `FFED2000\FFFFFFFF` rename

Bioshock 1/2/Infinite D2, Skyrim Legendary D2, Fallout 3 / New Vegas D2,
Oblivion D2, Mafia 2 D2, Mass Effect D2, Dishonored D2, Dark Souls II D2,
Dragon's Dogma D2, Forza 2 D2, Saints Row The Third D1, Saints Row IV D1.

Instruction: *rename* `content\0000000000000000\FFED2000\FFFFFFFF` to
`Content\0000000000000000\<real TitleID>\00000002`.

**[measured]** On Skyrim Legendary Edition (Disque 2) and Fallout New Vegas
Ultimate (Disque 2), the disc does carry a `default.xex` (an
`ExpansionInstaller`), so it takes the `BundledContent` path — and the packages
sitting in `FFED2000/FFFFFFFF` declare `type=00000002` with
`titleid=425307E6` (Skyrim) and `425307E0` (Fallout NV) in their own headers,
exactly the values the manual instruction tells you to rename to. Our path
performs that rename implicitly; the placeholder folder names are never used.

The same pass also picks up the `title_update.bin` left at the disc root
(`type=000B0000`, same TitleID), which the Iso2God note does not even mention,
and installs it under `000B0000` — renamed to its SHA-1 so two unrelated
`title_update.bin` cannot overwrite each other.

### Package types that must *not* be installed

**[measured]** Those discs also carry `nxeart` (`type=00030000`) and, on Halo 4
D2, an `AvatarAssetPack` (`type=00008000`, TitleID `FFFE07DF`). Both are real
STFS packages sitting at the disc root. `find_installable_packages_excluding`
only keeps Arcade / DLC / title-update types, so they are correctly skipped.

**[survey]** The `$SystemUpdate` exclusion is not a precaution, it is load-
bearing: **all 33** dumps carry `\$SystemUpdate/su20076000_00000000`, a title
update (`type=000B0000`) whose TitleID is the dashboard's own `FFFE07D1`.
Without the exclusion every single disc would file a dashboard update under a
bogus `FFFE07D1/000B0000` folder.

One side effect worth knowing: because the scan walks the whole disc rather
than just `Content/0000000000000000`, Halo 4 D2's root `AvatarAwards`
(`type=00000002`, TitleID `4D530919`) *is* installed as DLC. That is the same
breadth that makes the root `title_update.bin` work, and avatar awards do
belong under the game's content folder, so this looks desirable rather than
harmful — but it is not something the Iso2God instructions ask for.

### Package file names vs. the FATX limit

**[measured]** Redump content packages are named with a 42-character hash
(`0A0DBA135FA11394F664E423B7795C8F7C84926D42`). `FATX_MAX_NAME` is 42, so
`install_stfs_package` truncates nothing. Exactly at the limit, with no margin.

### Multi-disc sets where every disc is a GOD

Blue Dragon, Dead Space 2/3, Wolfenstein TNO; locally Castlevania: Lords of
Shadow, Mass Effect 3, Battlefield 3 D2, GTA V D2, Battlefield 4 D2.

`god::FileLayout` is built from `execution_info` (TitleID + MediaID), so each
disc gets its own package folder under the shared `<TitleID>`.
`cleanup_partial` in `crates/core/src/god.rs` exists precisely so cancelling
one disc does not delete its siblings, the DLC folder or the title updates.

### The zero-sized `Disc.N` marker

**[measured]** Castlevania: Lords of Shadow D1 has a root entry `Disc.1` with
`size=0` (D2 has `Disc.2`), sitting one sector after the root table itself.
That is the exact case documented in `CLAUDE.md`: `iso2god::iso::IsoReader`
stops at the first zero-sized entry and reads back an empty root, which would
mean "no `default.xex`" and an under-reported used size. Reading through
`crate::xdvd` is what makes this dump work — so the workaround is not
theoretical, it is needed by a game sitting in an ordinary collection.

### Tetris: The Grand Master Ace — "won't work with Padding 'Remove all', use 'Partial'"

We are on the safe side. `XdvdImage::max_used_prefix_size()`
(`crates/core/src/xdvd.rs`) takes the end of the furthest used region and
`convert_to_god` truncates only the tail (`data_size = max_used.min(available)`);
data inside the image is never rearranged. That is the "Partial" behaviour.
No option needs to be exposed.

### No "disc 1 is the game" assumption

Detection is per image, so the inverted sets — Alien Isolation, Battlefield 4,
GTA V, MGS V, Saints Row — where **disc 2** is the one that becomes the GOD
work with no user intervention. **[measured]** on BF4 and GTA V.

## Gaps and exceptions

### 1. The "Mix" method — Splinter Cell: Blacklist D2 (confirmed bug)

Instruction: install as GOD **and** copy `Content\0000000000000000\555308B6\00000002`.

**[measured]** That disc has a `default.xex` *and*
`Content/0000000000000000/555308B6/00000002/EE6A…55` (3.0 GB). Since
`has_bundled_content` wins over the executable, we classify it as
`BundledContent`, install the texture pack and **never build the GOD** — so the
second half of the campaign is silently lost. It is the only entry in the list
needing both treatments on one image.

#### The add dialog cannot carry the decision

The obvious idea — show the detected `IsoKind` before confirming and let the
user force a GOD — does not survive contact with how the library is stored.
For a `.zip`/`.7z`, `should_add_game` (`crates/gui/src/util.rs:68-76`) only calls
`archive::looks_valid`: the ISO is never opened. The `IsoKind` appears much
later, inside the job thread, through `install_archive` → `single_iso_in` →
recursion into `convert_into` (`convert.rs:662-672`), long after the modal is
gone. The whole local collection is zipped, so a dialog-side choice would cover
none of the real cases. Whatever decides has to decide **at conversion time**,
the only point where the image is in hand whatever the container.

#### Five criteria were measured, and all five fail

**[survey]** 10 of the 33 dumps carry an executable *and* a bundled
`Content/0000000000000000`. Exactly one of them is a "Mix" disc — and "Mix" has
exactly one row in the whole Iso2God table, so the class has one member in the
known universe, not merely in this sample.

| disc | Iso2God | A `disc n/N` | B module flags | C `OriginalPeName` | D `/Content` share | E XEX TitleID | E = a package's? |
|---|---|---|---|---|---|---|---|
| BF3 D1 | unlisted | 1/2 | `TITLE_MODULE` | `BF.Launcher_Xenon_Retail.exe` | 20.5 % | `45410950` | yes |
| BF4 D1 | No GOD | 1/2 | `TITLE_MODULE` | `BF.Launcher.exe` | 27.9 % | `454109BA` | yes |
| BioShock bonus | No GOD | 1/1 | `TITLE_MODULE` | `ExpansionInstaller.exe` | 98.3 % | `FFED2000` | no |
| BioShock 2 bonus | No GOD | 1/1 | `TITLE_MODULE` | `ExpansionInstaller.exe` | 98.7 % | `FFED2000` | no |
| Skyrim Legendary D2 | No GOD | 1/1 | `TITLE_MODULE` | `ExpansionInstaller.exe` | 96.9 % | `FFED2000` | no |
| Fallout NV Ultimate D2 | No GOD | 1/1 | `TITLE_MODULE` | `ExpansionInstaller.exe` | 97.7 % | `FFED2000` | no |
| Forza 3 D2 | No GOD | **2/2** | `TITLE_MODULE` | **`default.exe`** | 98.6 % | `4D53084D` | yes |
| GTA V D1 | No GOD | 1/2 | `TITLE_MODULE` | `game_xenon_final.exe` | 97.3 % | `545408A7` | yes |
| Halo 4 D2 | unlisted | **1/1** | `TITLE_MODULE` | `midnight_cache_release.exe` | **59.1 %** | `4D530919` | yes |
| **Splinter Cell D2** | **Mix** | **2/2** | `TITLE_MODULE` | **`default.exe`** | **40.9 %** | `555308B6` | yes |

- **A — disc number/count.** Fails. Forza 3 D2 is also `2/2` and wants no GOD.
  Halo 4 D2 is worse than useless: it reports `1/1` although it is physically the
  second disc of the set, so the field does not even describe the pressing.
- **B — `XexModuleFlags`.** Fails, and is eliminated for good: all ten discs are
  plain `TITLE_MODULE`. A bonus disc's executable is a normal title module.
- **C — `OriginalPeName`.** Fails *as a "Mix" detector*: Forza 3 D2 is named
  `default.exe`, exactly like Splinter Cell D2. It does identify the four
  "expansion installer" carriers perfectly — but so does E, semantically.
- **D — share of the volume under `/Content`.** Fails. Splinter Cell D2's 40.9 %
  sits *between* BF4 D1's 27.9 % and Halo 4 D2's 59.1 %, both of which must not
  get a GOD. No threshold exists.
- **E — does the disc's XEX claim the same TitleID as its packages?** Fails as a
  "Mix" detector: six of the ten say yes, only one is "Mix".

So the answer the first pass guessed is now measured: **there is no rule.** What
separates Splinter Cell: Blacklist D2 from Forza 3 D2 is not on the disc, it is
the editorial fact that one half of the campaign is bootable and the other's
bonus content is not.

#### What E *does* buy, and it is not nothing

**[survey]** E splits the bundled population cleanly in two, and the split is
semantic rather than numerical:

- **carriers (4)** — the XEX itself declares the `FFED2000` placeholder, and its
  module is literally named `ExpansionInstaller.exe`. `FFED2000` is therefore not
  just a folder name on these discs, it is what the executable claims to be.
  Nothing bootable is lost by never building their GOD.
- **real game discs that happen to carry content (6)** — BF3 D1, BF4 D1,
  Forza 3 D2, GTA V D1, Halo 4 D2, Splinter Cell D2. Each is a bootable half of
  its own game claiming its own TitleID, and for each of them we silently decline
  to build a GOD. Iso2God says that is right five times out of six.

That asymmetry is the actionable part: the loss is only ever possible in the
second group, and the app currently says nothing at all.

#### Decision: a quirks table, and nothing else

`MIX_DISCS` in `crates/core/src/iso_info.rs`, keyed by (TitleID, disc number),
holding the one known entry — `555308B6` disc 2. One line is not an
embarrassment: it is the entire known population. `inspect` now reads the disc's
executable before returning a bundled-content verdict (best-effort: an
unparseable XEX still installs as content, it just cannot be promoted), and a
match yields `IsoKind::GameWithBundledContent`, which `convert_into` treats by
running `install_game` then `install_bundled_content`.

**A warning for the six "the disc is itself a game" cases was considered and
dropped.** The software is a black box to the people using it: told that "this
disc is also a bootable game but no GOD was built", they have no way to know
whether that is normal, and nothing they can do about it. It would be noise with
a decision attached that only the project can make. Reports through GitHub issues
and Reddit are the better sensor, and new table entries are expected to come from
there rather than from any heuristic.

Always doing both stays rejected, now with numbers: `max_used_prefix_size()` puts
the nine discs that want no GOD between 3.4 and 7.9 GiB each — close to 50 GiB of
useless containers, plus a spurious extra disc entry under each game's TitleID.

### 2. Watch_Dogs — silently wrong, not just unsupported

Instruction: merge `installation1`/`installation2` from disc 1 into the
extracted disc 2, rebuild a ~10 GB ISO, convert that.

**[measured]** Disc 1 has `default.xex` + `installation1/` + `installation2/`
and **no** `Content` folder; disc 2 has `default.xex` and no `Content` folder
either. So both take the plain `IsoKind::Xbox360Game` path: we would happily
produce a GOD from the installation disc (useless) and a GOD from the game disc
(incomplete), with no warning. We can neither merge two images nor write an
XDVDFS ISO, so the operation itself stays out of scope — but the silence is
ours to fix (the `installation1`/`installation2` root pair is a recognisable
marker for Ubisoft install discs).

**[survey]** Confirmed: across the 33 dumps, the `installation1` +
`installation2` root pair appears on **Watch_Dogs D1 and nowhere else**. Both
discs report TitleID `555308B7` and classify as `Xbox360Game` (D1 `1/2`,
D2 `2/2`), so the two useless GODs were exactly what the app produced.

#### Solved without rebuilding an ISO

The blocker was taken at face value for too long: the instruction says *rebuild
an ISO*, but what that ISO would contain is a directory tree — and the app
already installs Xbox 360 games as extracted folders
(`Xbox360Format::Xex`, `DEFAULT_XEX_DIR`). Merging into a folder is something we
can do; writing XDVDFS is not, and is not needed.

Two quirks, in `crates/core/src/quirks.rs`:

- **D1 → `MergesFolderContents(["installation1", "installation2"])`.** Only those
  two subtrees are extracted, and the instruction's wording matters: it says
  *copy the **contents** of* those folders. They are a wrapper D1's installer
  would have unpacked, not a layout the game looks for, so
  `extract::extract_iso_subtree_contents` drops the folder level and writes what
  is inside straight into the game folder.

  **[measured]** They hold 281 files — `common.dat`/`.fat`,
  `shadersobj.dat`/`.fat`, `sound.dat`/`.fat`, `vidx/` (261 files) and `worlds/`
  (14) — which is plainly game data belonging beside D2's own `sound_*.dat`/`.fat`
  pairs. The two folders collide neither with each other nor with anything on D2
  (**zero** shared paths), so the merge overwrites nothing and the order the
  discs are added in stays irrelevant.

  They are 6.5 GiB of the 7.5 GiB the disc uses; the 0.17 GiB left is redundant
  with D2 — and includes a `default.xex` (`installer.exe`) that would overwrite
  D2's. The filter is about correctness, not disk space.
- **D2 → `ForceExtractedGame { entry_point: Some("game.xex") }`.** Extracted
  whatever format the target is configured for, because a GOD container cannot be
  completed by the other disc afterwards.

The folder name comes from the TitleID (`Watch Dogs [555308B7]`), which both
discs share, so **the order they are added in does not matter**. Adding D1 first
leaves a folder with no `default.xex`, which `game::detect_extracted_local`
declines to classify — the game simply is not listed until D2 arrives.

**[measured]** Both orders verified end to end with
`crates/core/examples/multi_disc_check.rs`: D1 then D2, and D2 then D1, converge
on the same 10.56 GiB folder. After D1 alone the library correctly lists nothing.

#### The entry point

**[measured]** D2 carries three root executables, and their XEX headers settle a
piece of forum lore:

| file | `OriginalPeName` | size |
|---|---|---|
| `default.xex` | `starter.exe` | 1.91 MiB |
| `game.xex` | **`default.exe`** | 12.97 MiB |
| `UplayBrowser.xex` | `UplayBrowser.exe` | 13.86 MiB |

(D1's `default.xex` is `installer.exe`, also 1.91 MiB — the same launcher shim in
its other role.)

`game.xex` was *built* as `default.exe`: it is the game's real entry point,
renamed at mastering so `starter.exe` could take the `default.xex` slot. The
launcher looks for an installation laid down by D1's installer and will not chain
from a hard drive, which is why forums say to run `game.xex`. So the quirk
restores it: `default.xex` is set aside as `default.original.xex` (inert — both
`game::detect_extracted_local` and Aurora match `default.xex` exactly) and
`game.xex` is *copied* over it, the original staying in place so nothing
referring to it by name can dangle. All three files share TitleID and MediaID, so
no identity changes.

Only the first swap sets the launcher aside, so re-adding the disc cannot
overwrite the backup with a copy of the game.

### 3. `ContentDisc` trusts folder names (latent, not hit by these dumps)

When an image has neither `default.xex` nor `default.xbe`, we take
`IsoKind::ContentDisc` and copy each `<TitleID>` folder **by name**, without
reading any STFS header. A `FFED2000/FFFFFFFF` tree would then be copied
verbatim and be unusable, plus it would add a bogus `FFED2000` entry to the
library. `FFED2000` appears nowhere in the codebase.

**[measured]** none of the 31 dumps reaches this path — every problematic disc
here carries an executable. The risk is limited to rebuilt, trimmed or
repacked images (and to folders prepared by hand). Cheap hardening: when a
`ContentDisc` folder name is a known placeholder, fall back to the STFS scan
the `BundledContent` path already uses, keeping the literal copy as the
default.

**[survey]** Still true on 33 dumps: a `FFED2000`/`FFFFFFFF` folder appears on
four discs (BioShock and BioShock 2 bonus, Skyrim Legendary D2, Fallout NV
Ultimate D2) and all four carry a `default.xex`, so all four take the
`BundledContent` path and never touch the folder name. New and useful for the
hardening, though: on those same four discs **the XEX declares TitleID
`FFED2000` too**. The placeholder is a value the format genuinely uses, not a
convention of one authoring tool — which makes recognising it in a folder name
a defensible check rather than a guess.

### 4. Console-side settings — nothing to do

"Disable fakelive" (CoD: World at War, Ultra Street Fighter IV) and "Run to
install, can be deleted afterwards" (Wolfenstein TNO D1) are Aurora/Dashlaunch
concerns. At most, a per-TitleID quirks note surfaced in the game details.

### 5. Assassin's Creed IV D2 — "Multiplayer disk", No GOD, no instruction

The disc is simply useless. If it carries a xex and no content, we would
convert it to a GOD without complaining — not destructive, just wasted space.

## Practical notes

- The bundled-content path extracts the **whole** disc to `work_dir` before
  filing the packages, so it needs as much free space as the ISO (8+ GB for
  GTA V D1) even though only a few GB are kept.
- **[measured]** `Battlefield 2 - Modern Combat (Europe) (En,Nl,Sv).zip` in the
  local set is corrupt: its local file header starts with four zero bytes
  instead of `PK\x03\x04`, and the file is 259 bytes longer than the central
  directory says. It is a bad download, not an app problem — our `zip` reader
  will reject it too.

## Priorities

1. ~~Gap 1 (Splinter Cell "Mix")~~ — **done**. The automatic rule was looked for
   and does not exist (see the criteria table); the fix is the one-entry
   `MIX_DISCS` table, deliberately without any user-facing warning.
2. ~~Gap 2 (Watch_Dogs)~~ — **done**, and better than the "detect and warn" this
   list first settled for: the set installs correctly, in either order, without
   rebuilding an ISO.
3. Gap 3 — cheap hardening, no dump in hand currently needs it.

## Reproducing the survey

```
cargo run --release -p txbm-core --example mix_disc_survey -- \
    <dump dir> --exclude Borderlands --jobs 4 --json survey.jsonl
```

`crates/core/examples/multi_disc_check.rs` is the companion for a set whose discs
only work together: it converts several images into the same target, in the order
given, and prints the library and the tree after each one (`--keep` to add to an
existing target rather than wipe it).

`crates/core/examples/mix_disc_survey.rs` takes ISOs and `.zip`/`.7z` archives,
unpacks each archive to a temporary directory it deletes right after, and reads
everything through `XdvdImage` and `stfs::inspect_reader`. It prints three
tables — every image, the population that carries both an executable and bundled
content, and the root markers that feed gaps 2 and 3 — and writes one JSON line
per image. The dump directory itself is only ever read. Roughly 20 minutes for
33 discs on four jobs, bound by inflate, not by disk.
