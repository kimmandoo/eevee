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
        no_animation: false,
        animation_override: None,
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

fn key(color: ColorMode, ascii: bool, width: u16, height: u16, background: [u8; 3]) -> CacheKey {
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
    assert_eq!(fitted_dimensions(&image, 10, 10), (8, 8));
    assert_eq!(fitted_dimensions(&image, 20, 5), (8, 8));
    assert_eq!(fitted_dimensions(&image, 20, 10), (16, 16));
    assert_eq!(fitted_dimensions(&image, 5, 10), (5, 5));
    let raster = rasterize(
        &image,
        key(ColorMode::TrueColor, false, 10, 10, [10, 20, 30]),
        0,
    );
    assert_eq!((raster.width, raster.height), (8, 4));
    assert_eq!(raster.cells[0].foreground, Color::Rgb(220, 100, 40));
    assert_eq!(raster.cells[0].background, Color::Rgb(220, 100, 40));
    let transparent = RgbaImage::from_pixel(1, 1, Rgba([200, 30, 20, 0]));
    assert_eq!(sample(&transparent, 0, 0, 1, 1, [10, 20, 30]), [10, 20, 30]);
    // Quadrants split each cell horizontally, keeping only original colors.
    let mut split = RgbaImage::from_pixel(2, 4, Rgba([0, 0, 255, 255]));
    for y in 0..4 {
        split.put_pixel(0, y, Rgba([255, 0, 0, 255]));
    }
    let raster = rasterize(&split, key(ColorMode::TrueColor, false, 1, 1, [0; 3]), 0);
    assert_eq!((raster.width, raster.height), (1, 1));
    assert_eq!(raster.cells[0].symbol, "▌");
    assert_eq!(raster.cells[0].foreground, Color::Rgb(255, 0, 0));
    assert_eq!(raster.cells[0].background, Color::Rgb(0, 0, 255));
    let three = [[200, 0, 0], [0, 0, 200], [0, 0, 200], [190, 0, 0]];
    assert_eq!(quadrant(three), ("▚", [200, 0, 0], [0, 0, 200]));
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
    assert_eq!(buffer[(0, 2)].symbol(), "█");
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
fn sprites_keep_exact_palette_and_trim_only_empty_border() {
    let mut image = RgbaImage::new(16, 16);
    for y in 4..12 {
        for x in 2..14 {
            image.put_pixel(x, y, Rgba([240, 120, 60, 255]));
        }
    }
    let cropped = trim_transparent_border(image);
    assert_eq!(cropped.dimensions(), (12, 8));
    assert_eq!(cropped.get_pixel(0, 0), &Rgba([240, 120, 60, 255]));
    let mut pixels = RgbaImage::from_pixel(2, 2, Rgba([240, 120, 60, 255]));
    pixels.put_pixel(0, 0, Rgba([10, 20, 30, 255]));
    // Integer enlargement repeats pixels, never inventing interpolated colors.
    for y in 0..6 {
        for x in 0..6 {
            let expected = if x < 3 && y < 3 {
                [10, 20, 30]
            } else {
                [240, 120, 60]
            };
            assert_eq!(sample(&pixels, x, y, 6, 6, [0; 3]), expected);
        }
    }
    assert_eq!(sample(&pixels, 0, 0, 1, 1, [0; 3]), [240, 120, 60]);
    let transparent = RgbaImage::new(4, 4);
    assert_eq!(trim_transparent_border(transparent).dimensions(), (4, 4));
}

#[test]
fn forms_on_one_canvas_share_a_crop_so_they_render_at_one_scale() {
    let sprite = |left, top, right, bottom| {
        let mut image = RgbaImage::new(16, 16);
        for y in top..bottom {
            for x in left..right {
                image.put_pixel(x, y, Rgba([240, 120, 60, 255]));
            }
        }
        image
    };
    let mut images = [Some(sprite(6, 8, 10, 14)), None, Some(sprite(2, 3, 14, 14))];
    trim_shared_border(&mut images);
    assert_eq!(images[0].as_ref().unwrap().dimensions(), (12, 11));
    assert_eq!(images[2].as_ref().unwrap().dimensions(), (12, 11));
    // The small form keeps its relative size and baseline.
    assert_eq!(images[0].as_ref().unwrap().get_pixel(4, 10)[3], 255);
    assert_eq!(images[0].as_ref().unwrap().get_pixel(0, 0)[3], 0);

    let mut mixed = [
        Some(sprite(6, 8, 10, 14)),
        Some(RgbaImage::from_pixel(3, 3, Rgba([1, 2, 3, 255]))),
    ];
    trim_shared_border(&mut mixed);
    assert_eq!(mixed[0].as_ref().unwrap().dimensions(), (4, 6));
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
fn auto_falls_back_to_cells_only_after_native_graphics_are_ruled_out() {
    let directory = TestDirectory::new();
    let image = RgbaImage::from_pixel(96, 96, Rgba([80, 120, 160, 255]));
    fs::write(directory.0.join("133.png"), png(&image)).unwrap();
    let mut settings = config(&directory.0);
    settings.images = ImageMode::Auto;
    let mut artwork = Artwork::load(&settings);
    let target = Rect::new(0, 0, 20, 6);
    let render = |artwork: &Artwork| {
        let mut buffer = Buffer::empty(target);
        artwork.render(0, target, &mut buffer);
        buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    };
    // Still detecting: no mosaic that a native image would replace a moment later.
    artwork.prepare(&[(0, target)], Color::Black);
    assert!(artwork.cache.is_empty());
    assert!(render(&artwork).contains("No native image"));
    // Detection settled without native graphics: cell art.
    artwork.set_cell_fallback(true);
    artwork.prepare(&[(0, target)], Color::Black);
    assert_eq!(artwork.cache.len(), 1);
    assert!(render(&artwork).contains('█'));
    // Native graphics own the target even if the fallback latch is set.
    artwork.set_native_active(true);
    assert!(render(&artwork).chars().all(|cell| cell == ' '));
    // Explicit modes are unaffected by the auto fallback.
    settings.images = ImageMode::Off;
    let mut off = Artwork::load(&settings);
    off.set_cell_fallback(true);
    off.prepare(&[(0, target)], Color::Black);
    assert!(off.cache.is_empty());
}

#[test]
fn sixel_requires_full_erase_on_overlay_move_resize_and_capability_loss() {
    use crate::{platform::input::GraphicsCapabilities, platform::native::NativeGraphics};

    let directory = TestDirectory::new();
    fs::write(
        directory.0.join("133.png"),
        png(&RgbaImage::from_pixel(96, 96, Rgba([80, 120, 160, 255]))),
    )
    .unwrap();
    let mut settings = config(&directory.0);
    settings.images = ImageMode::Auto;
    let artwork = Artwork::load(&settings);
    let mut graphics = NativeGraphics::new(&settings);
    graphics.apply_capabilities(GraphicsCapabilities {
        kitty: false,
        sixel: true,
        cell_size: Some((8, 16)),
    });
    let target = Rect::new(1, 1, 20, 8);
    assert!(
        !graphics
            .prepare(&artwork, &[(0, target)], Color::Black)
            .unwrap()
    );
    // Idle drawing does not clear or re-encode the cached Sixel.
    assert!(
        !graphics
            .prepare(&artwork, &[(0, target)], Color::Black)
            .unwrap()
    );
    assert!(graphics.prepare(&artwork, &[], Color::Black).unwrap());
    assert!(!graphics.prepare(&artwork, &[], Color::Black).unwrap());
    assert!(
        !graphics
            .prepare(&artwork, &[(0, target)], Color::Black)
            .unwrap()
    );
    let moved = Rect::new(2, 1, 20, 8);
    assert!(
        graphics
            .prepare(&artwork, &[(0, moved)], Color::Black)
            .unwrap()
    );
    let resized = Rect::new(2, 1, 24, 12);
    assert!(
        graphics
            .prepare(&artwork, &[(0, resized)], Color::Black)
            .unwrap()
    );
    let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 20));
    graphics.render(&mut buffer);
    // Sixel raster header declares the original integer-2x pixel canvas,
    // not a 24x12 or 24x24 terminal-cell approximation.
    assert!(
        buffer
            .content
            .iter()
            .any(|cell| cell.symbol().contains(";192;192"))
    );
    graphics.apply_capabilities(GraphicsCapabilities::default());
    assert!(
        graphics
            .prepare(&artwork, &[(0, resized)], Color::Black)
            .unwrap()
    );
    assert!(!graphics.is_native());
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
