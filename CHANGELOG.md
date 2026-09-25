## [0.15.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.14.0...v0.15.0) (2026-09-22)

### Features

* **draft:** Quirk for Splinter Cell - Blacklist (disc 2) ([36acafe](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/36acafe5bcfca15cd4d0ea927fad430fc7f61a91))
* Quirks for Watch Dogs and Splinter Cell Blacklist (disc 2) ([c1e0068](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/c1e00685b1d491a9365cfce84d61c6d40963e018))
* Update / Backup / Restore the Xbox OG compatibility partition ([c5d9237](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/c5d9237d50907b241115f3421e29e1d7b84aea69))
* Xbox OG - Update XEFU config files ([653d512](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/653d512486da4e79624b3f6f6115c516539df317))

### Bug Fixes

* List GOD games whose title folder is not named after the TitleID. ([a7efa31](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/a7efa31315d4a4fb50d5a5a2c3e291c2b1fff486))
* UI missing padding ([43f87a5](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/43f87a5609e9ba4eb792f10b80772f60691e6b21))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`

## [0.14.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.13.0...v0.14.0) (2026-09-17)

### Features

* FATX help URL ([0457924](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/045792423c1799cb9dfa9e8653184fea1ae30afd))
* Implement directory sweeping to clean up temporary files on startup ([c37ba8c](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/c37ba8ce8767308c6183fb9ab57021a0acf79452))
* Improves FATX detection on Windows / MacOS (not tested yet); fixes some inconsistencies on game listing during upload / deletion ([a966cc3](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/a966cc32bb32ba554cd2c49bd1f6d75ebffef7d3))
* Refactor target handling to support FATX drives ([787d312](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/787d31207ef4f3dea02427f472632430596ded7d))

### Bug Fixes

* **ci:** pin clang-22/lld-22 install to llvm-toolchain-bullseye-22 ([5fc09bf](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/5fc09bf2027df80c9ef7c9fb2bf0023bbd76b081))
* code review fixes ([88f3b5a](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/88f3b5a169f7a5d7b50cd9ac514a2d7e3cd9b1db))
* Oupsy, link missing ([d3f4bed](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/d3f4bed73bf6d10b739c4254593303783668ab2b))
* Performances and other things! ([2cd2632](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/2cd2632fb83a797fa21f856ba186c9c6790733f2))
* Update fatx dependency to specific commit ([d9c861c](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/d9c861c18690f38124e4699ac1c77de55757baa7))
* Update target architecture names and dependencies in configuration files ([b7bbdcf](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/b7bbdcfec3aba1457268d2997d5b1a7717edbf59))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`

## [0.13.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.12.0...v0.13.0) (2026-08-29)

### Features

* Add ABadAvatar version selection with ABadAvatar v1.3-beta! ([9ab3575](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/9ab357598d74fd59ec2902d22a36cc9d92f17b37))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`

## [0.12.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.11.0...v0.12.0) (2026-08-20)

### Features

* Add games buttons on the queue page ([96a7c05](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/96a7c0563ace0748c63850533a4a38fb4654779b))
* Enhance conversion queue management by skipping duplicates and notifying users ([294a0ba](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/294a0ba86334040131a679fecf5234f59c92e8ca))
* Implement cover source selection for Xbox 360 covers with fallback options ([ba2fabe](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/ba2fabe2137664ed6296673902917933e018321a))
* Refactor conversion queue to job queue (to properly manage deletions) ([72f79dc](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/72f79dc7c5317ff294cc0a616bc4334be6268877))

### Bug Fixes

* Update navbar buttons ([ef172cf](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/ef172cfc375a2fa5f971355b1b9e5b15be996483))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`

## [0.11.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.10.0...v0.11.0) (2026-08-17)

### Features

* Add disabled state and tooltip support to ToolboxCard and BadAvatarCard ([6e4c729](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/6e4c72985b50a4ed3caa2f85e63a7ad16cc84e03))
* Add support for additional directory structure in local Aurora path resolution ([c36a6b5](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/c36a6b5796ca6030752be6dddeab14cbcfdfc36e))
* Add support for configurable GOD storage layouts ([d866cc3](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/d866cc3159df926eb988c14300403f98d0ebb336))
* Enhance conversion process with progress tracking and UI updates ([2875b5a](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/2875b5a04d382f82d3d813ebd2b401674e3f0b04))
* Enhance game deletion and storage configuration ([9c7c282](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/9c7c282c6b60601b0e57468844a53fe3d3fbf70d))
* Implement game overwrite warnings and enhance conversion queue management ([559902a](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/559902a91465c05403ea70531c19b7c820f338f9))
* Implement sorting functionality for game table and enhance UI components ([ce64613](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/ce64613dcc9108f94448dc15b40ffac6d48d4709))
* Refactor game content management and enhance game details display ([f6a09ab](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/f6a09abc36b57427ebcdb15955953f32455d2e10))

### Bug Fixes

* Allow storage config modal to shrink to content by removing minimum height constraint ([64f58e2](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/64f58e2bc9dcd964a9355fee70c07f77b8bc5358))
* Update stack size configuration for Windows targets in build scripts ([3bbc58b](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/3bbc58b41917fa39a436d2f62f5ff1ab02333465))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`

## [0.10.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.9.0...v0.10.0) (2026-08-01)

### Features

* Enhance conversion process and GUI ([566f954](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/566f9543185e06194fb9bd8f6a3beb445e34f108))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`

## [0.9.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.8.0...v0.9.0) (2026-07-27)

### Features

* Implement cancellation handling for conversion queue with user confirmation ([62fd3b6](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/62fd3b6728bf34826ecf01baf3c7d38acc6ea460))
* Implement cancellation handling for file uploads and extraction processes ([fc324a6](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/fc324a621bc4d2c75f98d151bc1c34bde65e9741))

### Bug Fixes

* Add tooltip support for CardActionRow and update BadAvatarCard behavior based on target connection ([fc814b7](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/fc814b79369ac3240d140f6e86f4735ec12ef10f))
* Enhance cancellation handling and user feedback during conversion processes ([e94bd92](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/e94bd923c4107b80cd163634aa786fad6fa33272))
* Implement case-insensitive file detection for extracted game directories (Xbox360 XEX) ([6e80efb](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/6e80efb4f03d003b2af173ff532953a5c0e8c7bb))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`

## [0.8.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.7.0...v0.8.0) (2026-07-24)

### Features

* Add support for displaying Aurora installation directory and enhance thumbnail caching mechanism ([8cbf183](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/8cbf183a6944ba458a470aa61effbb840866d5dc))
* Update target selection button behavior during BadAvatar USB key creation ([660c452](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/660c452949df69f1166e93ba20384bb18689bf55))

### Bug Fixes

* Improve thumbnail freshness check ([f586e8e](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/f586e8e5e114dd7d0f89dade54da889512fa910f))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`

## [0.7.0](https://github.com/jeanmatthieud/TinyXbox360BackupManager/compare/v0.6.1...v0.7.0) (2026-07-24)

### Features

* Add notification for clearing covers cache ([78e9947](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/78e9947fc18df866997d0c35b3bc9119a8675263))
* Enhance game info modal to display stored components with status and improved layout ([c6aaa48](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/c6aaa481f7a20c7b47c45a17021124de0422c8ec))
* Implement content deletion functionality for game components with confirmation modal ([bd44cb7](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/bd44cb7ee3120d7eb5489990e85365487e516be7))
* Implement thumbnail caching and ensure downscaled images for covers ([71921cd](https://github.com/jeanmatthieud/TinyXbox360BackupManager/commit/71921cd40da89dae38a09da275242e0f31543fb5))


---


<a href="https://github.com/sponsors/jeanmatthieud">
  <img src="https://img.shields.io/badge/GitHub%20Sponsors-Support-ea4aaa?logo=github-sponsors&logoColor=white" alt="GitHub Sponsors">
</a>
<a href="https://ko-fi.com/W6I723OON9">
  <img src="https://img.shields.io/badge/Ko--fi-Support%20Me-ff5e5b?logo=ko-fi&logoColor=white" alt="Ko-fi">
</a>


Between my freelance work and my two little daughters, free time is a rare commodity! I build this tool on my own time, with a lot of late-night coffee.

If TinyXbox360BackupManager saved you time or made you smile, **a donation** (much cheaper than a new game) **helps me keep maintaining and improving it**.

Thanks! 🎮

<hr />

> [!TIP]
> **:window: Windows installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-windows-x64.exe`

> [!TIP]
> **:penguin: Linux installation:**\
> Most users should download `TinyXbox360BackupManager-vX.X.X-linux-x86_64.AppImage`
> `chmod +x /path/to/AppImage` after downloading may be required

> [!IMPORTANT]
> **:apple: macOS installation:**\
> The app is not notarized, you must allow it manually after installing by running this command in Terminal:\
> `xattr -rd com.apple.quarantine /Applications/TinyXbox360BackupManager.app`
