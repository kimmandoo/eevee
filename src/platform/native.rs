use std::io::{self, Write};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Alignment, Rect, Size},
    style::Color,
    widgets::{Paragraph, Widget, Wrap},
};
use ratatui_image::{
    Image,
    protocol::{Protocol, sixel::Sixel},
};

use crate::{
    config::{ColorMode, Config, ImageMode},
    platform::input::GraphicsCapabilities,
    ui::artwork::Artwork,
};

// Private IDs, never delete another application's Kitty images.
const IMAGE_BASE: u32 = 0xEE00_0000;
const MAX_PIXELS: u32 = 2048;
static ALLOCATED: AtomicU16 = AtomicU16::new(0);
static SIXEL_ACTIVE: AtomicBool = AtomicBool::new(false);

fn image_id(index: usize) -> u32 {
    IMAGE_BASE | ((std::process::id() & 0xFFFF) << 4) | index as u32
}

fn delete(index: usize, free: bool, output: &mut impl Write) -> io::Result<()> {
    write!(
        output,
        "\x1b_Ga=d,d={},i={},q=2;\x1b\\",
        if free { 'I' } else { 'i' },
        image_id(index)
    )
}

pub fn cleanup() -> io::Result<()> {
    let allocated = ALLOCATED.swap(0, Ordering::SeqCst);
    let sixel = SIXEL_ACTIVE.swap(false, Ordering::SeqCst);
    if allocated == 0 && !sixel {
        return Ok(());
    }
    let mut output = io::stdout().lock();
    let mut failure = None;
    for index in 0..9 {
        if allocated & (1 << index) != 0
            && let Err(error) = delete(index, true, &mut output)
        {
            failure.get_or_insert(error);
        }
    }
    // Sixel has no image IDs. ED2 is required, not painting spaces or ECH:
    // terminals may preserve graphics when ordinary text cells are overwritten.
    // The restoration hook calls this while still in our alternate screen.
    if sixel && let Err(error) = output.write_all(b"\x1b[2J") {
        failure.get_or_insert(error);
    }
    if let Err(error) = output.flush() {
        failure.get_or_insert(error);
    }
    failure.map_or(Ok(()), Err)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Kitty,
    Sixel,
}

enum Encoded {
    Kitty { data: String, transmitted: bool },
    Sixel(Protocol),
}

struct Cached {
    target: Rect,
    cell: (u16, u16),
    background: [u8; 3],
    generation: u64,
    kind: Kind,
    size: Size,
    encoded: Encoded,
}

pub struct NativeGraphics {
    eligible: bool,
    kind: Option<Kind>,
    cell: Option<(u16, u16)>,
    cache: [Option<Cached>; 9],
    visible: u16,
    omitted: [Option<Rect>; 9],
    status: String,
}

impl NativeGraphics {
    pub fn new(config: &Config) -> Self {
        let eligible =
            config.images == ImageMode::Auto && config.color != ColorMode::Mono && !config.ascii;
        let status = match config.images {
            ImageMode::Off => "Images off; no graphics discovery",
            ImageMode::Cells => "Low-resolution cell preview (explicit --images cells)",
            ImageMode::Auto if !eligible => "Cell art: ASCII/monochrome mode",
            ImageMode::Auto => "Cell art: native graphics not confirmed",
        };
        Self {
            eligible,
            kind: None,
            cell: None,
            cache: std::array::from_fn(|_| None),
            visible: 0,
            omitted: [None; 9],
            status: status.into(),
        }
    }

    pub fn apply_capabilities(&mut self, capabilities: GraphicsCapabilities) {
        if !self.eligible {
            return;
        }
        self.kind = if capabilities.kitty {
            Some(Kind::Kitty)
        } else if capabilities.sixel {
            Some(Kind::Sixel)
        } else {
            None
        };
        self.cell = capabilities
            .cell_size
            .filter(|&(width, height)| width > 0 && height > 0);
        self.status = match (self.kind, self.cell) {
            (Some(kind), Some((width, height))) => format!(
                "Native {}: negotiated / {width}x{height} pixels per cell / integer scaling",
                if kind == Kind::Kitty {
                    "Kitty"
                } else {
                    "Sixel"
                }
            ),
            (None, _) => "Cell art: terminal did not confirm Kitty or Sixel support".into(),
            (_, None) => "Cell art: terminal supplied no usable pixel/cell dimensions".into(),
        };
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn is_native(&self) -> bool {
        self.kind.is_some() && self.cell.is_some()
    }

    /// ED2 and terminal resets may evict Kitty image data, not just placements.
    /// Keep encoded pixels but renew terminal residency before the next paint.
    pub fn terminal_cleared(&mut self) {
        for cached in self.cache.iter_mut().flatten() {
            if let Encoded::Kitty { transmitted, .. } = &mut cached.encoded {
                *transmitted = false;
            }
        }
    }

    /// Returns true when the caller MUST clear the terminal AND reset its draw
    /// buffer before drawing. Sixel cannot reliably be erased using cell diffs.
    /// Caches survive hiding and reappearing; no image is encoded on idle frames.
    pub fn prepare(
        &mut self,
        artwork: &Artwork,
        targets: &[(usize, Rect)],
        background: Color,
    ) -> io::Result<bool> {
        let mut output = io::stdout().lock();
        self.prepare_to(artwork, targets, background, &mut output)
    }

    fn prepare_to(
        &mut self,
        artwork: &Artwork,
        targets: &[(usize, Rect)],
        background: Color,
        output: &mut impl Write,
    ) -> io::Result<bool> {
        let background = artwork.background_rgb(background);
        let mut next_visible = 0u16;
        let mut clear_screen = false;
        self.omitted = [None; 9];
        if let (Some(kind), Some(cell)) = (self.kind, self.cell) {
            for &(index, target) in targets {
                if index >= 9 || target.is_empty() {
                    continue;
                }
                let Some(source) = artwork.original(index) else {
                    continue;
                };
                let Some((width, height, size)) = native_geometry(source, target, cell) else {
                    self.omitted[index] = Some(target);
                    continue;
                };
                let regenerate = self.cache[index].as_ref().is_none_or(|cache| {
                    cache.target.width != target.width
                        || cache.target.height != target.height
                        || cache.cell != cell
                        || cache.background != background
                        || cache.kind != kind
                        || cache.generation != artwork.generation()
                });
                if regenerate {
                    if let Some(old) = self.cache[index].as_ref() {
                        if old.kind == Kind::Kitty {
                            delete(index, true, output)?;
                        } else if self.visible & (1 << index) != 0 {
                            clear_screen = true;
                        }
                    }
                    // Kitty keeps real alpha, so the sprite sits on the
                    // terminal's own background; Sixel needs an opaque canvas.
                    let image = native_pixels(
                        source,
                        width,
                        height,
                        size,
                        cell,
                        (kind != Kind::Kitty).then_some(background),
                    );
                    let encoded = match kind {
                        Kind::Kitty => Encoded::Kitty {
                            data: encode_kitty(&image, image_id(index)),
                            transmitted: false,
                        },
                        Kind::Sixel => {
                            let mut sixel =
                                Sixel::new(DynamicImage::ImageRgba8(image), size, false)
                                    .map_err(io::Error::other)?;
                            // Explicit square-pixel raster dimensions also bound the final
                            // partial sixel band. icy_sixel 0.5 omits this raster header.
                            let start = sixel
                                .data
                                .find("\x1bP")
                                .ok_or_else(|| io::Error::other("Sixel encoder omitted DCS"))?;
                            let header = sixel.data[start..].find('q').ok_or_else(|| {
                                io::Error::other("Sixel encoder omitted raster introducer")
                            })?;
                            sixel.data.insert_str(
                                start + header + 1,
                                &format!(
                                    "\"1;1;{};{}",
                                    u32::from(size.width) * u32::from(cell.0),
                                    u32::from(size.height) * u32::from(cell.1),
                                ),
                            );
                            // Restore the backend's expected cursor after the escaped cell.
                            sixel.data = format!("\x1b[s{}\x1b[u\x1b[{}C", sixel.data, size.width);
                            Encoded::Sixel(Protocol::Sixel(sixel))
                        }
                    };
                    self.cache[index] = Some(Cached {
                        target,
                        cell,
                        background,
                        generation: artwork.generation(),
                        kind,
                        size,
                        encoded,
                    });
                    if kind == Kind::Kitty {
                        ALLOCATED.fetch_or(1 << index, Ordering::SeqCst);
                    }
                } else {
                    let cache = self.cache[index].as_mut().expect("prepared protocol");
                    let moved = cache.target != target;
                    if moved && self.visible & (1 << index) != 0 {
                        if kind == Kind::Kitty {
                            delete(index, false, output)?;
                        } else {
                            clear_screen = true;
                        }
                    }
                    cache.target = target;
                }
                next_visible |= 1 << index;
                if kind == Kind::Sixel {
                    SIXEL_ACTIVE.store(true, Ordering::SeqCst);
                }
            }
        }
        for index in 0..9 {
            if self.visible & !next_visible & (1 << index) != 0 {
                if self.cache[index]
                    .as_ref()
                    .is_some_and(|cache| cache.kind == Kind::Sixel)
                {
                    clear_screen = true;
                } else {
                    delete(index, false, output)?;
                }
            }
        }
        self.visible = next_visible;
        output.flush()?;
        Ok(clear_screen)
    }

    pub fn render(&self, buffer: &mut Buffer) {
        for (index, cached) in self.cache.iter().enumerate() {
            if let Some(target) = self.omitted[index] {
                Paragraph::new("Image needs more space (1x minimum)")
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true })
                    .render(target, buffer);
            }
            if self.visible & (1 << index) == 0 {
                continue;
            }
            let cached = cached.as_ref().expect("visible image prepared");
            let Encoded::Sixel(protocol) = &cached.encoded else {
                continue;
            };
            let size = cached.size;
            let area = Rect::new(
                cached.target.x + (cached.target.width - size.width) / 2,
                cached.target.y + (cached.target.height - size.height) / 2,
                size.width,
                size.height,
            );
            render_protocol(protocol, area, buffer);
        }
    }

    /// Paint ordinary Kitty placements after ratatui's text frame. Capability
    /// discovery proves basic Kitty transport, not Unicode placeholder support.
    pub fn paint(&mut self) -> io::Result<()> {
        let mut output = io::stdout().lock();
        self.paint_to(&mut output)
    }

    fn paint_to(&mut self, output: &mut impl Write) -> io::Result<()> {
        for (index, cached) in self.cache.iter_mut().enumerate() {
            if self.visible & (1 << index) == 0 {
                continue;
            }
            let cached = cached.as_mut().expect("visible image prepared");
            let Encoded::Kitty { data, transmitted } = &mut cached.encoded else {
                continue;
            };
            if !*transmitted {
                output.write_all(data.as_bytes())?;
                *transmitted = true;
            }
            let x = cached.target.x + (cached.target.width - cached.size.width) / 2;
            let y = cached.target.y + (cached.target.height - cached.size.height) / 2;
            // Fixed placement ID replaces the previous placement, never accumulates.
            // C=1 leaves the cursor unmoved; save/restore preserves the editor cursor.
            write!(
                output,
                "\x1b7\x1b[{};{}H\x1b_Ga=p,i={},p=1,c={},r={},C=1,z=1,q=2;\x1b\\\x1b8",
                u32::from(y) + 1,
                u32::from(x) + 1,
                image_id(index),
                cached.size.width,
                cached.size.height,
            )?;
        }
        output.flush()
    }
}

fn encode_kitty(image: &RgbaImage, id: u32) -> String {
    use std::fmt::Write as _;
    const CHUNK_BYTES: usize = 3072; // 4096 base64 bytes, Kitty's maximum chunk.
    let bytes = image.as_raw();
    let mut data =
        String::with_capacity(bytes.len().div_ceil(3) * 4 + bytes.len().div_ceil(CHUNK_BYTES) * 64);
    for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
        data.push_str("\x1b_G");
        if index == 0 {
            write!(
                data,
                "a=t,f=32,t=d,i={id},s={},v={},",
                image.width(),
                image.height()
            )
            .expect("write String");
        }
        let more = usize::from((index + 1) * CHUNK_BYTES < bytes.len());
        write!(data, "m={more},q=2;").expect("write String");
        base64_simd::STANDARD.encode_append(chunk, &mut data);
        data.push_str("\x1b\\");
    }
    data
}

// Geometry is in real terminal pixels. Never downsample to cell dimensions or
// fractionally resize; a too-small target has no native picture, not a mosaic.
fn native_geometry(source: &RgbaImage, target: Rect, cell: (u16, u16)) -> Option<(u32, u32, Size)> {
    if source.width() == 0 || source.height() == 0 || cell.0 == 0 || cell.1 == 0 {
        return None;
    }
    let available_width = (u32::from(target.width) * u32::from(cell.0)).min(MAX_PIXELS);
    let available_height = (u32::from(target.height) * u32::from(cell.1)).min(MAX_PIXELS);
    let scale = (available_width / source.width()).min(available_height / source.height());
    if scale == 0 {
        return None;
    }
    let width = source.width() * scale;
    let height = source.height() * scale;
    let size = Size::new(
        width.div_ceil(u32::from(cell.0)) as u16,
        height.div_ceil(u32::from(cell.1)) as u16,
    );
    // Include alignment padding in the allocation bound.
    if u32::from(size.width) * u32::from(cell.0) > MAX_PIXELS
        || u32::from(size.height) * u32::from(cell.1) > MAX_PIXELS
    {
        return None;
    }
    Some((width, height, size))
}

fn native_pixels(
    source: &RgbaImage,
    width: u32,
    height: u32,
    size: Size,
    cell: (u16, u16),
    background: Option<[u8; 3]>,
) -> RgbaImage {
    let padded_width = u32::from(size.width) * u32::from(cell.0);
    let padded_height = u32::from(size.height) * u32::from(cell.1);
    let mut canvas = RgbaImage::from_pixel(
        padded_width,
        padded_height,
        match background {
            Some([r, g, b]) => Rgba([r, g, b, 255]),
            None => Rgba([0, 0, 0, 0]),
        },
    );
    let scale = width / source.width();
    let left = (padded_width - width) / 2;
    let top = (padded_height - height) / 2;
    // One bounded allocation, direct integer replication and alpha composition.
    // Padding prevents cell-sized Kitty placements from stretching partial cells.
    for y in 0..height {
        for x in 0..width {
            let pixel = source.get_pixel(x / scale, y / scale);
            let Some(background) = background else {
                canvas.put_pixel(left + x, top + y, *pixel);
                continue;
            };
            let alpha = u32::from(pixel[3]);
            let rgb: [u8; 3] = std::array::from_fn(|channel| {
                ((u32::from(pixel[channel]) * alpha
                    + u32::from(background[channel]) * (255 - alpha)
                    + 127)
                    / 255) as u8
            });
            canvas.put_pixel(left + x, top + y, Rgba([rgb[0], rgb[1], rgb[2], 255]));
        }
    }
    canvas
}

fn render_protocol(protocol: &Protocol, area: Rect, buffer: &mut Buffer) {
    Image::new(protocol).render(area, buffer);
    // An escaped Sixel payload is not visible text width. Do not unskip any of
    // the later rows, which the protocol deliberately reserves for pixel data.
    if let Some(width) = NonZeroU16::new(area.width)
        && let Some(cell) = buffer.cell_mut((area.x, area.y))
    {
        cell.set_diff_option(CellDiffOption::ForcedWidth(width));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_preserves_source_bytes_and_uses_only_basic_cached_placements() {
        let image = RgbaImage::from_pixel(96, 96, Rgba([80, 120, 160, 255]));
        let data = encode_kitty(&image, image_id(0));
        assert!(data.contains("a=t,f=32,t=d"));
        assert!(data.contains("s=96,v=96"));
        assert!(!data.contains("U=1"));
        assert!(!data.contains('\u{10EEEE}'));
        let mut decoded = Vec::new();
        for command in data.split("\x1b_G").skip(1) {
            let (header, payload) = command.split_once(';').unwrap();
            let payload = payload.strip_suffix("\x1b\\").unwrap();
            assert!(payload.len() <= 4096);
            assert!(header.contains("q=2"));
            decoded.extend(base64_simd::STANDARD.decode_to_vec(payload).unwrap());
        }
        assert_eq!(decoded, *image.as_raw());
        let mut graphics = NativeGraphics {
            eligible: true,
            kind: Some(Kind::Kitty),
            cell: Some((8, 16)),
            cache: std::array::from_fn(|_| None),
            visible: 1,
            omitted: [None; 9],
            status: String::new(),
        };
        graphics.cache[0] = Some(Cached {
            target: Rect::new(3, 4, 12, 6),
            cell: (8, 16),
            background: [0; 3],
            generation: 1,
            kind: Kind::Kitty,
            size: Size::new(12, 6),
            encoded: Encoded::Kitty {
                data,
                transmitted: false,
            },
        });
        let mut first = Vec::new();
        graphics.paint_to(&mut first).unwrap();
        let first = String::from_utf8(first).unwrap();
        assert!(first.contains("a=t,"));
        assert!(first.contains("\x1b[5;4H"));
        assert!(first.contains("p=1,c=12,r=6,C=1"));
        let mut second = Vec::new();
        graphics.paint_to(&mut second).unwrap();
        let second = String::from_utf8(second).unwrap();
        assert!(!second.contains("a=t,"));
        assert!(second.contains("a=p,"));
        // A full erase during resize can remove the terminal's cached pixels.
        graphics.terminal_cleared();
        let mut restored = Vec::new();
        graphics.paint_to(&mut restored).unwrap();
        let restored = String::from_utf8(restored).unwrap();
        let mut pixels = Vec::new();
        for command in restored.split("\x1b_G").skip(1) {
            let (header, payload) = command.split_once(';').unwrap();
            let payload = payload.split("\x1b\\").next().unwrap();
            if !header.contains("a=p") {
                pixels.extend(base64_simd::STANDARD.decode_to_vec(payload).unwrap());
            } else {
                assert_eq!(
                    pixels,
                    *image.as_raw(),
                    "restore pixels before placing them"
                );
            }
        }
        graphics.visible = 0;
        let mut hidden = Vec::new();
        graphics.paint_to(&mut hidden).unwrap();
        assert!(hidden.is_empty());
        delete(0, false, &mut hidden).unwrap();
        assert_eq!(
            String::from_utf8(hidden).unwrap(),
            format!("\x1b_Ga=d,d=i,i={},q=2;\x1b\\", image_id(0))
        );
    }

    #[test]
    fn native_geometry_never_shrinks_or_fractionally_scales() {
        let source = RgbaImage::new(96, 96);
        assert_eq!(
            native_geometry(&source, Rect::new(0, 0, 20, 6), (8, 16)),
            Some((96, 96, Size::new(12, 6)))
        );
        assert_eq!(
            native_geometry(&source, Rect::new(0, 0, 30, 13), (8, 16)),
            Some((192, 192, Size::new(24, 12)))
        );
        assert!(native_geometry(&source, Rect::new(0, 0, 11, 6), (8, 16)).is_none());
        assert!(native_geometry(&source, Rect::new(0, 0, 20, 5), (8, 16)).is_none());
        assert!(native_geometry(&source, Rect::new(0, 0, 20, 6), (0, 16)).is_none());
    }

    #[test]
    fn pixels_replicate_exactly_and_composite_on_theme_background() {
        let mut source = RgbaImage::from_pixel(2, 1, Rgba([80, 120, 160, 255]));
        source.put_pixel(1, 0, Rgba([255, 0, 0, 0]));
        let pixels = native_pixels(&source, 4, 2, Size::new(2, 1), (3, 4), Some([10, 20, 30]));
        assert_eq!(pixels.dimensions(), (6, 4));
        for y in 1..3 {
            for x in 1..3 {
                assert_eq!(*pixels.get_pixel(x, y), Rgba([80, 120, 160, 255]));
            }
        }
        assert_eq!(*pixels.get_pixel(3, 1), Rgba([10, 20, 30, 255]));
        assert_eq!(*pixels.get_pixel(0, 0), Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn kitty_pixels_preserve_alpha_and_transparent_cell_padding() {
        let mut source = RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]));
        source.put_pixel(1, 0, Rgba([255, 255, 255, 255]));
        source.put_pixel(0, 1, Rgba([80, 120, 160, 128]));
        source.put_pixel(1, 1, Rgba([255, 0, 0, 0]));
        let pixels = native_pixels(&source, 4, 4, Size::new(2, 2), (3, 3), None);
        for y in 0..6 {
            for x in 0..6 {
                let expected = if (1..5).contains(&x) && (1..5).contains(&y) {
                    *source.get_pixel((x - 1) / 2, (y - 1) / 2)
                } else {
                    Rgba([0, 0, 0, 0])
                };
                assert_eq!(*pixels.get_pixel(x, y), expected);
            }
        }
        let opaque = native_pixels(&source, 4, 4, Size::new(2, 2), (3, 3), Some([10, 20, 30]));
        assert!(opaque.pixels().all(|pixel| pixel[3] == 255));
        assert_eq!(*opaque.get_pixel(1, 3), Rgba([45, 70, 95, 255]));
        assert_eq!(*opaque.get_pixel(3, 3), Rgba([10, 20, 30, 255]));
        assert_eq!(*opaque.get_pixel(0, 0), Rgba([10, 20, 30, 255]));
    }

    #[test]
    fn image_transmission_does_not_skip_neighbouring_text_or_later_rows() {
        let protocol = Protocol::Sixel(
            Sixel::new(
                DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([100, 150, 200, 255]))),
                Size::new(2, 2),
                false,
            )
            .unwrap(),
        );
        let area = Rect::new(0, 0, 12, 4);
        let before = Buffer::empty(area);
        let mut after = before.clone();
        render_protocol(&protocol, Rect::new(1, 0, 2, 2), &mut after);
        after[(5, 0)].set_symbol("A");
        after[(0, 3)].set_symbol("Z");
        let changed = before.diff(&after);
        assert!(
            changed
                .iter()
                .any(|(x, y, cell)| (*x, *y) == (5, 0) && cell.symbol() == "A")
        );
        assert!(
            changed
                .iter()
                .any(|(x, y, cell)| (*x, *y) == (0, 3) && cell.symbol() == "Z")
        );
    }
}
