use std::io::{self, Write};
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU16, Ordering};

use image::{DynamicImage, imageops::FilterType};
use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Rect, Size},
    widgets::Widget,
};
use ratatui_image::{
    Image,
    protocol::{Protocol, kitty::Kitty},
};

use crate::{
    artwork::Artwork,
    config::{ColorMode, Config, ImageMode},
};

// IDs are private to this application's alternate screen. Delete only our IDs,
// never every image in the terminal. The restoration hook runs before leaving it.
const IMAGE_BASE: u32 = 0xEE00_0000;
static ALLOCATED: AtomicU16 = AtomicU16::new(0);

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
    if allocated == 0 {
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
    if let Err(error) = output.flush() {
        failure.get_or_insert(error);
    }
    failure.map_or(Ok(()), Err)
}

struct Cached {
    target: Rect,
    cell: (u16, u16),
    protocol: Protocol,
}

pub struct NativeGraphics {
    eligible: bool,
    cell: Option<(u16, u16)>,
    cache: [Option<Cached>; 9],
    visible: u16,
    status: String,
}

impl NativeGraphics {
    pub fn new(config: &Config) -> Self {
        // No stdin discovery: Crossterm remains the sole input owner. Native
        // rendering is qualified by observed version, not terminal brand alone.
        let eligible = cfg!(all(target_os = "macos", target_arch = "aarch64"))
            && config.images == ImageMode::Auto
            && config.color != ColorMode::Mono
            && !config.ascii
            && std::env::var("TERM_PROGRAM").as_deref() == Ok("ghostty")
            && std::env::var("TERM_PROGRAM_VERSION").as_deref() == Ok("1.3.1")
            && ["TMUX", "STY", "SSH_CONNECTION", "SSH_TTY"]
                .iter()
                .all(|key| std::env::var_os(key).is_none_or(|v| v.is_empty()));
        let mut graphics = Self {
            eligible,
            cell: None,
            cache: std::array::from_fn(|_| None),
            visible: 0,
            status: "Cells: native protocol not qualified for this terminal/version".into(),
        };
        if config.images == ImageMode::Off {
            graphics.status = "Images off; no graphics discovery".into();
        } else if config.images == ImageMode::Cells
            || config.ascii
            || config.color == ColorMode::Mono
        {
            graphics.status = "Cells/text: graphics discovery bypassed".into();
        }
        graphics.discover_cell();
        graphics
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn is_native(&self) -> bool {
        self.cell.is_some()
    }

    pub fn discover_cell(&mut self) {
        if !self.eligible {
            return;
        }
        // window_size is a kernel/console query, not a terminal escape query.
        // A missing pixel report immediately falls back; no reader or timer can
        // swallow typed keys or leave late query responses in the editor.
        self.cell = crossterm::terminal::window_size().ok().and_then(|size| {
            if size.columns == 0 || size.rows == 0 {
                return None;
            }
            let width = size.width / size.columns;
            let height = size.height / size.rows;
            (width > 0 && width <= 64 && height > 0 && height <= 128).then_some((width, height))
        });
        self.status = if self.cell.is_some() {
            "Native Kitty: Ghostty 1.3.1 / kernel-reported cell size".into()
        } else {
            "Cells: terminal supplied no usable pixel/cell dimensions".into()
        };
    }

    pub fn prepare(&mut self, artwork: &Artwork, targets: &[(usize, Rect)]) -> io::Result<()> {
        let Some(cell) = self.cell else {
            if self.visible != 0 {
                let mut output = io::stdout().lock();
                for index in 0..9 {
                    if self.visible & (1 << index) != 0 {
                        delete(index, false, &mut output)?;
                    }
                }
                output.flush()?;
                self.visible = 0;
            }
            return Ok(());
        };
        let mut next_visible = 0u16;
        let mut output = io::stdout().lock();
        for &(index, target) in targets {
            if index >= 9 || target.width == 0 || target.height == 0 {
                continue;
            }
            let Some(source) = artwork.original(index) else {
                continue;
            };
            let pixel_width = u32::from(target.width) * u32::from(cell.0);
            let pixel_height = u32::from(target.height) * u32::from(cell.1);
            if pixel_width > 2048 || pixel_height > 2048 {
                continue;
            }
            let regenerate = self.cache[index].as_ref().is_none_or(|cache| {
                cache.target.width != target.width
                    || cache.target.height != target.height
                    || cache.cell != cell
            });
            if regenerate {
                if self.cache[index].is_some() {
                    delete(index, true, &mut output)?;
                }
                let scale = (pixel_width as f64 / source.width() as f64)
                    .min(pixel_height as f64 / source.height() as f64);
                let width = ((source.width() as f64 * scale).floor() as u32).max(1);
                let height = ((source.height() as f64 * scale).floor() as u32).max(1);
                let image = image::imageops::resize(source, width, height, FilterType::Lanczos3);
                let size = Size::new(
                    width.div_ceil(u32::from(cell.0)) as u16,
                    height.div_ceil(u32::from(cell.1)) as u16,
                );
                let protocol = Kitty::new(
                    DynamicImage::ImageRgba8(image),
                    size,
                    image_id(index),
                    false,
                )
                .map(Protocol::Kitty)
                .map_err(io::Error::other)?;
                self.cache[index] = Some(Cached {
                    target,
                    cell,
                    protocol,
                });
                ALLOCATED.fetch_or(1 << index, Ordering::SeqCst);
            } else {
                let cache = self.cache[index].as_mut().expect("prepared protocol");
                let moved = cache.target != target;
                if moved {
                    delete(index, false, &mut output)?;
                }
                cache.target = target;
                // Recreate virtual placement from retained data when an overlay
                // hid it. No image re-encoding/retransmission is necessary.
                if moved || self.visible & (1 << index) == 0 {
                    write!(output, "\x1b_Ga=p,U=1,i={},q=2;\x1b\\", image_id(index))?;
                }
            }
            next_visible |= 1 << index;
        }
        for index in 0..9 {
            if self.visible & !next_visible & (1 << index) != 0 {
                delete(index, false, &mut output)?;
            }
        }
        self.visible = next_visible;
        output.flush()
    }

    pub fn render(&self, buffer: &mut Buffer) {
        for (index, cached) in self.cache.iter().enumerate() {
            if self.visible & (1 << index) == 0 {
                continue;
            }
            let cached = cached.as_ref().expect("visible image prepared");
            let size = cached.protocol.size();
            let area = Rect::new(
                cached.target.x + (cached.target.width - size.width) / 2,
                cached.target.y + (cached.target.height - size.height) / 2,
                size.width,
                size.height,
            );
            render_protocol(&cached.protocol, area, buffer);
        }
    }
}

fn render_protocol(protocol: &Protocol, area: Rect, buffer: &mut Buffer) {
    Image::new(protocol).render(area, buffer);
    // ratatui-image 11 emits an entire escaped row in its first cell. Ratatui
    // 0.30.2 otherwise measures the payload as visible text and skips the rest
    // of the frame. Account for physical cells, not base64/escape byte width.
    if let Some(width) = NonZeroU16::new(area.width) {
        for y in area.y..area.bottom() {
            if let Some(cell) = buffer.cell_mut((area.x, y)) {
                cell.set_diff_option(CellDiffOption::ForcedWidth(width));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn image_transmission_does_not_skip_neighbouring_text_or_later_rows() {
        let area = Rect::new(0, 0, 12, 4);
        let before = Buffer::empty(area);
        let mut after = before.clone();
        let protocol = Protocol::Kitty(
            Kitty::new(
                DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([100, 150, 200, 255]))),
                Size::new(2, 2),
                17,
                false,
            )
            .unwrap(),
        );
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
        let hidden = Buffer::empty(area);
        assert!(
            after
                .diff(&hidden)
                .iter()
                .any(|(x, y, _)| (*x, *y) == (5, 0))
        );
    }
}
