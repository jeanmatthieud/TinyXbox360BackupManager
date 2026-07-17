#!/bin/bash
# SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
# SPDX-FileContributor: Modified by Jean-Matthieu Dechriste (TinyXbox360BackupManager)
# SPDX-License-Identifier: GPL-3.0-only

TARGET_RESOLUTIONS=("16x16" "32x32" "48x48" "64x64" "128x128" "256x256" "512x512")
MAGICK_ARGS="-strip -colors 8 -dither None"

# Common
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 256x256 assets/TinyXbox360BackupManager-256x256.png
oxipng -sao6 assets/TinyXbox360BackupManager-256x256.png

# Linux
rm -rf package/linux/usr/share/icons
for res in "${TARGET_RESOLUTIONS[@]}"; do
  mkdir -p package/linux/usr/share/icons/hicolor/${res}/apps
  magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize ${res} package/linux/usr/share/icons/hicolor/${res}/apps/fr.dechriste.TinyXbox360BackupManager.png
  oxipng -sao6 package/linux/usr/share/icons/hicolor/${res}/apps/fr.dechriste.TinyXbox360BackupManager.png
done

# Windows
rm -f package/windows/icon.ico package/windows/TinyXbox360BackupManager-64x64.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -define icon:auto-resize=16,24,32,48,256 package/windows/icon.ico
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 64x64 package/windows/TinyXbox360BackupManager-64x64.png
oxipng -sao6 package/windows/TinyXbox360BackupManager-64x64.png

# macOS
rm -f package/macos/TinyXbox360BackupManager.icns
rm -rf package/macos/TinyXbox360BackupManager.iconset
mkdir package/macos/TinyXbox360BackupManager.iconset
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 16x16 package/macos/TinyXbox360BackupManager.iconset/icon_16x16.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_16x16.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 32x32 package/macos/TinyXbox360BackupManager.iconset/icon_16x16@2x.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_16x16@2x.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 64x64 package/macos/TinyXbox360BackupManager.iconset/icon_32x32@2x.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_32x32@2x.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 128x128 package/macos/TinyXbox360BackupManager.iconset/icon_128x128.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_128x128.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 256x256 package/macos/TinyXbox360BackupManager.iconset/icon_128x128@2x.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_128x128@2x.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 256x256 package/macos/TinyXbox360BackupManager.iconset/icon_256x256.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_256x256.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 512x512 package/macos/TinyXbox360BackupManager.iconset/icon_256x256@2x.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_256x256@2x.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 512x512 package/macos/TinyXbox360BackupManager.iconset/icon_512x512.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_512x512.png
magick assets/TinyXbox360BackupManager-1024x1024.png ${MAGICK_ARGS} -resize 1024x1024 package/macos/TinyXbox360BackupManager.iconset/icon_512x512@2x.png
oxipng -sao6 package/macos/TinyXbox360BackupManager.iconset/icon_512x512@2x.png
iconutil -c icns package/macos/TinyXbox360BackupManager.iconset
mv package/macos/TinyXbox360BackupManager.icns package/macos/App/Contents/Resources/
