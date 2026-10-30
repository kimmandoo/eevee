use std::fs::{self, OpenOptions};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

use image::{DynamicImage, ImageDecoder, Limits, RgbaImage, codecs::png::PngDecoder};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::Color,
    widgets::{Paragraph, Widget, Wrap},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    config::{ColorMode, Config, ImageMode, Theme},
    messages::{Message, Template},
};

pub const SPECIES: [u16; 9] = [133, 134, 135, 136, 196, 197, 470, 471, 700];
const SOURCE: &str = "https://github.com/PokeAPI/sprites";
const REVISION: &str = "0dc105d5b12416932f29b594078282ab4be045f5";
const ATTRIBUTION: &str = "Sprites distributed by PokeAPI/sprites; Pokémon artwork rights remain with The Pokémon Company, Nintendo, Game Freak, and Creatures. No affiliation or endorsement.";
// Original 96x96 front sprites at REVISION, not the detailed official artwork.
const HASHES: [&str; 9] = [
    "56ad4ec369aadb71218b080e6a67b85eea97b3cd63f9d3097d63d9b4b7483df9",
    "709755b8676dcabf09ccad8a98b2af918869f8a092b27ee2ac7d25cafdbfee89",
    "17a9215bd8276df8cdfc38709e956551b699188738bf628cab89a55bd384f85d",
    "80d64fb2e7ad492e68d4a7a3e48ef430275246c6e48da4b3678eebfd903b8c44",
    "445986a03a7fd22b72d1d675317e3ec550ae2d4b24c1394c3ef3bbac4df31056",
    "72df4ab24222b470e2d145c0c747b704a4abca4042d739e645a0e37ef7eeaf5c",
    "dc4699891713ce44a6dc4e433ba40075c081675947d9fdebcda37e662438c236",
    "54d2d8cdc5f0ed5d2f5da3c4006e4defa226bdd196e30dd4f54c2dd5321de266",
    "1ba3ba4d59fbe1e10ac86436cccf58931ec3d23646a496201738e2c6b9d76824",
];
/// Item sprites at the same pinned revision (`sprites/items/{name}.png`).
/// They live in `items/` under the asset cache, are always hash-verified and
/// are optional: a missing or different file shows the text icon instead.
pub const ITEMS: [(&str, &str); 11] = [
    (
        "potion",
        "db5a0fa2343c5761599d9efd4514a30125164ced094b55f760a763ed0c9c64d5",
    ),
    (
        "oran-berry",
        "4d4d31e7c29a4540d5f438b87bb1762bf3cd7b0920e523fc9020f6c0bace1c1a",
    ),
    (
        "tm-normal",
        "b349d5435a7b8d14c50c4f0b6e2bd9bb65b6b6d6dd6c44613e9255682702b5bc",
    ),
    (
        "tm-water",
        "d0512082244a4e5a1c99c150d7eec98c300135d93eec5e651b32a9f686ed25aa",
    ),
    (
        "tm-electric",
        "9836de38bf7f2de336293f91b58a7cd74d5573a36829e07e94450a60125c2bb7",
    ),
    (
        "tm-fire",
        "10c0559ce0777d405ed071d6288be4c4c7d378f52318c88dcc4d3054dbd73724",
    ),
    (
        "tm-psychic",
        "2389efbbf9db12a15d27ecae5178fe05fb364dc8e83e54416782861be9181e38",
    ),
    (
        "tm-dark",
        "6ffecdbf68d1d69870ab11038af3d7d477066f510dc21e6fc925ace9fd53c9e4",
    ),
    (
        "tm-grass",
        "accad0bdbdf1d9f7c7d72b7cc152872be6a5d10f6ec37f645ef9906d2e6f02fa",
    ),
    (
        "tm-ice",
        "53f48f29f6bdea329d44582e0d268eccf2facc762088dcd69d9c5902ccc9164a",
    ),
    (
        "tm-fairy",
        "13e6d2f3b387676f0c96a2118a7958fc7ebd25139ea775d409996a582a7f42aa",
    ),
];
/// Prepared item cell sizes: bag icon and notice.
pub const ITEM_SIZES: [(u16, u16); 2] = [(6, 3), (10, 5)];
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_DIMENSION: u32 = 1024;
const MAX_DECODE_ALLOC: u64 = 16 * 1024 * 1024;
const MAX_CELL_WIDTH: u16 = 256;
const MAX_CELL_HEIGHT: u16 = 128;
const MAX_CACHE_CELLS: usize = 128 * 1024;
const MAX_CACHE_ENTRIES: usize = 36;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static ARTWORK_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    source: String,
    revision: String,
    attribution: String,
    images: Vec<SourceImage>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceImage {
    id: u16,
    url: String,
    sha256: String,
}

fn item_url(name: &str) -> String {
    format!("https://raw.githubusercontent.com/PokeAPI/sprites/{REVISION}/sprites/items/{name}.png")
}

/// Index into [`ITEMS`] for a bag/notice item; key items have no sprite.
pub fn item_sprite(item: crate::app::game::Item) -> Option<usize> {
    use crate::app::game::Item;
    match item {
        Item::Potion => Some(0),
        Item::Berry => Some(1),
        Item::Tm(form) => Some(2 + form.min(8)),
        Item::Key => None,
    }
}

fn image_url(id: u16) -> String {
    format!("https://raw.githubusercontent.com/PokeAPI/sprites/{REVISION}/sprites/pokemon/{id}.png")
}

fn manifest() -> Manifest {
    Manifest {
        version: 1,
        source: SOURCE.into(),
        revision: REVISION.into(),
        attribution: ATTRIBUTION.into(),
        images: SPECIES
            .iter()
            .zip(HASHES)
            .map(|(&id, sha256)| SourceImage {
                id,
                url: image_url(id),
                sha256: sha256.into(),
            })
            .collect(),
    }
}

fn validate_manifest(value: &Manifest) -> Result<(), String> {
    if value.version != 1
        || value.source != SOURCE
        || value.revision != REVISION
        || value.attribution != ATTRIBUTION
        || value.images.len() != SPECIES.len()
    {
        return Err("source manifest does not match the pinned PokeAPI source; move sources.json aside and retry Images setup, or choose an empty --assets-dir (existing files were preserved)".into());
    }
    for (&id, hash) in SPECIES.iter().zip(HASHES) {
        let mut records = value.images.iter().filter(|record| record.id == id);
        let Some(record) = records.next() else {
            return Err(format!(
                "source manifest is missing species {id}; move sources.json aside and retry Images setup"
            ));
        };
        if records.next().is_some() || record.url != image_url(id) || record.sha256 != hash {
            return Err(format!(
                "source manifest has an invalid URL/hash or duplicate species {id}; move sources.json aside and retry Images setup (existing files were preserved)"
            ));
        }
    }
    Ok(())
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Reject FIFOs/devices based on the opened handle without waiting for
        // a pipe writer, including a path swapped between metadata and open.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(format!(
            "{}: requires a regular file of at most {maximum} bytes",
            path.display()
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() as u64 > maximum {
        return Err(format!(
            "{}: exceeds the {maximum}-byte limit",
            path.display()
        ));
    }
    Ok(bytes)
}

fn load_manifest(directory: &Path) -> Result<Option<Manifest>, String> {
    let path = directory.join("sources.json");
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
        Ok(_) => {}
    }
    let bytes = read_bounded(&path, MAX_MANIFEST_BYTES)?;
    let value = serde_json::from_slice(&bytes).map_err(|error| {
        format!("{}: corrupt source manifest: {error}. Move sources.json aside and retry Images setup; existing files were preserved.", path.display())
    })?;
    validate_manifest(&value)?;
    Ok(Some(value))
}

fn verify_hash(bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err("SHA256 differs from the pinned source; restore this PNG or use an unmanifested local directory".into());
    }
    Ok(())
}

fn decode_png(bytes: &[u8]) -> Result<RgbaImage, String> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("PNG exceeds {MAX_FILE_BYTES} bytes"));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let decoder = PngDecoder::with_limits(Cursor::new(bytes), limits)
        .map_err(|error| format!("invalid or oversized PNG (maximum {MAX_DIMENSION}x{MAX_DIMENSION}, {MAX_DECODE_ALLOC}-byte decode budget): {error}"))?;
    if decoder.is_apng().map_err(|error| error.to_string())? {
        return Err("animated PNG is unsupported; provide a static PNG".into());
    }
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || decoder.total_bytes() > MAX_DECODE_ALLOC {
        return Err("PNG dimensions or decoded allocation exceed the limit".into());
    }
    DynamicImage::from_decoder(decoder)
        .map(DynamicImage::into_rgba8)
        .map_err(|error| format!("corrupt PNG: {error}"))
}

struct Asset {
    image: Option<RgbaImage>,
    status: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CacheKey {
    species: usize,
    width: u16,
    height: u16,
    background: [u8; 3],
    /// The terminal's default background: transparent cells stay unpainted.
    transparent: bool,
    color: ColorMode,
    ascii: bool,
}

struct RasterCell {
    symbol: &'static str,
    foreground: Color,
    background: Color,
}

struct Raster {
    key: CacheKey,
    width: u16,
    height: u16,
    cells: Vec<RasterCell>,
    touched: u64,
}

pub struct Artwork {
    assets: [Asset; 9],
    /// Pre-rasterized item sprites per [`ITEM_SIZES`]; `None` shows text icons.
    items: Vec<[Option<Raster>; 2]>,
    cache: Vec<Raster>,
    color: ColorMode,
    ascii: bool,
    theme: Theme,
    tick: u64,
    generation: u64,
    cells_enabled: bool,
    /// Auto mode draws cells once the terminal is known to lack native graphics.
    auto: bool,
    cell_fallback: bool,
    native_active: bool,
}

impl Artwork {
    /// Reads only local files, once. Images-off never opens a manifest or PNG.
    pub fn load(config: &Config) -> Self {
        let sources = if config.images == ImageMode::Off {
            Ok(None)
        } else {
            load_manifest(&config.assets_dir)
        };
        let assets = std::array::from_fn(|index| {
            if config.images == ImageMode::Off {
                return Asset {
                    image: None,
                    status: "Artwork off (--images off)".into(),
                };
            }
            let path = config.assets_dir.join(format!("{}.png", SPECIES[index]));
            let result = (|| {
                let source = sources.as_ref().map_err(Clone::clone)?;
                let bytes = read_bounded(&path, MAX_FILE_BYTES)?;
                if source.is_some() {
                    verify_hash(&bytes, HASHES[index])?;
                }
                decode_png(&bytes)
            })();
            match result {
                Ok(image) => Asset {
                    image: Some(image),
                    status: if matches!(&sources, Ok(Some(_))) {
                        "Original PokeAPI sprite / SHA256 verified".into()
                    } else {
                        "Local PNG / source unverified (no sources.json)".into()
                    },
                },
                Err(error) => Asset {
                    image: None,
                    status: Template::ProvidersArtworkUnavailable
                        .format(&[("error", &error), ("species", &SPECIES[index].to_string())])
                        .unwrap_or_else(|error| format!("Message error: {error}")),
                },
            }
        });
        let mut assets: [Asset; 9] = assets;
        let mut images = assets.each_mut().map(|asset| asset.image.take());
        trim_shared_border(&mut images);
        for (asset, image) in assets.iter_mut().zip(images) {
            asset.image = image;
        }
        let background = if config.images == ImageMode::Off {
            None
        } else {
            Some(crate::ui::image_background(config))
        };
        let items = ITEMS
            .iter()
            .map(|(name, hash)| {
                let image = background.and_then(|_| {
                    let path = config.assets_dir.join("items").join(format!("{name}.png"));
                    let bytes = read_bounded(&path, MAX_FILE_BYTES).ok()?;
                    verify_hash(&bytes, hash).ok()?;
                    decode_png(&bytes).ok().map(trim_transparent_border)
                });
                ITEM_SIZES.map(|(width, height)| {
                    let (image, background) = (image.as_ref()?, background?);
                    let key = CacheKey {
                        species: usize::MAX,
                        width,
                        height,
                        background: rgb(background, config.theme),
                        transparent: background == Color::Reset,
                        color: config.color,
                        ascii: config.ascii,
                    };
                    Some(rasterize(image, key, 0))
                })
            })
            .collect();
        Self {
            assets,
            items,
            cache: Vec::new(),
            color: config.color,
            ascii: config.ascii,
            theme: config.theme,
            tick: 0,
            cells_enabled: config.images == ImageMode::Cells,
            auto: config.images == ImageMode::Auto,
            cell_fallback: false,
            native_active: false,
            generation: ARTWORK_GENERATION.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub fn status(&self, index: usize) -> &str {
        self.assets
            .get(index)
            .map_or("Unknown species", |asset| asset.status.as_str())
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn available(&self) -> usize {
        self.assets
            .iter()
            .filter(|asset| asset.image.is_some())
            .count()
    }

    /// The same bounded decoded source is available to the native renderer.
    pub fn original(&self, index: usize) -> Option<&RgbaImage> {
        self.assets
            .get(index)
            .and_then(|asset| asset.image.as_ref())
    }

    pub(crate) fn background_rgb(&self, background: Color) -> [u8; 3] {
        rgb(background, self.theme)
    }

    /// Native rendering owns loaded-image targets, including size-omission labels.
    pub fn set_native_active(&mut self, active: bool) {
        self.native_active = active;
    }

    /// Auto mode only: draw cell art because native graphics were not confirmed.
    pub fn set_cell_fallback(&mut self, active: bool) {
        self.cell_fallback = self.auto && active;
    }

    fn cells_active(&self) -> bool {
        self.cells_enabled || (self.cell_fallback && !self.native_active)
    }

    fn key(&self, index: usize, area: Rect, background: Color) -> CacheKey {
        CacheKey {
            species: index,
            width: area.width.min(MAX_CELL_WIDTH),
            height: area.height.min(MAX_CELL_HEIGHT),
            background: rgb(background, self.theme),
            transparent: background == Color::Reset,
            color: self.color,
            ascii: self.ascii,
        }
    }

    /// Called before drawing, never by render. Bounded caches retain offline form
    /// switches and multiple thumbnail sizes without decoding or resizing frames.
    pub fn prepare(&mut self, targets: &[(usize, Rect)], background: Color) {
        if !self.cells_active() {
            return;
        }
        self.tick = self.tick.wrapping_add(1);
        for &(index, area) in targets {
            let key = self.key(index, area, background);
            if key.width == 0 || key.height == 0 {
                continue;
            }
            if let Some(existing) = self.cache.iter_mut().find(|raster| raster.key == key) {
                existing.touched = self.tick;
                continue;
            }
            let Some(image) = self.original(index) else {
                continue;
            };
            let raster = rasterize(image, key, self.tick);
            while self.cache.len() >= MAX_CACHE_ENTRIES
                || self
                    .cache
                    .iter()
                    .map(|entry| entry.cells.len())
                    .sum::<usize>()
                    + raster.cells.len()
                    > MAX_CACHE_CELLS
            {
                let oldest = self
                    .cache
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, entry)| entry.touched)
                    .map(|(index, _)| index);
                if let Some(oldest) = oldest {
                    self.cache.swap_remove(oldest);
                } else {
                    break;
                }
            }
            self.cache.push(raster);
        }
    }

    /// Draws a cached item sprite of the closest prepared size. Returns false
    /// (drawing nothing) when no verified sprite is cached.
    pub fn render_item(
        &self,
        item: crate::app::game::Item,
        area: Rect,
        buffer: &mut Buffer,
    ) -> bool {
        let Some(sizes) = item_sprite(item).and_then(|index| self.items.get(index)) else {
            return false;
        };
        let Some(raster) = sizes
            .iter()
            .flatten()
            .filter(|raster| raster.width <= area.width && raster.height <= area.height)
            .max_by_key(|raster| raster.width)
        else {
            return false;
        };
        for row in 0..raster.height {
            for column in 0..raster.width {
                if let Some(cell) =
                    buffer.cell_mut((area.x.saturating_add(column), area.y.saturating_add(row)))
                {
                    let prepared = &raster.cells
                        [usize::from(row) * usize::from(raster.width) + usize::from(column)];
                    cell.set_symbol(prepared.symbol);
                    if !raster.key.ascii && raster.key.color != ColorMode::Mono {
                        cell.set_fg(prepared.foreground).set_bg(prepared.background);
                    }
                }
            }
        }
        true
    }

    /// Copies prepared cells only: no image IO, allocation, decode, or resizing.
    pub fn render(&self, index: usize, area: Rect, buffer: &mut Buffer) {
        if self.original(index).is_none() {
            let label = if self.status(index).starts_with("Artwork off") {
                Message::ProvidersArtworkImagesOff.text()
            } else {
                Message::ProvidersArtworkNoImageImagesSetup.text()
            };
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .render(area, buffer);
            return;
        }
        if !self.cells_active() {
            if self.native_active {
                return;
            }
            Paragraph::new(Message::ProvidersArtworkNoNativeImage.text())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .render(area, buffer);
            return;
        }
        let Some(raster) = self
            .cache
            .iter()
            .filter(|raster| {
                raster.key.species == index
                    && raster.key.width == area.width.min(MAX_CELL_WIDTH)
                    && raster.key.height == area.height.min(MAX_CELL_HEIGHT)
            })
            .max_by_key(|raster| raster.touched)
        else {
            return;
        };
        let x = area
            .x
            .saturating_add(area.width.saturating_sub(raster.width) / 2);
        let y = area
            .y
            .saturating_add(area.height.saturating_sub(raster.height) / 2);
        for row in 0..raster.height {
            for column in 0..raster.width {
                if let Some(cell) =
                    buffer.cell_mut((x.saturating_add(column), y.saturating_add(row)))
                {
                    let prepared = &raster.cells
                        [usize::from(row) * usize::from(raster.width) + usize::from(column)];
                    cell.set_symbol(prepared.symbol);
                    if !raster.key.ascii && raster.key.color != ColorMode::Mono {
                        cell.set_fg(prepared.foreground).set_bg(prepared.background);
                    }
                }
            }
        }
    }
}

/// Opaque pixel bounds as (left, top, right, bottom); None when fully transparent.
fn opaque_bounds(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] != 0 {
            bounds = Some(bounds.map_or((x, y, x + 1, y + 1), |(l, t, r, b)| {
                (l.min(x), t.min(y), r.max(x + 1), b.max(y + 1))
            }));
        }
    }
    bounds
}

fn crop_to(image: RgbaImage, (left, top, right, bottom): (u32, u32, u32, u32)) -> RgbaImage {
    if left == 0 && top == 0 && right == image.width() && bottom == image.height() {
        return image;
    }
    image::imageops::crop_imm(&image, left, top, right - left, bottom - top).to_image()
}

// Fit the visible sprite without its transparent 96x96 canvas, preserving every
// nontransparent pixel (including ears, tails, and partially transparent edges).
fn trim_transparent_border(image: RgbaImage) -> RgbaImage {
    match opaque_bounds(&image) {
        Some(bounds) => crop_to(image, bounds),
        None => image,
    }
}

// Same-canvas forms are cropped to the union of their opaque bounds, so every form
// shares one scale and baseline instead of each small sprite filling the frame.
// Mixed canvas sizes (user-supplied art) fall back to per-image trimming.
fn trim_shared_border(images: &mut [Option<RgbaImage>]) {
    let mut loaded = images.iter().flatten().map(RgbaImage::dimensions);
    let shared = loaded
        .next()
        .filter(|first| loaded.all(|size| size == *first))
        .and_then(|_| {
            images.iter().flatten().filter_map(opaque_bounds).reduce(
                |(l, t, r, b), (l2, t2, r2, b2)| (l.min(l2), t.min(t2), r.max(r2), b.max(b2)),
            )
        });
    for image in images.iter_mut() {
        *image = image.take().map(|image| match shared {
            Some(bounds) => crop_to(image, bounds),
            None => trim_transparent_border(image),
        });
    }
}

/// Fit the explicitly selected low-resolution cell preview; native transport
/// uses its own integer-only geometry and never shrinks source pixels.
fn fitted_pixels(image: &RgbaImage, width: u32, height: u32) -> (u32, u32) {
    let scale = (f64::from(width) / f64::from(image.width()))
        .min(f64::from(height) / f64::from(image.height()));
    let scale = if scale >= 1.0 { scale.floor() } else { scale };
    (
        (f64::from(image.width()) * scale).floor().max(1.0) as u32,
        (f64::from(image.height()) * scale).floor().max(1.0) as u32,
    )
}

fn fitted_dimensions(image: &RgbaImage, width: u16, rows: u16) -> (u16, u16) {
    let (width, height) = fitted_pixels(image, u32::from(width), u32::from(rows) * 2);
    (width as u16, height as u16)
}

fn rasterize(image: &RgbaImage, key: CacheKey, touched: u64) -> Raster {
    let (width, pixel_height) = fitted_dimensions(image, key.width, key.height);
    let height = pixel_height.div_ceil(2);
    // Color cells use quadrant blocks: two half-width samples per column double
    // horizontal detail. ASCII/mono keeps one luminance sample per half cell.
    let quadrants = !key.ascii && key.color != ColorMode::Mono;
    let columns = if quadrants { width * 2 } else { width };
    let pixel = |x: u16, y: u16| {
        if y < pixel_height {
            sample(image, x, y, columns, pixel_height, key.background)
        } else {
            (key.background, 0)
        }
    };
    let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
    for row in 0..height {
        for column in 0..width {
            if quadrants {
                let (x, y) = (column * 2, row * 2);
                let block = [
                    pixel(x, y),
                    pixel(x + 1, y),
                    pixel(x, y + 1),
                    pixel(x + 1, y + 1),
                ];
                let mask = block.iter().enumerate().fold(0, |mask, (bit, pixel)| {
                    mask | if pixel.1 != 0 { 1 << bit } else { 0 }
                });
                if key.transparent && mask != 15 {
                    // A transparent quadrant must remain terminal background,
                    // even when an opaque sample has exactly that same RGB.
                    // The remaining glyph color is the best original opaque
                    // sample; a cell cannot represent transparency plus two colors.
                    let foreground = block
                        .iter()
                        .filter(|pixel| pixel.1 != 0)
                        .min_by_key(|(candidate, _)| {
                            block
                                .iter()
                                .filter(|pixel| pixel.1 != 0)
                                .map(|(pixel, _)| {
                                    (0..3)
                                        .map(|channel| {
                                            (i32::from(candidate[channel])
                                                - i32::from(pixel[channel]))
                                            .pow(2)
                                        })
                                        .sum::<i32>()
                                })
                                .sum::<i32>()
                        })
                        .map_or(Color::Reset, |(color, _)| output_color(*color, key.color));
                    cells.push(RasterCell {
                        symbol: QUADRANTS[mask],
                        foreground,
                        background: Color::Reset,
                    });
                    continue;
                }
                let (symbol, foreground, background) = quadrant(block.map(|pixel| pixel.0));
                cells.push(RasterCell {
                    symbol,
                    foreground: output_color(foreground, key.color),
                    background: output_color(background, key.color),
                });
                continue;
            }
            let (upper, lower) = (pixel(column, row * 2).0, pixel(column, row * 2 + 1).0);
            let average = std::array::from_fn(|channel| {
                ((u16::from(upper[channel]) + u16::from(lower[channel])) / 2) as u8
            });
            let difference = (luminance(average) - luminance(key.background)).abs();
            const RAMP: [&str; 10] = [" ", ".", ":", "-", "=", "+", "*", "#", "%", "@"];
            let contrast = luminance(key.background)
                .max(255.0 - luminance(key.background))
                .max(1.0);
            let level = ((difference / contrast) * 9.0).round().clamp(0.0, 9.0) as usize;
            cells.push(RasterCell {
                symbol: RAMP[level],
                foreground: Color::Reset,
                background: Color::Reset,
            });
        }
    }
    Raster {
        key,
        width,
        height,
        cells,
        touched,
    }
}

/// Quadrant glyphs indexed by foreground bits: 1 upper-left, 2 upper-right,
/// 4 lower-left, 8 lower-right.
const QUADRANTS: [&str; 16] = [
    " ", "▘", "▝", "▀", "▖", "▌", "▞", "▛", "▗", "▚", "▐", "▜", "▄", "▙", "▟", "█",
];

/// A cell holds two colors, so pick the pair of the block's own colors that
/// fits its four samples best; no blended color is ever invented.
fn quadrant(block: [[u8; 3]; 4]) -> (&'static str, [u8; 3], [u8; 3]) {
    let distance = |a: [u8; 3], b: [u8; 3]| {
        (0..3)
            .map(|channel| (i32::from(a[channel]) - i32::from(b[channel])).pow(2))
            .sum::<i32>()
    };
    let mut best = (QUADRANTS[15], block[0], block[0], i32::MAX);
    for (index, &foreground) in block.iter().enumerate() {
        for &background in &block[index + 1..] {
            if foreground == background {
                continue;
            }
            let (mut mask, mut error) = (0, 0);
            for (bit, &pixel) in block.iter().enumerate() {
                let (near, far) = (distance(pixel, foreground), distance(pixel, background));
                if near <= far {
                    mask |= 1 << bit;
                    error += near;
                } else {
                    error += far;
                }
            }
            if error < best.3 {
                best = (QUADRANTS[mask], foreground, background, error);
            }
        }
    }
    (best.0, best.1, best.2)
}

// Nearest source pixels preserve the sprite's original palette and hard edges.
// Composite alpha only after selecting the pixel, without filtering transparent
// RGB into outlines, and retain the source alpha separately for cell coverage.
// Rows are half-cell samples; quadrant columns are half that width.
fn sample(
    image: &RgbaImage,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    background: [u8; 3],
) -> ([u8; 3], u8) {
    let source_x =
        ((u64::from(x) * 2 + 1) * u64::from(image.width()) / (u64::from(width) * 2)) as u32;
    let source_y =
        ((u64::from(y) * 2 + 1) * u64::from(image.height()) / (u64::from(height) * 2)) as u32;
    let pixel = image.get_pixel(
        source_x.min(image.width() - 1),
        source_y.min(image.height() - 1),
    );
    let alpha = u32::from(pixel[3]);
    let color = std::array::from_fn(|channel| {
        ((u32::from(pixel[channel]) * alpha + u32::from(background[channel]) * (255 - alpha) + 127)
            / 255) as u8
    });
    (color, pixel[3])
}

fn luminance(color: [u8; 3]) -> f64 {
    0.2126 * f64::from(color[0]) + 0.7152 * f64::from(color[1]) + 0.0722 * f64::from(color[2])
}

const ANSI: [[u8; 3]; 16] = [
    [0, 0, 0],
    [128, 0, 0],
    [0, 128, 0],
    [128, 128, 0],
    [0, 0, 128],
    [128, 0, 128],
    [0, 128, 128],
    [192, 192, 192],
    [128, 128, 128],
    [255, 0, 0],
    [0, 255, 0],
    [255, 255, 0],
    [0, 0, 255],
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 255],
];

fn indexed_rgb(index: u8) -> [u8; 3] {
    if index < 16 {
        return ANSI[usize::from(index)];
    }
    if index >= 232 {
        return [8 + (index - 232) * 10; 3];
    }
    let index = index - 16;
    let levels = [0, 95, 135, 175, 215, 255];
    [
        levels[usize::from(index / 36)],
        levels[usize::from(index / 6 % 6)],
        levels[usize::from(index % 6)],
    ]
}

fn rgb(color: Color, theme: Theme) -> [u8; 3] {
    match color {
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Indexed(index) => indexed_rgb(index),
        Color::Black => ANSI[0],
        Color::Red => ANSI[1],
        Color::Green => ANSI[2],
        Color::Yellow => ANSI[3],
        Color::Blue => ANSI[4],
        Color::Magenta => ANSI[5],
        Color::Cyan => ANSI[6],
        Color::Gray => ANSI[7],
        Color::DarkGray => ANSI[8],
        Color::LightRed => ANSI[9],
        Color::LightGreen => ANSI[10],
        Color::LightYellow => ANSI[11],
        Color::LightBlue => ANSI[12],
        Color::LightMagenta => ANSI[13],
        Color::LightCyan => ANSI[14],
        Color::White => ANSI[15],
        Color::Reset => {
            if theme == Theme::Light {
                [255; 3]
            } else {
                [0; 3]
            }
        }
    }
}

fn output_color(value: [u8; 3], mode: ColorMode) -> Color {
    if mode == ColorMode::TrueColor {
        return Color::Rgb(value[0], value[1], value[2]);
    }
    // The nearest cube entry is separable by channel; only that entry, the
    // nearest grayscale entry, and the 16 ANSI entries can minimize distance.
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let components: [u8; 3] = std::array::from_fn(|channel| {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, level)| (i16::from(value[channel]) - i16::from(**level)).abs())
            .map_or(0, |(index, _)| index as u8)
    });
    let cube = 16 + components[0] * 36 + components[1] * 6 + components[2];
    let mean = value.iter().map(|&channel| f64::from(channel)).sum::<f64>() / 3.0;
    let gray = 232 + ((mean - 8.0) / 10.0).round().clamp(0.0, 23.0) as u8;
    let index = (0..16u8)
        .chain([cube, gray])
        .min_by_key(|&index| {
            let candidate = indexed_rgb(index);
            value
                .iter()
                .zip(candidate)
                .map(|(&a, b)| (i32::from(a) - i32::from(b)).pow(2))
                .sum::<i32>()
        })
        .unwrap_or(0);
    Color::Indexed(index)
}

struct StagingDirectory(PathBuf);
impl Drop for StagingDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn staging_directory(directory: &Path) -> Result<StagingDirectory, String> {
    for _ in 0..64 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(".eevee-download-{}-{sequence}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(StagingDirectory(path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot create download staging directory: {error}")),
        }
    }
    Err("cannot reserve a unique download staging directory".into())
}

fn write_staged(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("{}: {error}", path.display()))
}

/// Explicit command only. Normal load/prepare/render cannot initiate networking.
/// Resume a partial pinned cache without replacing any existing file.
pub fn download(config: &Config) -> Result<String, String> {
    download_with_progress(config, &AtomicBool::new(false), |_| {})
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Acquire) {
        Err(
            "Image setup cancelled. Completed images were kept; retry resumes missing files."
                .into(),
        )
    } else {
        Ok(())
    }
}

fn missing_images(directory: &Path) -> Result<Vec<usize>, String> {
    let mut missing = Vec::with_capacity(SPECIES.len());
    // Inspect every destination before any network request. Never overwrite a
    // user's custom/corrupt image, even when a pinned manifest is present.
    for (index, id) in SPECIES.iter().enumerate() {
        let path = directory.join(format!("{id}.png"));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing.push(index),
            Err(error) => return Err(format!("{}: {error}", path.display())),
            Ok(metadata) => {
                let result = if metadata.file_type().is_symlink() {
                    Err("symbolic links are not download destinations".into())
                } else {
                    read_bounded(&path, MAX_FILE_BYTES).and_then(|bytes| {
                        verify_hash(&bytes, HASHES[index])?;
                        decode_png(&bytes).map(|_| ())
                    })
                };
                if let Err(error) = result {
                    return Err(format!(
                        "{} was preserved: {error}. Move this conflicting file aside and Retry, or choose an empty --assets-dir.",
                        path.display()
                    ));
                }
            }
        }
    }
    Ok(missing)
}

fn publish(directory: &Path, staging: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    let staged = staging.join(name);
    write_staged(&staged, bytes)?;
    let destination = directory.join(name);
    // Unlike rename, hard_link fails if another process created the destination.
    fs::hard_link(&staged, &destination).map_err(|error| {
        format!(
            "Cannot publish {} without overwriting: {error}. Existing files were preserved; reload or retry.",
            destination.display()
        )
    })
}

fn download_with_progress(
    config: &Config,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(String),
) -> Result<String, String> {
    check_cancelled(cancelled)?;
    let directory = &config.assets_dir;
    fs::create_dir_all(directory).map_err(|error| format!("{}: {error}", directory.display()))?;
    let has_manifest = load_manifest(directory)?.is_some();
    let missing = missing_images(directory)?;
    let initial_ready = SPECIES.len() - missing.len();
    progress(format!(
        "Verified {initial_ready}/9 cached images; existing files are preserved."
    ));
    let staging = staging_directory(directory)?;
    if !has_manifest {
        check_cancelled(cancelled)?;
        let bytes = serde_json::to_vec_pretty(&manifest()).map_err(|error| error.to_string())?;
        // Establish provenance before the first image, allowing safe resume after
        // a cancellation, failed download, or process exit.
        publish(directory, &staging.0, "sources.json", &bytes)?;
    }
    let agent = ureq::AgentBuilder::new()
        .https_only(true)
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build();
    for (done, index) in missing.iter().copied().enumerate() {
        check_cancelled(cancelled)?;
        let id = SPECIES[index];
        progress(format!(
            "Downloading {id}.png from PokeAPI; {}/9 ready…",
            initial_ready + done
        ));
        let url = image_url(id);
        let response = agent
            .get(&url)
            .call()
            .map_err(|error| format!("{url}: {error}. Retry resumes completed images."))?;
        check_cancelled(cancelled)?;
        if response.status() != 200 {
            return Err(format!(
                "{url}: expected HTTP 200, got {}",
                response.status()
            ));
        }
        if response
            .header("Content-Length")
            .and_then(|length| length.parse::<u64>().ok())
            .is_some_and(|length| length > MAX_FILE_BYTES)
        {
            return Err(format!("{url}: exceeds {MAX_FILE_BYTES}-byte limit"));
        }
        let mut bytes = Vec::new();
        let mut reader = response.into_reader().take(MAX_FILE_BYTES + 1);
        let mut chunk = [0; 16 * 1024];
        loop {
            check_cancelled(cancelled)?;
            let count = reader
                .read(&mut chunk)
                .map_err(|error| format!("{url}: {error}"))?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("{url}: exceeds {MAX_FILE_BYTES}-byte limit"));
        }
        verify_hash(&bytes, HASHES[index]).map_err(|error| format!("{url}: {error}"))?;
        decode_png(&bytes).map_err(|error| format!("{url}: {error}"))?;
        check_cancelled(cancelled)?;
        publish(directory, &staging.0, &format!("{id}.png"), &bytes)?;
    }
    // Item sprites are optional decoration: a failure keeps the species
    // result, reports what is missing, and text icons stand in.
    let items = download_items(directory, &agent, cancelled, &staging.0);
    Ok(format!(
        "All nine pinned PNGs are SHA256-verified in {}. {}{ATTRIBUTION}",
        directory.display(),
        match items {
            Ok(count) => format!("Item sprites: {count}/{} verified. ", ITEMS.len()),
            Err(error) => format!("Item sprites incomplete ({error}); text icons are used. "),
        }
    ))
}

fn download_items(
    directory: &Path,
    agent: &ureq::Agent,
    cancelled: &AtomicBool,
    staging: &Path,
) -> Result<usize, String> {
    let items = directory.join("items");
    fs::create_dir_all(&items).map_err(|error| format!("{}: {error}", items.display()))?;
    let mut ready = 0;
    for (name, hash) in ITEMS {
        check_cancelled(cancelled)?;
        let file = format!("{name}.png");
        let path = items.join(&file);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "{}: symbolic links are not download destinations",
                    path.display()
                ));
            }
            Ok(_) => {
                // Never overwrite: an existing file either verifies or is kept aside.
                if read_bounded(&path, MAX_FILE_BYTES)
                    .is_ok_and(|bytes| verify_hash(&bytes, hash).is_ok())
                {
                    ready += 1;
                }
                continue;
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {error}", path.display()));
            }
            Err(_) => {}
        }
        let url = item_url(name);
        let response = agent
            .get(&url)
            .call()
            .map_err(|error| format!("{url}: {error}"))?;
        if response.status() != 200 {
            return Err(format!(
                "{url}: expected HTTP 200, got {}",
                response.status()
            ));
        }
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("{url}: {error}"))?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("{url}: exceeds {MAX_FILE_BYTES}-byte limit"));
        }
        verify_hash(&bytes, hash).map_err(|error| format!("{url}: {error}"))?;
        decode_png(&bytes).map_err(|error| format!("{url}: {error}"))?;
        let staged = staging.join(&file);
        write_staged(&staged, &bytes)?;
        fs::hard_link(&staged, &path).map_err(|error| format!("{}: {error}", path.display()))?;
        ready += 1;
    }
    Ok(ready)
}

#[derive(Clone, Copy)]
pub enum Request {
    Download,
    Reload,
}

pub enum Event {
    Progress(String),
    Finished {
        artwork: Box<Artwork>,
        result: Result<String, String>,
    },
}

/// One explicit operation at a time. No network request is made on startup.
pub struct Worker {
    commands: SyncSender<Request>,
    events: Receiver<Event>,
    busy: AtomicBool,
    cancelled: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(config: Config) -> std::io::Result<Self> {
        let (commands, requests) = mpsc::sync_channel(1);
        // At most ten progress messages and one completion per request.
        let (results, events) = mpsc::sync_channel(16);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let worker_stopped = stopped.clone();
        thread::Builder::new()
            .name("eevee-artwork".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    if worker_stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let result = match request {
                        Request::Download => {
                            download_with_progress(&config, &worker_cancelled, |message| {
                                let _ = results.send(Event::Progress(message));
                            })
                        }
                        Request::Reload => Ok("Reloaded local artwork; no network request.".into()),
                    };
                    if worker_stopped.load(Ordering::Acquire) {
                        break;
                    }
                    // Reload on failure too: completed downloads remain useful.
                    let artwork = Box::new(Artwork::load(&config));
                    if results.send(Event::Finished { artwork, result }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            commands,
            events,
            busy: AtomicBool::new(false),
            cancelled,
            stopped,
        })
    }

    pub fn try_request(&self, request: Request) -> bool {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        self.cancelled.store(false, Ordering::Release);
        if self.commands.try_send(request).is_err() {
            self.busy.store(false, Ordering::Release);
            return false;
        }
        true
    }

    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// A current socket read may take up to its 30-second request timeout.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn try_event(&self) -> Option<Event> {
        let event = self.events.try_recv().ok()?;
        if matches!(&event, Event::Finished { .. }) {
            self.busy.store(false, Ordering::Release);
        }
        Some(event)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.cancel();
    }
}

#[cfg(test)]
mod tests;
