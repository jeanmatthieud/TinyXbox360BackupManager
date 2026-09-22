# Original Xbox compatibility partition — research notes

Background for `crates/core/src/ogxbox_compat.rs` and the Toolbox card that
drives it. Everything here was measured on 2026-09-18 against the published
archives; it is recorded because it is expensive to re-derive and because two
of the findings look wrong until you see the evidence.

## The partition

The Xbox 360 runs original Xbox games through an emulator Microsoft called
*Xenon Fusion* (`xefu`). It lives on a partition of its own, at a fixed offset
with no partition table — `0x120EB0000`, 256 MiB, listed as `compat` in the
`fatx` crate's `X360_PARTITION_LAYOUT`. The console exposes it as **`HddX`**,
both to its own file browser and over Aurora's FTP server, and it holds exactly
one folder: `Compatibility`.

A typical populated partition:

```
Compatibility/
  xbox.xex            front-end: whitelist + per-title xefu mapping
  xefu.xex            the emulator itself (plus xefu1_1, 2, 3, 5, 6, 7, 7b…)
  xefutitle5.xex      externalised per-game settings tables (since xefu5)
  config.bin          written by the console itself — in no published pack
  dash/
    xboxdash.xbe      880-byte stub
    fonts/xbox.xtf    18 MiB
    fonts/Xbox Book.xtf  14 MiB
    xodash/xonlinedash.xbe
```

The fonts are what make a pack 38–42 MiB; everything else is under 5 MiB.

**The partition is only created when a drive is formatted at the Microsoft
factory.** A third-party drive, a reformatted one or one whose partition was
lost has no `HddX` at all. Nothing in the `fatx` crate can create one (there is
no `mkfs` of any kind), and nothing can over FTP either, so the tool detects the
absence and points at *HDD Compatibility Partition Fixer* (homebrew, run on the
console) or FATXplorer.

Not to be confused with `Hdd1\Compatibility\Xbox1\{TDATA,UDATA}` on the *content*
partition — that is the original Xbox's E drive, i.e. the **save games**.

## The two "retail" packs are not in conflict

Two sets circulate and look contradictory. SHA-1 settles it:

| | `xbox.xex` | `xefu.xex` | Rest |
|---|---|---|---|
| FATXplorer *Backwards Compatibility Files* | `1C577B74…` | `A97D65C4…` (base, 2005‑09‑14) | dash/fonts + the April 2018 title update |
| ConsoleMods *Unmodified Retail Xefu Pack* | `1C577B74…` (identical) | `87CD5F28…` (patched, 2005‑11‑16) | the eight official xefu + the `xefutitle*` |

`strings` on the title update in the FATXplorer archive
(`tu20075c00_00000000`, a `LIVE` package: content type `000B0000` at offset
`0x344`, title ID `FFFE07D2` at `0x360`, build 5832, April 2018) lists:

```
xbox.xexp  xefu.xexp
xefu1_1.xex  xefu2.xex  xefu3.xex  xefu5.xex  xefu6.xex  xefu7.xex  xefu7b.xex
xefutitle5.xex  xefutitle6.xex  xefutitle7.xex  xefutitle7b.xex
```

So the title update **is** the delivery vehicle for exactly what ConsoleMods
lays down directly on the partition, plus two `.xexp` patches that take the base
files to build 5832. FATXplorer distributes a *restoration kit* (bare 2005
partition + the update that catches it up); ConsoleMods distributes a *dump of a
factory partition*.

The tool therefore offers only the ConsoleMods packs: same result, one write,
one partition, no second download, no second session. What is given up is the
`.xexp` patch on `xbox.xex` — an older whitelist, irrelevant on a modded console
(the hacked packs drop the whitelist entirely) and re-delivered over Live on a
stock one. If it is ever wanted back, the update is at
`Hdd1:\Content\0000000000000000\FFFE07D2\000B0000\tu20075c00_00000000`, which is
the very path `crates/core/src/title_updates.rs` already manipulates.

A user's own drive, checked against the above, was byte-for-byte the FATXplorer
set without the title update — i.e. an emulator frozen in 2005.

## consolemods.org cannot be downloaded from

The wiki sits behind a Cloudflare managed challenge. Every non-browser client is
answered `403` with `cf-mitigated: challenge`, reproduced with curl over HTTP/1.1
and HTTP/2 and with python's urllib, in each case with a full browser
User-Agent, `Accept`/`Accept-Language` headers and a same-origin `Referer`. It
is a TLS/HTTP fingerprint decision, not a header one, so `ureq` fails the same
way — the browser-like agent in `crates/core/src/download.rs` does not help.

The URLs in `COMPAT_PACKS` are therefore pinned web.archive.org snapshots
(`/web/<timestamp>id_/<original url>`), which answer `200 application/zip`. That
is also an improvement in its own right: the wiki replaces these files in place
as they are updated, so an unpinned URL would silently change what the tool
installs.

The card's download button hands that same URL to the browser (`Message.OpenThat`)
rather than fetching it itself — a browser passes the challenge, and the user
gets to see where the pack actually comes from.

Sizes at the pinned snapshot, for reference:

| Pack | Bytes |
|---|---|
| `Unmodified_Retail_Xefu_Pack.zip` | 37 430 835 |
| `Hacked_Xefu_Pack.zip` | 40 614 646 |
| `Hacked_Xefu_Pack_with_HUD.zip` | 40 605 241 |

The *HDD Compatibility Partition Fixer* homebrew, referenced in the error
message when `HddX` is missing, is at
`consolemods.org/wiki/images/b/b2/Hdd_compat_partition_fixer_v1.zip` and is
reachable the same way.

## Archive shapes differ

The three packs do not wrap their payload the same way:

```
Unmodified_Retail_Xefu_Pack/Compatibility/…      wrapper folder
Hacked_Xefu_Pack/Compatibility/…                 wrapper folder
Compatibility/…                                  no wrapper
```

Hence `download::find_entry(root, "Compatibility", true)` rather than a fixed
path. The same helper handles the archives BadAvatar downloads, for the same
reason.

## The `fatx` crate misplaced this partition's cluster area

Found on 2026-09-18 on a real drive (`/dev/sda`): the tool reported the
partition as empty while it held a full *Hacked XeFu Pack*, and `space()` on the
same session reported 206 MiB in use. Both readings came from the same library,
so the FAT was being read correctly and the directory was not.

`libfatx` sizes the FAT from the cluster count **plus the reserved entry**, then
widens and page-aligns it (`libfatx/fatx.c:118-148`):

```c
fs->fat_size  = fs->partition_size / fs->bytes_per_cluster;
fs->fat_size += FATX_FAT_RESERVED_ENTRIES_COUNT;   /* ← before widening */
if (fs->fat_size < 0xfff0) { fs->fat_type = 16; fs->fat_size *= 2; } else …
if (fs->fat_size % 4096) fs->fat_size += 4096 - fs->fat_size % 4096;
fs->cluster_offset = fs->fat_offset + fs->fat_size;
```

The Rust port dropped that line, adding the reserved entry only afterwards to
`num_clusters` (`rust/fatx/src/fs.rs`, `num_fat_entries`). The page rounding
normally swallows the difference, which is why nothing else ever failed — but
the compatibility partition is 256 MiB of 16 KiB clusters, i.e. **16384 entries
× 2 bytes = exactly eight pages**, so there the missing entry pushes the FAT
into a ninth page and the whole cluster area moves by 4096 bytes. The reader
then looks for the root directory one page early, finds zeroes, and reports an
empty partition. On the same drive `sysext2` (`0x08000000`, likewise exactly on
a page boundary) was misread the same way; `sysext` and `data` were not, their
rounding absorbing the entry.

Writing through the same library would have put the pack a page away from the
real data — i.e. corrupted the partition. Only the read-only inspection step
stood between the bug and that.

The fix is one line in the fork, and `ogxbox_compat_check` now pins it: the
example plants a directory entry at the offset `libfatx` computes and asserts
the reader finds it, so building the test image with the reader's own
arithmetic can no longer hide the problem.

## A backup is a pack

The backup writes a zip whose entries keep the `Compatibility/…` prefix the
published packs use. That is deliberate: `stage_pack` finds the folder it needs
with `download::find_entry(root, "Compatibility", true)`, so an archive made
here is already a valid input for it. Restoring a backup is therefore the same
install path pointed at a local file, not a second implementation — which is
what the eventual restore feature should do rather than inventing its own
format. `ogxbox_compat_check` asserts this round trip.

The restore entry point is stricter than `stage_pack` on purpose: it requires
`Compatibility/` **at the archive's root** (`zip_holds_compatibility`, which reads
the central directory only). A published pack may bury the folder under a
release-named wrapper and is hunted down with `find_entry`; a file the user
points at by hand is worth refusing in the file dialog rather than fifty
megabytes later.

## Prior art

[XeCLI](https://saveeditors.github.io/xecli/wiki/Original-Xbox-Compatibility.html)
(`rgh ogxbox install hacked|hud|retail`) does the same job over FTP, to the same
`/HddX/Compatibility`, from the same three packs. Useful as a cross-check on
behaviour; it also stages the partition fixer, which this tool does not.
