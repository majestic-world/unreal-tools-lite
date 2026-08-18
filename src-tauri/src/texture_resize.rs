//! Recursive icon downscaler for legacy Lineage 2 texture folders.
//!
//! The tool is intentionally independent from UTX mutation. It works on the
//! exported DDS/TGA files, preserves their container type, and writes a clean
//! sibling tree under `modified/<source-folder-name>`.

use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use image_dds::{
    dds_from_image, dds_image_format,
    ddsfile::Dds,
    image::{
        imageops::{self, FilterType},
        DynamicImage, ImageFormat as RasterImageFormat, Rgba, Rgba32FImage, RgbaImage,
    },
    image_from_dds, ImageFormat as DdsImageFormat, Mipmaps, Quality,
};
use rayon::prelude::*;
use serde::Serialize;

type ResizeResult<T> = Result<T, String>;

const MIN_UE2_TEXTURE_RESOLUTION: u32 = 4;
const MAX_UE2_TEXTURE_RESOLUTION: u32 = 2048;
const MAX_REPORTED_ERRORS: usize = 30;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeProgress {
    pub completed: usize,
    pub total: usize,
    pub file_name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeSummary {
    pub output_directory: String,
    pub total_files: usize,
    pub resized_files: usize,
    pub preserved_files: usize,
    pub copied_metadata: usize,
    pub failed_files: usize,
    pub errors: Vec<String>,
}

#[derive(Default)]
struct ResizeTotals {
    resized_files: usize,
    preserved_files: usize,
    copied_metadata: usize,
    failed_files: usize,
    errors: Vec<String>,
}

#[derive(Clone, Copy)]
enum TextureKind {
    Tga,
    Dds(DdsImageFormat),
}

struct DecodedTexture {
    image: RgbaImage,
    kind: TextureKind,
}

/// Returns whether a square texture size is accepted by the UE2-oriented
/// workflow. Power-of-two dimensions are required by the target renderer.
pub fn is_supported_resolution(resolution: u32) -> bool {
    resolution.is_power_of_two()
        && (MIN_UE2_TEXTURE_RESOLUTION..=MAX_UE2_TEXTURE_RESOLUTION).contains(&resolution)
}

/// Resizes only textures that match `source_resolution` under `directory`,
/// recursively. Textures at every other size are copied byte-for-byte. The
/// source tree is never changed; output is written under
/// `modified/<source-name>`.
///
/// DDS DXT1, DXT3 and DXT5 are decoded, resized using premultiplied-alpha
/// Lanczos3 filtering, and encoded again in the original DXT format. TGA
/// images remain TGA. A sibling exported metadata TXT is copied and its
/// geometry-dependent values are scaled when the image dimensions change.
pub fn resize_directory_with_progress<F>(
    directory: &str,
    source_resolution: u32,
    target_resolution: u32,
    progress: F,
) -> ResizeResult<ResizeSummary>
where
    F: Fn(ResizeProgress) + Send + Sync,
{
    validate_resolution(source_resolution)?;
    validate_resolution(target_resolution)?;
    let input_root = Path::new(directory);
    if !input_root.is_dir() {
        return Err("Escolha uma pasta válida que contenha os ícones.".into());
    }
    let canonical_root = input_root
        .canonicalize()
        .map_err(|error| format!("Não foi possível acessar a pasta de ícones: {error}"))?;
    let source_name = canonical_root
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or("Não foi possível determinar o nome da pasta de ícones.")?;
    let output_root = canonical_root.join("modified").join(source_name);
    let ignored_output_parent = canonical_root.join("modified");
    let inputs = collect_texture_files(&canonical_root, &ignored_output_parent)?;
    if inputs.is_empty() {
        return Err("Nenhum arquivo DDS ou TGA foi encontrado na pasta selecionada.".into());
    }

    fs::create_dir_all(&output_root)
        .map_err(|error| format!("Não foi possível criar a pasta de saída: {error}"))?;

    let total = inputs.len();
    let completed = Arc::new(AtomicUsize::new(0));
    let totals = Arc::new(Mutex::new(ResizeTotals::default()));

    inputs.par_iter().for_each(|source| {
        let relative = match source.strip_prefix(&canonical_root) {
            Ok(relative) => relative,
            Err(error) => {
                record_failure(&totals, source, error.to_string());
                return;
            }
        };
        let target = output_root.join(relative);
        match resize_one(source, &target, source_resolution, target_resolution) {
            Ok((resized, copied_metadata)) => {
                if let Ok(mut guard) = totals.lock() {
                    if resized {
                        guard.resized_files += 1;
                    } else {
                        guard.preserved_files += 1;
                    }
                    if copied_metadata {
                        guard.copied_metadata += 1;
                    }
                }
            }
            Err(error) => record_failure(&totals, source, error),
        }

        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
        if done == total || done % 16 == 0 {
            progress(ResizeProgress {
                completed: done,
                total,
                file_name: source
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("textura")
                    .to_owned(),
            });
        }
    });

    let totals = totals
        .lock()
        .map_err(|_| "Não foi possível consolidar o resultado do redimensionamento.")?;
    Ok(ResizeSummary {
        output_directory: display_path(&output_root),
        total_files: total,
        resized_files: totals.resized_files,
        preserved_files: totals.preserved_files,
        copied_metadata: totals.copied_metadata,
        failed_files: totals.failed_files,
        errors: totals.errors.clone(),
    })
}

/// `canonicalize` returns Windows extended-length paths (`\\?\C:\...`). They
/// are useful for filesystem calls but are implementation detail, so never
/// expose that prefix in the desktop UI.
fn display_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc_path) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc_path}")
    } else if let Some(normal_path) = path.strip_prefix(r"\\?\") {
        normal_path.to_owned()
    } else {
        path.into_owned()
    }
}

fn validate_resolution(resolution: u32) -> ResizeResult<()> {
    if is_supported_resolution(resolution) {
        Ok(())
    } else {
        Err("A resolução deve ser uma potência de dois entre 4 e 2048 pixels.".into())
    }
}

fn collect_texture_files(root: &Path, ignored_output_parent: &Path) -> ResizeResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_texture_files_recursive(root, ignored_output_parent, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_texture_files_recursive(
    directory: &Path,
    ignored_output_parent: &Path,
    files: &mut Vec<PathBuf>,
) -> ResizeResult<()> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Não foi possível ler {}: {error}", directory.display()))?
    {
        let entry =
            entry.map_err(|error| format!("Não foi possível ler uma entrada da pasta: {error}"))?;
        let path = entry.path();
        if path == ignored_output_parent {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Não foi possível identificar {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_texture_files_recursive(&path, ignored_output_parent, files)?;
        } else if file_type.is_file() && is_supported_texture_path(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_supported_texture_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("dds") || extension.eq_ignore_ascii_case("tga")
        })
}

fn resize_one(
    source: &Path,
    target: &Path,
    source_resolution: u32,
    target_resolution: u32,
) -> ResizeResult<(bool, bool)> {
    let source_bytes = fs::read(source)
        .map_err(|error| format!("Não foi possível ler {}: {error}", source.display()))?;
    let texture = decode_texture(source, &source_bytes)?;
    let source_width = texture.image.width();
    let source_height = texture.image.height();
    let should_resize = source_width == source_resolution
        && source_height == source_resolution
        && source_resolution != target_resolution;

    let output = if should_resize {
        let resized =
            resize_lanczos3_premultiplied(&texture.image, target_resolution, target_resolution);
        encode_texture(&resized, texture.kind)?
    } else {
        source_bytes
    };
    let parent = target
        .parent()
        .ok_or("Não foi possível determinar a pasta do arquivo de saída.")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Não foi possível criar {}: {error}", parent.display()))?;
    fs::write(target, output)
        .map_err(|error| format!("Não foi possível gravar {}: {error}", target.display()))?;

    let copied_metadata = copy_texture_metadata(
        source,
        target,
        source_width,
        source_height,
        target_resolution,
        should_resize,
    )?;
    Ok((should_resize, copied_metadata))
}

fn decode_texture(path: &Path, bytes: &[u8]) -> ResizeResult<DecodedTexture> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("tga") {
        let image = image_dds::image::load_from_memory_with_format(bytes, RasterImageFormat::Tga)
            .map_err(|error| format!("TGA inválido em {}: {error}", path.display()))?
            .to_rgba8();
        return Ok(DecodedTexture {
            image,
            kind: TextureKind::Tga,
        });
    }

    let dds = Dds::read(Cursor::new(bytes))
        .map_err(|error| format!("DDS inválido em {}: {error}", path.display()))?;
    let format = dds_image_format(&dds)
        .map_err(|error| format!("Formato DDS não suportado em {}: {error:?}", path.display()))?;
    if !matches!(
        format,
        DdsImageFormat::BC1RgbaUnorm | DdsImageFormat::BC2RgbaUnorm | DdsImageFormat::BC3RgbaUnorm
    ) {
        return Err(format!(
            "{} usa um DDS não suportado. Apenas DXT1, DXT3 e DXT5 podem ser redimensionados.",
            path.display()
        ));
    }
    let image = image_from_dds(&dds, 0)
        .map_err(|error| format!("Não foi possível decodificar {}: {error}", path.display()))?;
    Ok(DecodedTexture {
        image,
        kind: TextureKind::Dds(format),
    })
}

fn encode_texture(image: &RgbaImage, kind: TextureKind) -> ResizeResult<Vec<u8>> {
    match kind {
        TextureKind::Tga => {
            let mut output = Cursor::new(Vec::new());
            DynamicImage::ImageRgba8(image.clone())
                .write_to(&mut output, RasterImageFormat::Tga)
                .map_err(|error| format!("Não foi possível codificar o TGA: {error}"))?;
            Ok(output.into_inner())
        }
        TextureKind::Dds(format) => {
            let dds = dds_from_image(image, format, Quality::Slow, Mipmaps::Disabled)
                .map_err(|error| format!("Não foi possível codificar o DDS: {error}"))?;
            build_legacy_dxt_dds(image.width(), image.height(), format, &dds.data)
        }
    }
}

/// UE2's importer only accepts the legacy 128-byte DDS header and the DXT1/
/// DXT3/DXT5 FourCC values. `image_dds` creates a modern DX10 container by
/// default, even for BC1/BC2/BC3, so the already-compressed payload is placed
/// in a legacy container here without changing a single compressed byte.
fn build_legacy_dxt_dds(
    width: u32,
    height: u32,
    format: DdsImageFormat,
    pixels: &[u8],
) -> ResizeResult<Vec<u8>> {
    let (four_cc, block_bytes) = match format {
        DdsImageFormat::BC1RgbaUnorm => (*b"DXT1", 8usize),
        DdsImageFormat::BC2RgbaUnorm => (*b"DXT3", 16usize),
        DdsImageFormat::BC3RgbaUnorm => (*b"DXT5", 16usize),
        _ => return Err("Formato DDS incompatível para o Unreal Engine 2.".into()),
    };
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height).ok().and_then(|height| {
                width
                    .div_ceil(4)
                    .checked_mul(height.div_ceil(4))
                    .and_then(|blocks| blocks.checked_mul(block_bytes))
            })
        })
        .ok_or("Dimensões DDS muito grandes.")?;
    if pixels.len() != expected {
        return Err("O compressor DDS gerou um tamanho de textura inesperado.".into());
    }
    let mut output = vec![0u8; 128];
    output[..4].copy_from_slice(b"DDS ");
    output[4..8].copy_from_slice(&124u32.to_le_bytes());
    output[8..12].copy_from_slice(&0x0008_1007u32.to_le_bytes());
    output[12..16].copy_from_slice(&height.to_le_bytes());
    output[16..20].copy_from_slice(&width.to_le_bytes());
    output[20..24].copy_from_slice(&(expected as u32).to_le_bytes());
    output[76..80].copy_from_slice(&32u32.to_le_bytes());
    output[80..84].copy_from_slice(&4u32.to_le_bytes());
    output[84..88].copy_from_slice(&four_cc);
    output[108..112].copy_from_slice(&0x1000u32.to_le_bytes());
    output.extend_from_slice(pixels);
    Ok(output)
}

/// Lanczos3 is applied after moving RGB to linear, premultiplied-alpha space.
/// This prevents transparent pixels from bleeding black/dark colors into icon
/// edges when a source has soft alpha.
fn resize_lanczos3_premultiplied(source: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    let mut linear = Rgba32FImage::new(source.width(), source.height());
    for (x, y, pixel) in source.enumerate_pixels() {
        let alpha = f32::from(pixel[3]) / 255.0;
        linear.put_pixel(
            x,
            y,
            Rgba([
                srgb_to_linear(pixel[0]) * alpha,
                srgb_to_linear(pixel[1]) * alpha,
                srgb_to_linear(pixel[2]) * alpha,
                alpha,
            ]),
        );
    }
    let scaled = imageops::resize(&linear, width, height, FilterType::Lanczos3);
    let mut output = RgbaImage::new(width, height);
    for (x, y, pixel) in scaled.enumerate_pixels() {
        let alpha = pixel[3].clamp(0.0, 1.0);
        let (red, green, blue) = if alpha > f32::EPSILON {
            (
                linear_to_srgb((pixel[0] / alpha).clamp(0.0, 1.0)),
                linear_to_srgb((pixel[1] / alpha).clamp(0.0, 1.0)),
                linear_to_srgb((pixel[2] / alpha).clamp(0.0, 1.0)),
            )
        } else {
            (0, 0, 0)
        };
        output.put_pixel(
            x,
            y,
            Rgba([red, green, blue, (alpha * 255.0).round() as u8]),
        );
    }
    output
}

fn srgb_to_linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(value: f32) -> u8 {
    let value = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn copy_texture_metadata(
    source: &Path,
    target: &Path,
    source_width: u32,
    source_height: u32,
    target_resolution: u32,
    resized: bool,
) -> ResizeResult<bool> {
    let source_metadata = source.with_extension("txt");
    if !source_metadata.is_file() {
        return Ok(false);
    }
    let target_metadata = target.with_extension("txt");
    let source_text = fs::read_to_string(&source_metadata).map_err(|error| {
        format!(
            "Não foi possível ler os metadados de {}: {error}",
            source_metadata.display()
        )
    })?;
    let text = if resized {
        scale_metadata(&source_text, source_width, source_height, target_resolution)
    } else {
        source_text
    };
    fs::write(&target_metadata, text).map_err(|error| {
        format!(
            "Não foi possível gravar os metadados de {}: {error}",
            target_metadata.display()
        )
    })?;
    Ok(true)
}

fn scale_metadata(source: &str, source_width: u32, source_height: u32, resolution: u32) -> String {
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut section = String::new();
    let mut lines = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_ascii_lowercase();
            lines.push(line.to_owned());
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            lines.push(line.to_owned());
            continue;
        };
        let key_trimmed = key.trim();
        let parsed = value.trim().parse::<i64>();
        let replacement = match (section.as_str(), key_trimmed, parsed) {
            ("texture", "UClamp", Ok(value)) if value == i64::from(source_width) => {
                Some(i64::from(resolution))
            }
            ("texture", "VClamp", Ok(value)) if value == i64::from(source_height) => {
                Some(i64::from(resolution))
            }
            ("split9", "Split9X1" | "Split9X2" | "Split9X3", Ok(value)) => {
                Some(scale_value(value, source_width, resolution))
            }
            ("split9", "Split9Y1" | "Split9Y2" | "Split9Y3", Ok(value)) => {
                Some(scale_value(value, source_height, resolution))
            }
            _ => None,
        };
        if let Some(replacement) = replacement {
            lines.push(format!("{key}={replacement}"));
        } else {
            lines.push(line.to_owned());
        }
    }
    let mut output = lines.join(newline);
    if source.ends_with('\n') {
        output.push_str(newline);
    }
    output
}

fn scale_value(value: i64, source_size: u32, target_size: u32) -> i64 {
    if value <= 0 || source_size == 0 {
        return value.max(0);
    }
    (value * i64::from(target_size) + i64::from(source_size / 2)) / i64::from(source_size)
}

fn record_failure(totals: &Arc<Mutex<ResizeTotals>>, source: &Path, error: String) {
    if let Ok(mut guard) = totals.lock() {
        guard.failed_files += 1;
        if guard.errors.len() < MAX_REPORTED_ERRORS {
            guard.errors.push(format!("{}: {error}", source.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn accepts_only_ue2_power_of_two_resolutions() {
        assert!(is_supported_resolution(32));
        assert!(is_supported_resolution(2048));
        assert!(!is_supported_resolution(48));
        assert!(!is_supported_resolution(4096));
    }

    #[test]
    fn hides_the_windows_extended_path_prefix_from_display() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\Icons\modified\Icons")),
            r"C:\Icons\modified\Icons"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\icons")),
            r"\\server\share\icons"
        );
    }

    #[test]
    fn scales_only_geometry_metadata_values() {
        let metadata = "[Texture]\r\nAlpha=True\r\nUClamp=64\r\nVClamp=32\r\n[Split9]\r\nSplit9X1=8\r\nSplit9X2=16\r\nSplit9X3=40\r\nSplit9Y1=4\r\nSplit9Y2=12\r\nSplit9Y3=16\r\n[Animations]\r\nPrimeCount=3\r\n";
        let scaled = scale_metadata(metadata, 64, 32, 32);
        assert!(scaled.contains("UClamp=32\r\n"));
        assert!(scaled.contains("VClamp=32\r\n"));
        assert!(scaled.contains("Split9X1=4\r\n"));
        assert!(scaled.contains("Split9Y2=12\r\n"));
        assert!(scaled.contains("PrimeCount=3\r\n"));
    }

    #[test]
    fn resizes_recursively_into_the_modified_tree() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir()
            .join(format!("unreal-tools-resize-{unique}"))
            .join("etc_i");
        let nested = root.join("panel");
        fs::create_dir_all(&nested).unwrap();
        let image = RgbaImage::from_fn(64, 64, |x, y| {
            Rgba([
                (x * 4) as u8,
                (y * 4) as u8,
                120,
                if x + y > 20 { 255 } else { 0 },
            ])
        });
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut encoded, RasterImageFormat::Tga)
            .unwrap();
        let source = nested.join("button.tga");
        fs::write(&source, encoded.into_inner()).unwrap();
        fs::write(
            source.with_extension("txt"),
            "[Texture]\nUClamp=64\nVClamp=64\n[Split9]\nSplit9X1=8\nSplit9Y1=8\n",
        )
        .unwrap();
        let dds = dds_from_image(
            &RgbaImage::from_fn(64, 64, |x, y| Rgba([(x * 4) as u8, 40, (y * 4) as u8, 255])),
            DdsImageFormat::BC1RgbaUnorm,
            Quality::Slow,
            Mipmaps::Disabled,
        )
        .unwrap();
        let mut dds_output = Vec::new();
        dds.write(&mut dds_output).unwrap();
        fs::write(nested.join("button_dxt.dds"), dds_output).unwrap();

        let non_matching = nested.join("already_32.tga");
        let mut non_matching_encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(32, 32, Rgba([7, 8, 9, 255])))
            .write_to(&mut non_matching_encoded, RasterImageFormat::Tga)
            .unwrap();
        let non_matching_bytes = non_matching_encoded.into_inner();
        fs::write(&non_matching, &non_matching_bytes).unwrap();

        let summary =
            resize_directory_with_progress(root.to_str().unwrap(), 64, 32, |_| {}).unwrap();
        let output = root
            .join("modified")
            .join("etc_i")
            .join("panel")
            .join("button.tga");
        let output_image = image_dds::image::load_from_memory_with_format(
            &fs::read(&output).unwrap(),
            RasterImageFormat::Tga,
        )
        .unwrap();
        assert_eq!((output_image.width(), output_image.height()), (32, 32));
        let output_dds = Dds::read(
            fs::File::open(
                root.join("modified")
                    .join("etc_i")
                    .join("panel")
                    .join("button_dxt.dds"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!((output_dds.get_width(), output_dds.get_height()), (32, 32));
        assert_eq!(
            dds_image_format(&output_dds).unwrap(),
            DdsImageFormat::BC1RgbaUnorm
        );
        assert_eq!(
            fs::metadata(
                root.join("modified")
                    .join("etc_i")
                    .join("panel")
                    .join("button_dxt.dds")
            )
            .unwrap()
            .len(),
            640
        );
        assert_eq!(summary.total_files, 3);
        assert_eq!(summary.resized_files, 2);
        assert_eq!(summary.preserved_files, 1);
        assert_eq!(summary.copied_metadata, 1);
        assert_eq!(
            fs::read(
                root.join("modified")
                    .join("etc_i")
                    .join("panel")
                    .join("already_32.tga")
            )
            .unwrap(),
            non_matching_bytes
        );
        assert!(fs::read_to_string(output.with_extension("txt"))
            .unwrap()
            .contains("UClamp=32"));

        let second_summary =
            resize_directory_with_progress(root.to_str().unwrap(), 64, 32, |_| {}).unwrap();
        assert_eq!(second_summary.total_files, 3);
        fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }
}
