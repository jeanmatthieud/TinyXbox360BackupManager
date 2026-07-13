// SPDX-FileCopyrightText: 2026 Manuel Quarneti <mq1@ik.me>
// SPDX-License-Identifier: GPL-3.0-only

use crate::{DisplayedHomebrewApp, DisplayedOscApp, util::MIB};
use image::ImageFormat;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer, ToSharedString};
use std::path::Path;
use twbm_core::homebrew::HomebrewApp;

const NO_IMAGE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="gray" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-image-off-icon lucide-image-off"><line x1="2" x2="22" y1="2" y2="22"/><path d="M10.41 10.41a2 2 0 1 1-2.83-2.83"/><line x1="13.5" x2="6" y1="13.5" y2="21"/><line x1="18" x2="21" y1="12" y2="15"/><path d="M3.59 3.59A1.99 1.99 0 0 0 3 5v14a2 2 0 0 0 2 2h14c.55 0 1.052-.22 1.41-.59"/><path d="M21 15V5a2 2 0 0 0-2-2H9"/></svg>"#;

fn no_icon() -> Image {
    Image::load_from_svg_data(NO_IMAGE).unwrap()
}

fn get_icon(app: &HomebrewApp) -> Option<Image> {
    let bytes = app.icon_bytes.as_ref()?;

    let image = image::load_from_memory_with_format(bytes, ImageFormat::Png).ok()?;
    let rgba8 = image.into_rgba8();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        rgba8.as_raw(),
        rgba8.width(),
        rgba8.height(),
    );

    Some(Image::from_rgba8(buffer))
}

impl DisplayedHomebrewApp {
    #[must_use]
    pub fn new(app: &HomebrewApp, osc_app: DisplayedOscApp) -> Self {
        let icon = get_icon(app).unwrap_or_else(no_icon);
        let slug = app.path.file_name().unwrap_or_default().to_string_lossy();

        Self {
            slug: slug.to_shared_string(),
            path: app.path.to_string_lossy().to_shared_string(),
            size_mib: app.size as f32 / MIB,
            icon,
            name: app.meta.name.to_shared_string(),
            coder: app.meta.coder.to_shared_string(),
            version: app.meta.version.to_shared_string(),
            release_date: app.meta.release_date.to_shared_string(),
            short_description: app.meta.short_description.to_shared_string(),
            long_description: app.meta.long_description.to_shared_string(),
            osc_app,
        }
    }
}

pub fn scan_drive(root_path: &Path) -> Vec<HomebrewApp> {
    let apps_dir = root_path.join("apps");
    twbm_core::homebrew::scan_dir(apps_dir).collect()
}
