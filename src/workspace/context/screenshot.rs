//! Explicit binary source capture; independent of PNG-only species artwork.
use crate::workspace::repo::{self, FileState};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

pub const MAX_COMPRESSED_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 4096;
pub const MAX_PIXELS: u64 = 8_388_608;
pub const MAX_DECODED_BYTES: u64 = 32 * 1024 * 1024;

/// Worker-produced, process-local disclosure, never a configuration authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteReview {
    pub task_id: i64,
    pub partner: i64,
    pub agent: crate::providers::Agent,
    pub selection: crate::providers::ModelSelection,
    pub capability: Result<String, String>,
}
impl RouteReview {
    pub fn matches(
        &self,
        task_id: i64,
        partner: i64,
        agent: crate::providers::Agent,
        selection: &crate::providers::ModelSelection,
    ) -> bool {
        self.task_id == task_id
            && self.partner == partner
            && self.agent == agent
            && &self.selection == selection
    }
}

#[derive(Clone, Debug)]
pub struct Captured {
    pub attachment: Attachment,
    pub route: RouteReview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Png,
    Jpeg,
}
impl Format {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
        }
    }
    fn decoder(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub task_id: i64,
    pub partner: i64,
    pub root: String,
    pub path: String,
    pub sha256: [u8; 32],
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub bytes: usize,
}
impl Capture {
    pub fn validate(&self) -> Result<(), String> {
        if self.task_id <= 0
            || self.partner <= 0
            || self.root.len() > 32 * 1024
            || self.path.len() > 32 * 1024
            || !Path::new(&self.root).is_absolute()
            || super::excluded(Path::new(&self.path), &[])
            || self.bytes == 0
            || self.bytes > MAX_COMPRESSED_BYTES
            || self.width == 0
            || self.height == 0
            || self.width > MAX_DIMENSION
            || self.height > MAX_DIMENSION
            || u64::from(self.width) * u64::from(self.height) > MAX_PIXELS
        {
            return Err(
                "Invalid or oversized screenshot identity, dimensions or compressed bytes".into(),
            );
        }
        super::validate_project_root(Path::new(&self.root))?;
        if crate::workspace::privacy::redact(&self.root) != self.root
            || crate::workspace::privacy::redact(&self.path) != self.path
            || self.path.chars().chain(self.root.chars()).any(|ch| ch.is_control()
                || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
            return Err("Screenshot source identity contains protected or unsafe text".into());
        }
        Ok(())
    }
    pub fn sha256_hex(&self) -> String {
        super::sha256_hex(&self.sha256)
    }
    pub fn disclosure(&self) -> String {
        format!(
            "Screenshot: {:?}\nRoot: {:?}\n{} · {} × {} · {} compressed bytes\nTask #{} · partner #{} · SHA-256 {}\nTerminal metadata is NOT visual review. Inspect the original image yourself.\nImages may contain secrets; Eevee cannot reliably detect or redact them.\nExact original compressed bytes are sent, without OCR or redaction.\nRetained locally under task quota until explicit purge/task deletion; removal only deselects.\nRestart restores capture, never approval. Provider retention is separate; store:false is requested.",
            self.path,
            self.root,
            self.format.label(),
            self.width,
            self.height,
            self.bytes,
            self.task_id,
            self.partner,
            self.sha256_hex()
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub capture: Capture,
    pub bytes: Arc<[u8]>,
}
impl Attachment {
    pub fn validate(&self) -> Result<(), String> {
        self.capture.validate()?;
        if self.bytes.len() != self.capture.bytes
            || <[u8; 32]>::from(Sha256::digest(&self.bytes)) != self.capture.sha256
        {
            return Err(
                "Screenshot compressed content does not match its immutable identity".into(),
            );
        }
        let (format, width, height) = decode(&self.bytes)?;
        if (format, width, height) != (self.capture.format, self.capture.width, self.capture.height)
        {
            return Err("Screenshot decoded format/dimensions differ from capture".into());
        }
        Ok(())
    }
    pub fn data_url(&self) -> String {
        format!(
            "data:{};base64,{}",
            self.capture.format.mime(),
            base64_simd::STANDARD.encode_to_string(&self.bytes)
        )
    }
    pub fn revalidate(&self, report: &repo::Report, exclusions: &[PathBuf]) -> Result<(), String> {
        self.validate()?;
        if report.repository.root.to_str() != Some(self.capture.root.as_str()) {
            return Err("Screenshot belongs to a different canonical project root".into());
        }
        let stopped = AtomicBool::new(false);
        let digest = source(report, Path::new(&self.capture.path), exclusions, &stopped)?;
        if digest.as_slice() != self.bytes.as_ref() {
            return Err("Screenshot changed since capture; inspect and attach it again".into());
        }
        Ok(())
    }
}

/// Fully decodes bounded PNG/JPEG; headers/extensions alone never authorize input.
fn decode(bytes: &[u8]) -> Result<(Format, u32, u32), String> {
    if bytes.is_empty() || bytes.len() > MAX_COMPRESSED_BYTES {
        return Err("Screenshot compressed size exceeds 2 MiB or is empty".into());
    }
    let format =
        match image::guess_format(bytes).map_err(|_| "Screenshot is not a recognized PNG/JPEG")? {
            ImageFormat::Png => Format::Png,
            ImageFormat::Jpeg => Format::Jpeg,
            _ => return Err("Only PNG and JPEG screenshot input is supported".into()),
        };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format.decoder());
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|e| format!("Screenshot decoder refused input: {e}"))?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || decoder.total_bytes() > MAX_DECODED_BYTES
    {
        return Err("Screenshot exceeds 4096-axis, 8-megapixel or 32 MiB decoded bound".into());
    }
    let mut pixels = vec![
        0;
        usize::try_from(decoder.total_bytes())
            .map_err(|_| "Screenshot allocation overflow")?
    ];
    decoder
        .read_image(&mut pixels)
        .map_err(|e| format!("Screenshot full decode failed: {e}"))?;
    Ok((format, width, height))
}

fn source(
    report: &repo::Report,
    path: &Path,
    exclusions: &[PathBuf],
    stopped: &AtomicBool,
) -> Result<Vec<u8>, String> {
    super::validate_project_root(&report.repository.root)?;
    if super::excluded(path, exclusions) || report.repository.excludes_source(path) {
        return Err(
            "Screenshot is outside the admitted project or excluded/application storage".into(),
        );
    }
    let identity = report
        .current
        .files
        .get(path)
        .ok_or("Screenshot is absent from the current nonignored catalogue; refresh first")?;
    if !matches!(&identity.state, FileState::Regular { bytes, .. } if *bytes > 0 && *bytes <= MAX_COMPRESSED_BYTES as u64)
    {
        return Err("Screenshot source is oversized, linked, missing or unavailable".into());
    }
    if !repo::source_visible(&report.repository.root, path, stopped)? {
        return Err("Screenshot source is now ignored, deleted or unavailable".into());
    }
    let (bytes, digest) =
        repo::read_source(&report.repository.root, path, MAX_COMPRESSED_BYTES, stopped)?;
    if !matches!(&identity.state, FileState::Regular { sha256, .. } if sha256 == &digest) {
        return Err(
            "Screenshot changed since repository observation; refresh and capture again".into(),
        );
    }
    Ok(bytes)
}

pub(super) fn capture(
    task_id: i64,
    partner: i64,
    report: &repo::Report,
    path: &Path,
    exclusions: &[PathBuf],
    stopped: &AtomicBool,
) -> Result<Attachment, String> {
    let bytes = source(report, path, exclusions, stopped)?;
    let (format, width, height) = decode(&bytes)?;
    let attachment = Attachment {
        capture: Capture {
            task_id,
            partner,
            root: report
                .repository
                .root
                .to_str()
                .ok_or("Non-Unicode project root")?
                .into(),
            path: path.to_str().ok_or("Non-Unicode screenshot path")?.into(),
            sha256: Sha256::digest(&bytes).into(),
            format,
            width,
            height,
            bytes: bytes.len(),
        },
        bytes: bytes.into(),
    };
    attachment.capture.validate()?;
    Ok(attachment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{
        ImageEncoder,
        codecs::{jpeg::JpegEncoder, png::PngEncoder},
    };
    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }
    #[test]
    fn actual_decode_accepts_png_jpeg_and_rejects_other_truncated_and_oversized() {
        assert_eq!(decode(&png()).unwrap(), (Format::Png, 1, 1));
        let mut jpeg = Vec::new();
        JpegEncoder::new(&mut jpeg)
            .encode(&[1, 2, 3], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        assert_eq!(decode(&jpeg).unwrap(), (Format::Jpeg, 1, 1));
        assert!(decode(b"GIF89a").is_err());
        let p = png();
        assert!(decode(&p[..p.len() / 2]).is_err());
        assert!(decode(&vec![0; MAX_COMPRESSED_BYTES + 1]).is_err());
        let mut large = Vec::new();
        PngEncoder::new(&mut large)
            .write_image(&vec![0; 4097 * 4], 4097, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert!(decode(&large).is_err());
    }
    #[test]
    fn immutable_compressed_content_and_dimensions_are_not_metadata_trust() {
        let bytes = png();
        let mut a = Attachment {
            capture: Capture {
                task_id: 1,
                partner: 1,
                root: if cfg!(windows) {
                    r"C:\project"
                } else {
                    "/project"
                }
                .into(),
                path: "shot.bin".into(),
                sha256: Sha256::digest(&bytes).into(),
                format: Format::Png,
                width: 1,
                height: 1,
                bytes: bytes.len(),
            },
            bytes: bytes.into(),
        };
        a.validate().unwrap();
        assert!(a.data_url().starts_with("data:image/png;base64,"));
        a.capture.width = 2;
        assert!(a.validate().is_err());
        a.capture.width = 1;
        a.bytes = vec![0; a.capture.bytes].into();
        assert!(a.validate().is_err());
    }
}
