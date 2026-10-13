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

use crate::config::{ColorMode, Config, ImageMode, Theme};

pub const SPECIES: [u16; 9] = [133, 134, 135, 136, 196, 197, 470, 471, 700];
const SOURCE: &str = "https://github.com/PokeAPI/sprites";
const REVISION: &str = "0dc105d5b12416932f29b594078282ab4be045f5";
const ATTRIBUTION: &str = "Artwork distributed by PokeAPI/sprites; Pokémon artwork rights remain with The Pokémon Company, Nintendo, Game Freak, and Creatures. No affiliation or endorsement.";
// Source records from the local design preview; downloads must match these bytes.
const HASHES: [&str; 9] = [
    "33ea993d3478ee920062b0bf27792a045b3a2f75d941bdb8cb3912ef50b1fb44",
    "3f4a86ef2ec064c98dd8fb207046c7202c1669e5fb7fac05fc1acea364d2770f",
    "bf525c20b899e08398d93d0436d2e592463068d85da8760424ac340c7f8f8ed8",
    "a11d3f021baf917f9106266d5fec5b6326a2850b130e4560e333fa9f481c41ec",
    "d0e74b02adc3df07f9bf026303b1068c58183f8703ae2ca42b8d7b58e9a0696f",
    "8d7f18f7b323269dff84480f1ee8873a5d47a3eb3fd4b2ac1d2bbf0cf45a6ddd",
    "3cb145cfafe87da63f75d80d2540e2f7f61c3c97275ff4231a462d949f4bd339",
    "bc7c93a32e133f9e560b38722d0bc4992b7bb8d1cd902de984ccbd872a19f71b",
    "193777db6d89cdace234fe3198dca4e955a7a5a18011cd0105178b436f6199a7",
];
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

fn image_url(id: u16) -> String {
    format!(
        "https://raw.githubusercontent.com/PokeAPI/sprites/{REVISION}/sprites/pokemon/other/official-artwork/{id}.png"
    )
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
    cache: Vec<Raster>,
    color: ColorMode,
    ascii: bool,
    theme: Theme,
    tick: u64,
    generation: u64,
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
                decode_png(&bytes).map(trim_transparent_border)
            })();
            match result {
                Ok(image) => Asset {
                    image: Some(image),
                    status: if matches!(&sources, Ok(Some(_))) {
                        "Pinned PokeAPI artwork / SHA256 verified".into()
                    } else {
                        "Local PNG / source unverified (no sources.json)".into()
                    },
                },
                Err(error) => Asset {
                    image: None,
                    status: format!(
                        "Artwork unavailable: {error}. Open Images to download/retry, or supply {}.png with --assets-dir and reload.",
                        SPECIES[index]
                    ),
                },
            }
        });
        Self {
            assets,
            cache: Vec::new(),
            color: config.color,
            ascii: config.ascii,
            theme: config.theme,
            tick: 0,
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

    fn key(&self, index: usize, area: Rect, background: Color) -> CacheKey {
        CacheKey {
            species: index,
            width: area.width.min(MAX_CELL_WIDTH),
            height: area.height.min(MAX_CELL_HEIGHT),
            background: rgb(background, self.theme),
            color: self.color,
            ascii: self.ascii,
        }
    }

    /// Called before drawing, never by render. Bounded caches retain offline form
    /// switches and multiple thumbnail sizes without decoding or resizing frames.
    pub fn prepare(&mut self, targets: &[(usize, Rect)], background: Color) {
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

    /// Copies prepared cells only: no image IO, allocation, decode, or resizing.
    pub fn render(&self, index: usize, area: Rect, buffer: &mut Buffer) {
        if self.original(index).is_none() {
            let label = if self.status(index).starts_with("Artwork off") {
                "Images off"
            } else {
                "No image\nImages: setup"
            };
            Paragraph::new(label)
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

// Official images use a large transparent canvas. Fit the visible subject, not
// the empty border, so ears/tails remain visible without shrinking the portrait.
fn trim_transparent_border(image: RgbaImage) -> RgbaImage {
    let mut left = image.width();
    let mut top = image.height();
    let mut right = 0;
    let mut bottom = 0;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] != 0 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if right == 0 || (left == 0 && top == 0 && right == image.width() && bottom == image.height()) {
        return image;
    }
    image::imageops::crop_imm(&image, left, top, right - left, bottom - top).to_image()
}

fn fitted_dimensions(image: &RgbaImage, width: u16, rows: u16) -> (u16, u16) {
    let scale = (f64::from(width) / f64::from(image.width()))
        .min(f64::from(rows) * 2.0 / f64::from(image.height()));
    (
        (f64::from(image.width()) * scale).floor().max(1.0) as u16,
        (f64::from(image.height()) * scale).floor().max(1.0) as u16,
    )
}

fn rasterize(image: &RgbaImage, key: CacheKey, touched: u64) -> Raster {
    let (width, pixel_height) = fitted_dimensions(image, key.width, key.height);
    let height = pixel_height.div_ceil(2);
    let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
    for row in 0..height {
        for column in 0..width {
            let upper = sample(image, column, row * 2, width, pixel_height, key.background);
            let lower = if row * 2 + 1 < pixel_height {
                sample(
                    image,
                    column,
                    row * 2 + 1,
                    width,
                    pixel_height,
                    key.background,
                )
            } else {
                key.background
            };
            if key.ascii || key.color == ColorMode::Mono {
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
            } else {
                cells.push(RasterCell {
                    symbol: "▀",
                    foreground: output_color(upper, key.color),
                    background: output_color(lower, key.color),
                });
            }
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

// Interpolate composited samples rather than straight RGBA: transparent borders
// cannot introduce black fringes. Each half-cell has a square sample (cell 1:2).
fn sample(
    image: &RgbaImage,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    background: [u8; 3],
) -> [u8; 3] {
    if image.width() > u32::from(width) || image.height() > u32::from(height) {
        // A half-cell covers many source pixels at thumbnail sizes. Bilinear
        // point sampling discarded outlines and fine features; integrate the
        // complete footprint instead, compositing alpha before averaging.
        let left = f64::from(x) * f64::from(image.width()) / f64::from(width);
        let right = f64::from(x + 1) * f64::from(image.width()) / f64::from(width);
        let top = f64::from(y) * f64::from(image.height()) / f64::from(height);
        let bottom = f64::from(y + 1) * f64::from(image.height()) / f64::from(height);
        let mut total = [0.0; 3];
        for sy in top.floor() as u32..(bottom.ceil() as u32).min(image.height()) {
            let vertical = (bottom.min(f64::from(sy + 1)) - top.max(f64::from(sy))).max(0.0);
            for sx in left.floor() as u32..(right.ceil() as u32).min(image.width()) {
                let weight =
                    vertical * (right.min(f64::from(sx + 1)) - left.max(f64::from(sx))).max(0.0);
                let pixel = image.get_pixel(sx, sy);
                let alpha = f64::from(pixel[3]) / 255.0;
                for channel in 0..3 {
                    total[channel] += (f64::from(pixel[channel]) * alpha
                        + f64::from(background[channel]) * (1.0 - alpha))
                        * weight;
                }
            }
        }
        let area = (right - left) * (bottom - top);
        return total.map(|value| (value / area).round().clamp(0.0, 255.0) as u8);
    }
    let source_x = ((f64::from(x) + 0.5) * f64::from(image.width()) / f64::from(width) - 0.5)
        .clamp(0.0, f64::from(image.width() - 1));
    let source_y = ((f64::from(y) + 0.5) * f64::from(image.height()) / f64::from(height) - 0.5)
        .clamp(0.0, f64::from(image.height() - 1));
    let x0 = source_x.floor() as u32;
    let y0 = source_y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let dx = source_x.fract();
    let dy = source_y.fract();
    let samples = [
        (x0, y0, (1.0 - dx) * (1.0 - dy)),
        (x1, y0, dx * (1.0 - dy)),
        (x0, y1, (1.0 - dx) * dy),
        (x1, y1, dx * dy),
    ];
    std::array::from_fn(|channel| {
        samples
            .iter()
            .map(|&(sx, sy, weight)| {
                let pixel = image.get_pixel(sx, sy);
                let alpha = f64::from(pixel[3]) / 255.0;
                (f64::from(pixel[channel]) * alpha + f64::from(background[channel]) * (1.0 - alpha))
                    * weight
            })
            .sum::<f64>()
            .round()
            .clamp(0.0, 255.0) as u8
    })
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
    let mut complete = SPECIES.len() - missing.len();
    progress(format!(
        "Verified {complete}/9 cached images; existing files are preserved."
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
    for index in missing {
        check_cancelled(cancelled)?;
        let id = SPECIES[index];
        progress(format!(
            "Downloading {id}.png from PokeAPI; {complete}/9 ready…"
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
        complete += 1;
    }
    Ok(format!(
        "All nine pinned PNGs are SHA256-verified in {}. {ATTRIBUTION}",
        directory.display()
    ))
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
mod tests {
    use super::*;
    use crate::config::LayoutMode;
    use image::{ImageEncoder, Rgba, codecs::png::PngEncoder};
    use std::fs::File;

    #[cfg(unix)]
    #[test]
    fn named_pipe_assets_are_rejected_without_waiting_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let directory = TestDirectory::new();
        let pipe = directory.0.join("133.png");
        let path = std::ffi::CString::new(pipe.as_os_str().as_bytes()).unwrap();
        // SAFETY: the NUL-terminated path is live for the call and names an
        // isolated test fixture; mkfifo creates no open descriptors.
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(read_bounded(&pipe, MAX_FILE_BYTES).is_err());
        let artwork = Artwork::load(&config(&directory.0));
        assert!(artwork.original(0).is_none());
    }

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let root = std::env::temp_dir();
            for _ in 0..64 {
                let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = root.join(format!(
                    "eevee-artwork-test-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create test directory: {error}"),
                }
            }
            panic!("unable to reserve test directory");
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(path: &Path) -> Config {
        Config {
            ascii: false,
            theme: Theme::Dark,
            color: ColorMode::TrueColor,
            images: ImageMode::Cells,
            assets_dir: path.into(),
            layout: LayoutMode::Sanctuary,
            layout_explicit: false,
            data_dir: path.join("data"),
            storage_policy: crate::store::Policy::default(),
            context_exclusions: Vec::new(),
            capture_capacity: 64 * 1024,
        }
    }

    fn png(image: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut result = Vec::new();
        result.extend_from_slice(&(data.len() as u32).to_be_bytes());
        result.extend_from_slice(kind);
        result.extend_from_slice(data);
        let mut crc = u32::MAX;
        for &byte in kind.iter().chain(data) {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ if crc & 1 == 1 { 0xedb8_8320 } else { 0 };
            }
        }
        result.extend_from_slice(&(!crc).to_be_bytes());
        result
    }

    #[test]
    fn animated_png_is_rejected_instead_of_loading_a_first_frame() {
        let bytes = png(&RgbaImage::from_pixel(2, 2, Rgba([12, 34, 56, 255])));
        let mut animated = bytes[..33].to_vec(); // PNG signature and IHDR.
        animated.extend(chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
        animated.extend(chunk(
            b"fcTL",
            &[
                0, 0, 0, 0, // Sequence.
                0, 0, 0, 2, 0, 0, 0, 2, // Width and height.
                0, 0, 0, 0, 0, 0, 0, 0, // Offsets.
                0, 1, 0, 30, 0, 0, // Delay, disposal, blend.
            ],
        ));
        animated.extend_from_slice(&bytes[33..]);
        assert!(decode_png(&animated).unwrap_err().contains("animated PNG"));
    }

    fn key(
        color: ColorMode,
        ascii: bool,
        width: u16,
        height: u16,
        background: [u8; 3],
    ) -> CacheKey {
        CacheKey {
            species: 0,
            width,
            height,
            background,
            color,
            ascii,
        }
    }

    #[test]
    fn bounded_static_decode_and_hash_integrity() {
        let image = RgbaImage::from_pixel(2, 2, Rgba([12, 34, 56, 128]));
        let bytes = png(&image);
        assert_eq!(decode_png(&bytes).unwrap(), image);
        let hash = format!("{:x}", Sha256::digest(&bytes));
        assert!(verify_hash(&bytes, &hash).is_ok());
        assert!(verify_hash(&bytes, HASHES[0]).is_err());
        assert!(decode_png(b"not png").is_err());
        assert!(decode_png(&vec![0; MAX_FILE_BYTES as usize + 1]).is_err());
        let oversized = RgbaImage::from_pixel(MAX_DIMENSION + 1, 1, Rgba([0, 0, 0, 0]));
        assert!(decode_png(&png(&oversized)).is_err());
    }

    #[test]
    fn half_cells_preserve_aspect_alpha_and_original_colors() {
        let image = RgbaImage::from_pixel(8, 8, Rgba([220, 100, 40, 255]));
        assert_eq!(fitted_dimensions(&image, 10, 10), (10, 10));
        assert_eq!(fitted_dimensions(&image, 20, 5), (10, 10));
        let raster = rasterize(
            &image,
            key(ColorMode::TrueColor, false, 10, 10, [10, 20, 30]),
            0,
        );
        assert_eq!((raster.width, raster.height), (10, 5));
        assert_eq!(raster.cells[0].foreground, Color::Rgb(220, 100, 40));
        assert_eq!(raster.cells[0].background, Color::Rgb(220, 100, 40));
        let transparent = RgbaImage::from_pixel(1, 1, Rgba([200, 30, 20, 0]));
        assert_eq!(sample(&transparent, 0, 0, 1, 1, [10, 20, 30]), [10, 20, 30]);
        let partial = RgbaImage::from_pixel(1, 1, Rgba([210, 120, 30, 128]));
        assert_eq!(sample(&partial, 0, 0, 1, 1, [10, 20, 30]), [110, 70, 30]);
    }

    #[test]
    fn mono_ascii_and_indexed_rendering_have_real_boundaries() {
        let image = RgbaImage::from_pixel(2, 2, Rgba([255, 255, 255, 255]));
        for (mode, ascii) in [(ColorMode::Mono, false), (ColorMode::TrueColor, true)] {
            let raster = rasterize(&image, key(mode, ascii, 2, 1, [0; 3]), 0);
            assert!(raster.cells.iter().all(|cell| cell.symbol.is_ascii()
                && cell.foreground == Color::Reset
                && cell.background == Color::Reset));
            assert_eq!(raster.cells[0].symbol, "@");
        }
        let transparent = RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 0]));
        let raster = rasterize(&transparent, key(ColorMode::Mono, false, 2, 1, [255; 3]), 0);
        assert!(raster.cells.iter().all(|cell| cell.symbol == " "));
        assert_eq!(
            output_color([95, 135, 175], ColorMode::Indexed),
            Color::Indexed(67)
        );
    }

    #[test]
    fn ascii_art_preserves_the_backing_surface_instead_of_resetting_its_colors() {
        let directory = TestDirectory::new();
        let mut image = RgbaImage::from_pixel(2, 2, Rgba([255, 255, 255, 255]));
        image.put_pixel(0, 0, Rgba([255, 0, 0, 0]));
        fs::write(directory.0.join("133.png"), png(&image)).unwrap();
        let area = Rect::new(0, 0, 2, 1);
        let background = Color::Rgb(24, 33, 45);
        let foreground = Color::Rgb(232, 237, 244);
        for (mode, ascii) in [
            (ColorMode::Mono, false),
            (ColorMode::TrueColor, true),
            (ColorMode::Indexed, true),
        ] {
            let mut settings = config(&directory.0);
            settings.color = mode;
            settings.ascii = ascii;
            let mut artwork = Artwork::load(&settings);
            artwork.prepare(&[(0, area)], background);
            let mut buffer = Buffer::empty(area);
            for cell in &mut buffer.content {
                cell.set_bg(background).set_fg(foreground);
            }
            artwork.render(0, area, &mut buffer);
            assert_eq!(buffer[(0, 0)].symbol(), "=");
            assert_eq!(buffer[(1, 0)].symbol(), "@");
            assert!(
                buffer
                    .content
                    .iter()
                    .all(|cell| cell.bg == background && cell.fg == foreground)
            );
        }
    }

    #[test]
    fn local_missing_corrupt_and_off_states_are_honest() {
        let directory = TestDirectory::new();
        let mut settings = config(&directory.0);
        let artwork = Artwork::load(&settings);
        assert!(artwork.original(0).is_none());
        assert!(artwork.status(0).contains("--assets-dir"));
        fs::write(directory.0.join("133.png"), b"corrupt").unwrap();
        assert!(
            Artwork::load(&settings)
                .status(0)
                .contains("invalid or oversized PNG")
        );
        let bytes = png(&RgbaImage::from_pixel(2, 2, Rgba([12, 34, 56, 255])));
        fs::write(directory.0.join("133.png"), &bytes).unwrap();
        let mut artwork = Artwork::load(&settings);
        assert!(artwork.status(0).contains("source unverified"));
        artwork.prepare(&[(0, Rect::new(0, 0, 10, 10))], Color::Rgb(0, 0, 0));
        artwork.prepare(&[(0, Rect::new(0, 0, 10, 10))], Color::Rgb(0, 0, 0));
        assert_eq!(artwork.cache.len(), 1);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 10));
        artwork.render(0, Rect::new(0, 0, 10, 10), &mut buffer);
        assert_eq!(buffer[(0, 2)].symbol(), "▀");
        fs::write(directory.0.join("sources.json"), b"bad manifest").unwrap();
        assert!(
            Artwork::load(&settings)
                .status(0)
                .contains("corrupt source manifest")
        );
        settings.images = ImageMode::Off;
        let artwork = Artwork::load(&settings);
        assert!(artwork.original(0).is_none());
        assert!(artwork.status(0).contains("Artwork off"));
    }

    #[test]
    fn prepared_cache_is_bounded_and_theme_changes_have_separate_rasters() {
        let directory = TestDirectory::new();
        fs::write(
            directory.0.join("133.png"),
            png(&RgbaImage::from_pixel(4, 4, Rgba([12, 34, 56, 128]))),
        )
        .unwrap();
        let mut settings = config(&directory.0);
        settings.color = ColorMode::Mono;
        let mut artwork = Artwork::load(&settings);
        let area = Rect::new(0, 0, 4, 2);
        artwork.prepare(&[(0, area)], Color::Black);
        artwork.prepare(&[(0, area)], Color::White);
        assert_eq!(artwork.cache.len(), 2);
        for width in 129..=256 {
            artwork.prepare(&[(0, Rect::new(0, 0, width, 128))], Color::Black);
            assert!(artwork.cache.len() <= MAX_CACHE_ENTRIES);
            assert!(
                artwork
                    .cache
                    .iter()
                    .map(|raster| raster.cells.len())
                    .sum::<usize>()
                    <= MAX_CACHE_CELLS
            );
        }
    }

    #[test]
    fn pinned_manifest_identity_cannot_relabel_user_art_as_verified() {
        let mut source = manifest();
        assert!(validate_manifest(&source).is_ok());
        source.images[0].sha256 = "0".repeat(64);
        assert!(validate_manifest(&source).is_err());
        source = manifest();
        source.revision = "master".into();
        assert!(validate_manifest(&source).is_err());
        source = manifest();
        source.images[0].url = source.images[0].url.replace(REVISION, "master");
        assert!(validate_manifest(&source).is_err());
        source = manifest();
        source.images[1].id = source.images[0].id;
        assert!(validate_manifest(&source).is_err());
        let directory = TestDirectory::new();
        fs::write(
            directory.0.join("sources.json"),
            serde_json::to_vec(&manifest()).unwrap(),
        )
        .unwrap();
        fs::write(
            directory.0.join("133.png"),
            png(&RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]))),
        )
        .unwrap();
        assert!(
            Artwork::load(&config(&directory.0))
                .status(0)
                .contains("SHA256 differs")
        );
    }

    #[test]
    fn byte_limits_and_download_refuse_existing_user_assets_without_network() {
        let directory = TestDirectory::new();
        let path = directory.0.join("133.png");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(read_bounded(&path, MAX_FILE_BYTES).is_err());
        let error = download(&config(&directory.0)).unwrap_err();
        assert!(error.contains("was preserved"));
        assert_eq!(fs::metadata(path).unwrap().len(), MAX_FILE_BYTES + 1);
        let staged = directory.0.join("stage");
        write_staged(&staged, b"original").unwrap();
        assert!(write_staged(&staged, b"replacement").is_err());
        assert_eq!(fs::read(staged).unwrap(), b"original");
    }

    #[test]
    fn thumbnails_average_source_coverage_and_trim_only_empty_border() {
        let mut image = RgbaImage::new(16, 16);
        for y in 4..12 {
            for x in 2..14 {
                image.put_pixel(x, y, Rgba([240, 120, 60, 255]));
            }
        }
        let cropped = trim_transparent_border(image);
        assert_eq!(cropped.dimensions(), (12, 8));
        assert_eq!(cropped.get_pixel(0, 0), &Rgba([240, 120, 60, 255]));
        let mut narrow_line = RgbaImage::from_pixel(8, 8, Rgba([0, 0, 0, 255]));
        for y in 0..8 {
            narrow_line.put_pixel(0, y, Rgba([255, 255, 255, 255]));
        }
        // Point sampling the center lost this entire one-pixel feature.
        assert_eq!(sample(&narrow_line, 0, 0, 1, 1, [0; 3]), [32; 3]);
        let transparent = RgbaImage::new(4, 4);
        assert_eq!(trim_transparent_border(transparent).dimensions(), (4, 4));
    }

    #[test]
    fn every_missing_form_has_a_setup_placeholder_and_reload_generation() {
        let directory = TestDirectory::new();
        let settings = config(&directory.0);
        let artwork = Artwork::load(&settings);
        assert_eq!(artwork.available(), 0);
        assert_ne!(artwork.generation(), Artwork::load(&settings).generation());
        let area = Rect::new(0, 0, 20, 3);
        for index in 0..SPECIES.len() {
            let mut buffer = Buffer::empty(area);
            artwork.render(index, area, &mut buffer);
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains("Images: setup"));
        }
    }

    #[test]
    fn partial_manifest_is_resumable_and_conflicts_are_never_replaced() {
        let directory = TestDirectory::new();
        fs::write(
            directory.0.join("sources.json"),
            serde_json::to_vec(&manifest()).unwrap(),
        )
        .unwrap();
        assert!(load_manifest(&directory.0).unwrap().is_some());
        assert_eq!(
            missing_images(&directory.0).unwrap(),
            (0..9).collect::<Vec<_>>()
        );
        fs::write(directory.0.join("700.png"), b"keep this custom file").unwrap();
        let error = missing_images(&directory.0).unwrap_err();
        assert!(error.contains("700.png was preserved"));
        assert_eq!(
            fs::read(directory.0.join("700.png")).unwrap(),
            b"keep this custom file"
        );
        let cancelled = AtomicBool::new(true);
        assert!(
            download_with_progress(&config(&directory.0), &cancelled, |_| {})
                .unwrap_err()
                .contains("cancelled")
        );
    }

    #[test]
    fn worker_starts_idle_and_reloads_local_images_without_network() {
        let directory = TestDirectory::new();
        let worker = Worker::start(config(&directory.0)).unwrap();
        assert!(!worker.busy());
        assert!(worker.try_event().is_none());
        assert!(worker.try_request(Request::Reload));
        assert!(!worker.try_request(Request::Reload));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(Event::Finished { artwork, result }) = worker.try_event() {
                assert_eq!(artwork.available(), 0);
                assert!(result.unwrap().contains("no network"));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "local reload did not finish"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!worker.busy());
        assert!(!directory.0.join("sources.json").exists());
    }
}
