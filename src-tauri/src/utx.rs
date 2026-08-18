//! Lineage 2 UTX reader, exporter and bridge to the native texture engine.
//!
//! UTX packages share the Unreal Engine 2 container used by UGX, but texture
//! exports contain Unreal properties and mip maps.  Keeping this module free of
//! Tauri types makes every binary operation directly testable.

// Mutating UTX operations live in `texture_engine`. The former writer remains
// here only as executable fixture/reference code while the reader still shares
// its package records, so its private helpers are intentionally not all called
// by the production module.
#![allow(dead_code)]

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};

use crate::texture_engine;

const PACKAGE_MAGIC: i32 = 0x9e2a83c1_u32 as i32;
const NEW_UTX_TEMPLATE: &[u8] = include_bytes!("../assets/UnrealTlp.utx");
const NEW_UTX_TEMPLATE_PACKAGE_NAME: &str = "UnrealTlp";
const MAX_TEXTURE_IMPORT_ERRORS: usize = 20;
const MAX_UTX_EXTRACT_ERRORS: usize = 20;
const IMPORT_PROGRESS_STEP: usize = 32;
const IMPORT_PROGRESS_INTERVAL: Duration = Duration::from_millis(75);
type UtxResult<T> = Result<T, String>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UtxFormat {
    #[serde(rename = "P8")]
    P8,
    #[serde(rename = "RGBA7")]
    Rgba7,
    #[serde(rename = "RGB16")]
    Rgb16,
    #[serde(rename = "DXT1")]
    Dxt1,
    #[serde(rename = "RGB8")]
    Rgb8,
    #[serde(rename = "RGBA8")]
    Rgba8,
    #[serde(rename = "NODATA")]
    NoData,
    #[serde(rename = "DXT3")]
    Dxt3,
    #[serde(rename = "DXT5")]
    Dxt5,
    #[serde(rename = "L8")]
    L8,
    #[serde(rename = "G16")]
    G16,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl UtxFormat {
    fn from_value(value: u8) -> Self {
        match value {
            0 => Self::P8,
            1 => Self::Rgba7,
            2 => Self::Rgb16,
            3 => Self::Dxt1,
            4 => Self::Rgb8,
            5 => Self::Rgba8,
            6 => Self::NoData,
            7 => Self::Dxt3,
            8 => Self::Dxt5,
            9 => Self::L8,
            10 => Self::G16,
            _ => Self::Unknown,
        }
    }

    fn is_dxt(self) -> bool {
        matches!(self, Self::Dxt1 | Self::Dxt3 | Self::Dxt5)
    }

    fn is_previewable(self) -> bool {
        self == Self::Rgba8 || self.is_dxt()
    }

    fn value(self) -> Option<u8> {
        match self {
            Self::P8 => Some(0),
            Self::Rgba7 => Some(1),
            Self::Rgb16 => Some(2),
            Self::Dxt1 => Some(3),
            Self::Rgb8 => Some(4),
            Self::Rgba8 => Some(5),
            Self::NoData => Some(6),
            Self::Dxt3 => Some(7),
            Self::Dxt5 => Some(8),
            Self::L8 => Some(9),
            Self::G16 => Some(10),
            Self::Unknown => None,
        }
    }

    fn export_extension(self) -> &'static str {
        if self == Self::Rgba8 {
            "tga"
        } else if self.is_dxt() {
            "dds"
        } else {
            "bin"
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxEntry {
    pub name: String,
    pub format: UtxFormat,
    pub export_index: usize,
    pub width: i32,
    pub height: i32,
    pub has_alpha: bool,
    pub has_split9: bool,
    pub split9_x1: i32,
    pub split9_x2: i32,
    pub split9_x3: i32,
    pub split9_y1: i32,
    pub split9_y2: i32,
    pub split9_y3: i32,
    #[serde(skip)]
    settings: TextureSettings,
    #[serde(skip)]
    animation: Option<ExportedAnimation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub exported: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub completed: usize,
    pub total: usize,
    pub file_name: String,
}

/// Output representation selected by the standalone UTX extractor.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UtxExtractMode {
    Original,
    Png,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxExtractSummary {
    pub packages: usize,
    pub exported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub errors: Vec<String>,
    pub output_directory: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxExtractProgress {
    pub completed: usize,
    pub total: usize,
    pub package_name: String,
    pub file_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureImportSummary {
    pub replaced: usize,
    pub added: usize,
    pub skipped: usize,
    pub failed: usize,
    pub errors: Vec<String>,
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureImportProgress {
    pub completed: usize,
    pub total: usize,
    pub phase: String,
    pub file_name: String,
}

struct ImportProgressThrottle<'a, F> {
    report: &'a mut F,
    last_completed: usize,
    last_reported_at: Instant,
}

impl<F> ImportProgressThrottle<'_, F>
where
    F: FnMut(TextureImportProgress),
{
    fn new(report: &mut F) -> ImportProgressThrottle<'_, F> {
        ImportProgressThrottle {
            report,
            last_completed: 0,
            last_reported_at: Instant::now() - IMPORT_PROGRESS_INTERVAL,
        }
    }

    fn report(&mut self, progress: TextureImportProgress) {
        let should_report = progress.completed == 0
            || progress.completed >= progress.total
            || progress.file_name.is_empty()
            || progress.completed.saturating_sub(self.last_completed) >= IMPORT_PROGRESS_STEP
            || self.last_reported_at.elapsed() >= IMPORT_PROGRESS_INTERVAL;
        if should_report {
            self.last_completed = progress.completed;
            self.last_reported_at = Instant::now();
            (self.report)(progress);
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementRequest {
    pub export_index: usize,
    pub replacement_path: String,
}

struct ImportedTexture {
    name: String,
    format: UtxFormat,
    width: i32,
    height: i32,
    pixels: Vec<u8>,
    metadata: TextureMetadata,
}

struct PendingTextureImport {
    file_path: String,
    file_name: String,
    is_existing: bool,
    export_index: Option<usize>,
}

struct AppliedTextureImport {
    file_name: String,
}

struct RetriedTextureImport {
    file_path: String,
    file_name: String,
    export_index: usize,
    request: texture_engine::TextureImportRequest,
}

struct ResilientTextureImport {
    data: Vec<u8>,
    applied: Vec<AppliedTextureImport>,
    failures: Vec<(String, String)>,
}

struct ImportDebugLog {
    path: Option<PathBuf>,
    file: Option<fs::File>,
}

impl ImportDebugLog {
    fn start(package_path: &Path, source: &str) -> Self {
        let directory = std::env::temp_dir().join("unreal-tools");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let path = directory.join(format!("utx-import-{}-{stamp}.log", std::process::id()));
        let file = fs::create_dir_all(&directory)
            .and_then(|_| {
                fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
            })
            .ok();
        let mut log = Self {
            path: file.as_ref().map(|_| path),
            file,
        };
        log.write("Unreal Tools - UTX import diagnostic log");
        log.write(&format!("Package: {}", package_path.display()));
        log.write(&format!("Source: {source}"));
        log.write("---");
        log
    }

    fn write(&mut self, message: &str) {
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{message}");
        }
    }

    fn error(&mut self, scope: &str, file_path: &str, error: &str) {
        self.write(&format!("ERROR | {scope} | {file_path} | {error}"));
    }

    fn finish(&mut self, summary: &TextureImportSummary) {
        self.write("---");
        self.write(&format!(
            "Summary: {} replaced, {} added, {} skipped, {} failed.",
            summary.replaced, summary.added, summary.skipped, summary.failed
        ));
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
    }

    fn path_string(&self) -> Option<String> {
        self.path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
    }
}

fn import_existing_textures_resilient(
    package_data: Vec<u8>,
    package_name: &str,
    imports: &[RetriedTextureImport],
    log: &mut ImportDebugLog,
) -> ResilientTextureImport {
    if imports.is_empty() {
        return ResilientTextureImport {
            data: package_data,
            applied: Vec::new(),
            failures: Vec::new(),
        };
    }

    let original_data = package_data.clone();
    let requests = imports
        .iter()
        .map(|import| (import.export_index, import.request.clone()))
        .collect::<Vec<_>>();
    match texture_engine::replace_existing_textures(package_data, &requests) {
        Ok((data, outcomes)) => ResilientTextureImport {
            data,
            applied: imports
                .iter()
                .zip(outcomes)
                .map(|(import, _)| AppliedTextureImport {
                    file_name: import.file_name.clone(),
                })
                .collect(),
            failures: Vec::new(),
        },
        Err(error) if imports.len() == 1 => {
            let import = &imports[0];
            log.error("UTX engine", &import.file_path, &error);
            ResilientTextureImport {
                data: original_data,
                applied: Vec::new(),
                failures: vec![(import.file_name.clone(), error)],
            }
        }
        Err(error) => {
            log.write(&format!(
                "ENGINE BATCH ERROR | {package_name} | {} texture(s) | {error} | isolating the failing texture(s)",
                imports.len()
            ));
            let middle = imports.len() / 2;
            let mut left = import_existing_textures_resilient(
                original_data,
                package_name,
                &imports[..middle],
                log,
            );
            let right = import_existing_textures_resilient(
                left.data.clone(),
                package_name,
                &imports[middle..],
                log,
            );
            left.data = right.data;
            left.applied.extend(right.applied);
            left.failures.extend(right.failures);
            left
        }
    }
}

fn texture_engine_import_request(
    texture: ImportedTexture,
) -> UtxResult<texture_engine::TextureImportRequest> {
    let format = texture
        .format
        .value()
        .ok_or("Formato de textura inválido para importação.")?;
    let split9 = texture
        .metadata
        .split9
        .map(|split9| texture_engine::Split9Edit {
            enabled: true,
            x1: split9.x1,
            x2: split9.x2,
            x3: split9.x3,
            y1: split9.y1,
            y2: split9.y2,
            y3: split9.y3,
        });
    let animation =
        texture
            .metadata
            .animation
            .map(|animation| texture_engine::TextureAnimationImport {
                anim_next: animation.anim_next,
                max_frame_rate: animation.max_frame_rate,
                min_frame_rate: animation.min_frame_rate,
                one_time_anim_loop: animation.one_time_anim_loop,
                prime_count: animation.prime_count,
                total_frame_num: animation.total_frame_num,
            });
    Ok(texture_engine::TextureImportRequest {
        name: texture.name,
        format,
        width: texture.width,
        height: texture.height,
        pixels: texture.pixels,
        alpha: texture.metadata.settings.alpha,
        masked: texture.metadata.settings.masked,
        clamp: texture.metadata.settings.clamp_edit(),
        split9,
        animation,
    })
}

/// Existing game UTX files are not uniform: optional UE2 properties such as
/// `UClampMode` may be absent from one texture and serialized in another. A
/// replacement must never grow the property's byte stream, because that would
/// move the mip data and invalidate its serial size. When the destination does
/// not contain an optional property, retain its current value and replace the
/// pixels normally instead of turning the entire batch into a failed rewrite.
fn preserve_unsupported_existing_metadata(
    texture: &mut ImportedTexture,
    existing: &UtxEntry,
) -> Vec<&'static str> {
    let mut preserved = Vec::new();
    let settings = &mut texture.metadata.settings;

    if settings.alpha.is_some() && existing.settings.alpha.is_none() {
        settings.alpha = None;
        preserved.push("Alpha");
    }
    if settings.masked.is_some() && existing.settings.masked.is_none() {
        settings.masked = None;
        preserved.push("Masked");
    }
    if settings.u_clamp.is_some() && existing.settings.u_clamp.is_none() {
        settings.u_clamp = None;
        preserved.push("UClamp");
    }
    if settings.v_clamp.is_some() && existing.settings.v_clamp.is_none() {
        settings.v_clamp = None;
        preserved.push("VClamp");
    }
    if settings.u_clamp_mode.is_some() && existing.settings.u_clamp_mode.is_none() {
        settings.u_clamp_mode = None;
        preserved.push("UClampMode");
    }
    if settings.v_clamp_mode.is_some() && existing.settings.v_clamp_mode.is_none() {
        settings.v_clamp_mode = None;
        preserved.push("VClampMode");
    }
    if texture.metadata.split9.is_some() && !existing.has_split9 {
        texture.metadata.split9 = None;
        preserved.push("Split9");
    }
    if let Some(animation) = texture.metadata.animation.as_mut() {
        if let Some(existing_animation) = existing.animation.as_ref() {
            let available = existing_animation.properties;
            if animation.anim_next.is_some() && !available.anim_next {
                animation.anim_next = None;
                preserved.push("AnimNext");
            }
            if animation.max_frame_rate.is_some() && !available.max_frame_rate {
                animation.max_frame_rate = None;
                preserved.push("MaxFrameRate");
            }
            if animation.min_frame_rate.is_some() && !available.min_frame_rate {
                animation.min_frame_rate = None;
                preserved.push("MinFrameRate");
            }
            if animation.one_time_anim_loop.is_some() && !available.one_time_anim_loop {
                animation.one_time_anim_loop = None;
                preserved.push("OneTimeAnimLoop");
            }
            if animation.prime_count.is_some() && !available.prime_count {
                animation.prime_count = None;
                preserved.push("PrimeCount");
            }
            if animation.total_frame_num.is_some() && !available.total_frame_num {
                animation.total_frame_num = None;
                preserved.push("TotalFrameNum");
            }
        } else {
            texture.metadata.animation = None;
            preserved.push("Animations");
        }
    }
    if texture
        .metadata
        .animation
        .as_ref()
        .is_some_and(|animation| !animation.has_values())
    {
        texture.metadata.animation = None;
    }

    preserved
}

#[derive(Debug, Clone, Copy, Default)]
struct AnimationValues {
    anim_next: i32,
    max_frame_rate: f32,
    min_frame_rate: f32,
    one_time_anim_loop: bool,
    prime_count: i32,
    total_frame_num: i32,
}

#[derive(Debug, Clone, Copy, Default)]
struct AnimationPropertyPresence {
    anim_next: bool,
    max_frame_rate: bool,
    min_frame_rate: bool,
    one_time_anim_loop: bool,
    prime_count: bool,
    total_frame_num: bool,
}

impl AnimationValues {
    fn is_active(self) -> bool {
        self.anim_next != 0
            || self.max_frame_rate != 0.0
            || self.min_frame_rate != 0.0
            || self.one_time_anim_loop
            || self.prime_count != 0
            || self.total_frame_num != 0
    }
}

#[derive(Debug, Clone)]
struct ExportedAnimation {
    anim_next: Option<String>,
    values: AnimationValues,
    properties: AnimationPropertyPresence,
}

#[derive(Clone, Default)]
struct ImportedAnimation {
    anim_next: Option<String>,
    max_frame_rate: Option<f32>,
    min_frame_rate: Option<f32>,
    one_time_anim_loop: Option<bool>,
    prime_count: Option<i32>,
    total_frame_num: Option<i32>,
}

impl ImportedAnimation {
    fn has_values(&self) -> bool {
        self.anim_next.is_some()
            || self.max_frame_rate.is_some()
            || self.min_frame_rate.is_some()
            || self.one_time_anim_loop.is_some()
            || self.prime_count.is_some()
            || self.total_frame_num.is_some()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct TextureSettings {
    alpha: Option<bool>,
    masked: Option<bool>,
    u_clamp: Option<i32>,
    v_clamp: Option<i32>,
    u_clamp_mode: Option<i32>,
    v_clamp_mode: Option<i32>,
}

impl TextureSettings {
    fn has_values(self) -> bool {
        self.alpha.is_some()
            || self.masked.is_some()
            || self.u_clamp.is_some()
            || self.v_clamp.is_some()
            || self.u_clamp_mode.is_some()
            || self.v_clamp_mode.is_some()
    }

    fn clamp_edit(self) -> Option<texture_engine::TextureClampEdit> {
        (self.u_clamp.is_some()
            || self.v_clamp.is_some()
            || self.u_clamp_mode.is_some()
            || self.v_clamp_mode.is_some())
        .then_some(texture_engine::TextureClampEdit {
            u_clamp: self.u_clamp,
            v_clamp: self.v_clamp,
            u_clamp_mode: self.u_clamp_mode,
            v_clamp_mode: self.v_clamp_mode,
        })
    }
}

#[derive(Clone, Default)]
struct TextureMetadata {
    settings: TextureSettings,
    split9: Option<Split9>,
    animation: Option<ImportedAnimation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TexturePreview {
    pub data_url: String,
    pub width: i32,
    pub height: i32,
}

/// A single decrypted UTX session. The desktop app keeps this object alive
/// while its package is open, so previews and navigation never decrypt the
/// full package again.
pub struct CachedUtx {
    file_path: String,
    package: Package,
    encrypted: bool,
    xor_key: u8,
    header: Vec<u8>,
}

impl CachedUtx {
    pub fn matches_path(&self, file_path: &str) -> bool {
        Path::new(&self.file_path) == Path::new(file_path)
    }

    fn encrypted_bytes(&self, working: &[u8]) -> Vec<u8> {
        if !self.encrypted {
            return working.to_vec();
        }
        let mut output = self.header.clone();
        output.extend(working.iter().map(|byte| byte ^ self.xor_key));
        output
    }
}

pub fn open_cached(file_path: &str) -> UtxResult<(CachedUtx, Vec<UtxEntry>)> {
    let decrypted = decrypt_raw(Path::new(file_path))?;
    let package = Package::parse(decrypted.working)?;
    let entries = package.scan_entries()?;
    Ok((
        CachedUtx {
            file_path: file_path.to_owned(),
            package,
            encrypted: decrypted.encrypted,
            xor_key: decrypted.xor_key,
            header: decrypted.header,
        },
        entries,
    ))
}

pub fn cached_list_entries(cache: &CachedUtx) -> UtxResult<Vec<UtxEntry>> {
    cache.package.scan_entries()
}

pub fn cached_export_entry(
    cache: &CachedUtx,
    export_index: usize,
    output_path: &str,
) -> UtxResult<()> {
    let entry = cache.package.entry_to_model(export_index)?;
    validate_export_extension(Path::new(output_path), entry.format)?;
    fs::write(output_path, cache.package.export_bytes(&entry)?)
        .map_err(|error| format!("Não foi possível exportar a textura: {error}"))?;
    write_texture_metadata_file(&entry, Path::new(output_path))
}

pub fn cached_export_entries(
    cache: &CachedUtx,
    export_indices: Vec<usize>,
    output_dir: &str,
) -> UtxResult<ExportSummary> {
    cached_export_entries_with_progress(cache, export_indices, output_dir, |_| {})
}

pub fn cached_export_entries_with_progress<F>(
    cache: &CachedUtx,
    export_indices: Vec<usize>,
    output_dir: &str,
    mut report_progress: F,
) -> UtxResult<ExportSummary>
where
    F: FnMut(ExportProgress),
{
    let output_dir = Path::new(output_dir);
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("Não foi possível criar a pasta de destino: {error}"))?;
    let mut exported = 0;
    let mut failed = 0;
    let total = export_indices.len();
    for (position, export_index) in export_indices.into_iter().enumerate() {
        let entry = cache.package.entry_to_model(export_index);
        let file_name = entry
            .as_ref()
            .map(|entry| file_name_for_export(&entry.name))
            .unwrap_or_else(|_| format!("Textura #{}", export_index + 1));
        report_progress(ExportProgress {
            completed: position,
            total,
            file_name: file_name.clone(),
        });
        let result = match entry {
            Ok(entry) => (|| -> UtxResult<()> {
                let package_folder = package_prefix(&entry.name);
                let destination_dir = package_folder
                    .map_or_else(|| output_dir.to_path_buf(), |name| output_dir.join(name));
                fs::create_dir_all(&destination_dir).map_err(|error| {
                    format!("Não foi possível criar a pasta do pacote: {error}")
                })?;
                let destination = destination_dir.join(format!(
                    "{}.{}",
                    file_name_for_export(&entry.name),
                    entry.format.export_extension()
                ));
                fs::write(&destination, cache.package.export_bytes(&entry)?)
                    .map_err(|error| format!("Erro de escrita: {error}"))?;
                write_texture_metadata_file(&entry, &destination)
            })(),
            Err(error) => Err(error),
        };
        if result.is_ok() {
            exported += 1;
        } else {
            failed += 1;
        }
        report_progress(ExportProgress {
            completed: position + 1,
            total,
            file_name,
        });
    }
    Ok(ExportSummary { exported, failed })
}

pub fn cached_preview_texture(cache: &CachedUtx, export_index: usize) -> UtxResult<TexturePreview> {
    let entry = cache.package.entry_to_model(export_index)?;
    if !entry.format.is_previewable() {
        return Err(format!(
            "A pré-visualização não é suportada para {}.",
            format_label(entry.format)
        ));
    }
    encode_preview(
        &cache.package.mip0_pixels(export_index)?,
        entry.width,
        entry.height,
        entry.format,
    )
}

/// Reads the UE2 properties exposed by the texture editor dialog.
pub fn cached_texture_properties(
    cache: &CachedUtx,
    export_index: usize,
) -> UtxResult<texture_engine::TextureEditorState> {
    texture_engine::texture_editor_state(cache.package.data.clone(), export_index)
}

/// Persists editor property changes through the native texture engine.
pub fn cached_update_texture_properties(
    cache: &mut CachedUtx,
    export_index: usize,
    edit: texture_engine::TextureEditorEdit,
) -> UtxResult<()> {
    let working =
        texture_engine::edit_texture_properties(cache.package.data.clone(), export_index, edit)?;
    fs::write(&cache.file_path, cache.encrypted_bytes(&working))
        .map_err(|error| format!("Não foi possível gravar as propriedades da textura: {error}"))?;
    cache.package = Package::parse(working)?;
    Ok(())
}

/// Persists several property edits in one package rewrite, which keeps batch
/// operations atomic from the UI perspective and preserves encryption.
pub fn cached_update_texture_properties_batch(
    cache: &mut CachedUtx,
    edits: &[(usize, texture_engine::TextureEditorEdit)],
) -> UtxResult<()> {
    let working = texture_engine::edit_texture_properties_batch(cache.package.data.clone(), edits)?;
    fs::write(&cache.file_path, cache.encrypted_bytes(&working)).map_err(|error| {
        format!("Não foi possível gravar as propriedades das texturas: {error}")
    })?;
    cache.package = Package::parse(working)?;
    Ok(())
}

/// Duplicates a texture export while keeping the UTX cache and its original
/// encryption in sync with the file on disk.
pub fn cached_duplicate_texture(
    cache: &mut CachedUtx,
    source_export_index: usize,
    group_name: &str,
    texture_name: &str,
) -> UtxResult<usize> {
    let (working, export_index) = texture_engine::duplicate_texture(
        cache.package.data.clone(),
        source_export_index,
        group_name,
        texture_name,
    )?;
    fs::write(&cache.file_path, cache.encrypted_bytes(&working))
        .map_err(|error| format!("Não foi possível duplicar a textura: {error}"))?;
    cache.package = Package::parse(working)?;
    Ok(export_index)
}

/// Renames a texture export while preserving the cached package and its
/// original encryption on disk.
pub fn cached_rename_texture(
    cache: &mut CachedUtx,
    export_index: usize,
    texture_name: &str,
) -> UtxResult<()> {
    let working =
        texture_engine::rename_texture(cache.package.data.clone(), export_index, texture_name)?;
    fs::write(&cache.file_path, cache.encrypted_bytes(&working))
        .map_err(|error| format!("Não foi possível renomear a textura: {error}"))?;
    cache.package = Package::parse(working)?;
    Ok(())
}

pub fn cached_replace_entry(
    cache: &mut CachedUtx,
    export_index: usize,
    replacement_path: &str,
) -> UtxResult<()> {
    let replacement_path = Path::new(replacement_path);
    let request = texture_engine_import_request(read_imported_texture(replacement_path)?)?;
    let working =
        texture_engine::replace_texture(cache.package.data.clone(), export_index, &request)?;
    fs::write(&cache.file_path, cache.encrypted_bytes(&working))
        .map_err(|error| format!("Não foi possível gravar o pacote UTX: {error}"))?;
    cache.package = Package::parse(working)?;
    Ok(())
}

pub fn cached_import_entries(
    cache: &mut CachedUtx,
    replacements: Vec<ReplacementRequest>,
) -> UtxResult<ImportSummary> {
    if replacements.is_empty() {
        return Ok(ImportSummary {
            imported: 0,
            skipped: 0,
            failed: 0,
        });
    }
    let mut working = cache.package.data.clone();
    let mut imported = 0;
    let mut skipped = 0;
    let mut failed = 0;
    for request in replacements {
        let replacement_path = Path::new(&request.replacement_path);
        match read_imported_texture(replacement_path)
            .and_then(texture_engine_import_request)
            .and_then(|texture| {
                texture_engine::replace_texture(working.clone(), request.export_index, &texture)
            }) {
            Ok(updated) => {
                working = updated;
                imported += 1;
            }
            Err(error) if error.contains("Não foi possível ler") => failed += 1,
            Err(_) => skipped += 1,
        }
    }
    if imported > 0 {
        fs::write(&cache.file_path, cache.encrypted_bytes(&working))
            .map_err(|error| format!("Não foi possível gravar o pacote UTX: {error}"))?;
        cache.package = Package::parse(working)?;
    }
    Ok(ImportSummary {
        imported,
        skipped,
        failed,
    })
}

/// Imports texture files into one package namespace. Existing names replace
/// their pixels; names that do not exist become new Engine.Texture exports.
/// Image preparation runs in parallel while the package itself is updated as
/// one ordered write because its export tables and byte offsets are shared by
/// every texture added to it.
///
/// The caller receives a progress update for every file once it has been
/// accepted, skipped, or rejected.
pub fn cached_import_textures_with_progress<F>(
    cache: &mut CachedUtx,
    package_name: &str,
    texture_paths: Vec<String>,
    report_progress: F,
) -> UtxResult<TextureImportSummary>
where
    F: FnMut(TextureImportProgress),
{
    let mut log = ImportDebugLog::start(Path::new(&cache.file_path), "selected texture files");
    let result = cached_import_textures_with_progress_and_commit(
        cache,
        package_name,
        texture_paths,
        true,
        report_progress,
        &mut log,
    );
    match result {
        Ok(mut summary) => {
            summary.log_path = log.path_string();
            log.finish(&summary);
            Ok(summary)
        }
        Err(error) => {
            log.error("import", "<operation>", &error);
            log.write("Import aborted before a summary could be produced.");
            Err(error)
        }
    }
}

fn cached_import_textures_with_progress_and_commit<F>(
    cache: &mut CachedUtx,
    package_name: &str,
    texture_paths: Vec<String>,
    commit_to_disk: bool,
    mut report_progress: F,
    log: &mut ImportDebugLog,
) -> UtxResult<TextureImportSummary>
where
    F: FnMut(TextureImportProgress),
{
    let package_name = package_name.trim();
    if package_name.is_empty() {
        return Err("Selecione o pacote de destino antes de importar texturas.".into());
    }
    validate_texture_group_name(package_name)?;
    if texture_paths.is_empty() {
        return Ok(TextureImportSummary {
            replaced: 0,
            added: 0,
            skipped: 0,
            failed: 0,
            errors: Vec::new(),
            log_path: None,
        });
    }

    let total = texture_paths.len();
    let mut progress_reporter = ImportProgressThrottle::new(&mut report_progress);
    progress_reporter.report(TextureImportProgress {
        completed: 0,
        total,
        phase: "Preparando texturas…".into(),
        file_name: String::new(),
    });
    let prepared = read_imported_textures_parallel(texture_paths)?;
    let mut import_requests = Vec::new();
    let mut imported_files = Vec::new();
    let mut replaced = 0;
    let mut added = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut errors = Vec::new();
    let mut completed = 0;
    let mut selected_names = HashSet::new();
    let existing_textures = cache.package.texture_import_index()?;

    for (texture_path, prepared_texture) in prepared {
        let file_name = display_file_name(&texture_path);
        let mut texture = match prepared_texture {
            Ok(texture) => texture,
            Err(error) => {
                failed += 1;
                log.error("texture preparation", &texture_path, &error);
                if errors.len() < MAX_TEXTURE_IMPORT_ERRORS {
                    errors.push(format!("{file_name}: {error}"));
                }
                completed += 1;
                progress_reporter.report(TextureImportProgress {
                    completed,
                    total,
                    phase: format!("Arquivo recusado: {error}"),
                    file_name,
                });
                continue;
            }
        };

        let existing = existing_textures.get(&texture_import_key(package_name, &texture.name));
        if !selected_names.insert(texture.name.to_ascii_lowercase()) {
            skipped += 1;
            log.write(&format!(
                "SKIPPED | {texture_path} | duplicate texture name '{}' in the selected import batch.",
                texture.name
            ));
            completed += 1;
            progress_reporter.report(TextureImportProgress {
                completed,
                total,
                phase: "Nome duplicado ignorado.".into(),
                file_name,
            });
            continue;
        }
        let export_index = existing.as_ref().map(|entry| entry.export_index);
        let is_existing = export_index.is_some();
        if !is_existing && !texture.name.is_ascii() {
            failed += 1;
            let error = "Uma textura nova precisa de um nome ASCII; nomes Unicode só podem substituir uma textura já existente.";
            log.error("texture name", &texture_path, error);
            if errors.len() < MAX_TEXTURE_IMPORT_ERRORS {
                errors.push(format!("{file_name}: {error}"));
            }
            completed += 1;
            progress_reporter.report(TextureImportProgress {
                completed,
                total,
                phase: "Nome Unicode não pode criar uma nova textura.".into(),
                file_name,
            });
            continue;
        }
        if let Some(existing) = existing {
            if existing.format != texture.format
                || existing.width != texture.width
                || existing.height != texture.height
            {
                skipped += 1;
                log.write(&format!(
                    "SKIPPED | {texture_path} | incompatible replacement for '{}': source {} {}x{}, destination {} {}x{}.",
                    existing.name,
                    format_label(texture.format),
                    texture.width,
                    texture.height,
                    format_label(existing.format),
                    existing.width,
                    existing.height,
                ));
                completed += 1;
                progress_reporter.report(TextureImportProgress {
                    completed,
                    total,
                    phase: "Formato ou dimensões incompatíveis; ignorada.".into(),
                    file_name,
                });
                continue;
            }
            let preserved = preserve_unsupported_existing_metadata(&mut texture, existing);
            if !preserved.is_empty() {
                log.write(&format!(
                    "METADATA PRESERVED | {texture_path} | {} | the destination does not serialize these optional properties; pixels were replaced without changing the package layout.",
                    preserved.join(", ")
                ));
            }
        }
        match texture_engine_import_request(texture) {
            Ok(request) => {
                import_requests.push(request);
                imported_files.push(PendingTextureImport {
                    file_path: texture_path,
                    file_name,
                    is_existing,
                    export_index,
                });
            }
            Err(error) => {
                failed += 1;
                log.error("texture metadata", &texture_path, &error);
                if errors.len() < MAX_TEXTURE_IMPORT_ERRORS {
                    errors.push(format!("{file_name}: {error}"));
                }
                completed += 1;
                progress_reporter.report(TextureImportProgress {
                    completed,
                    total,
                    phase: format!("Textura incompatível: {error}"),
                    file_name,
                });
            }
        }
    }

    if !import_requests.is_empty() {
        progress_reporter.report(TextureImportProgress {
            completed,
            total,
            phase: "Gravando texturas no motor UTX…".into(),
            file_name: String::new(),
        });

        let retried_existing = imported_files
            .iter()
            .zip(&import_requests)
            .filter_map(|(pending, request)| {
                pending
                    .export_index
                    .map(|export_index| RetriedTextureImport {
                        file_path: pending.file_path.clone(),
                        file_name: pending.file_name.clone(),
                        export_index,
                        request: request.clone(),
                    })
            })
            .collect::<Vec<_>>();
        let recovered = import_existing_textures_resilient(
            cache.package.data.clone(),
            package_name,
            &retried_existing,
            log,
        );
        let mut working = recovered.data;

        for applied_import in recovered.applied {
            completed += 1;
            replaced += 1;
            progress_reporter.report(TextureImportProgress {
                completed,
                total,
                phase: "Textura substituída pelo motor UTX.".into(),
                file_name: applied_import.file_name,
            });
        }
        for (file_name, error) in recovered.failures {
            skipped += 1;
            if errors.len() < MAX_TEXTURE_IMPORT_ERRORS {
                errors.push(format!("{file_name}: {error}"));
            }
            completed += 1;
            progress_reporter.report(TextureImportProgress {
                completed,
                total,
                phase: format!("Textura problemática ignorada: {error}"),
                file_name,
            });
        }

        let additions = imported_files
            .iter()
            .zip(&import_requests)
            .filter(|(pending, _)| !pending.is_existing)
            .collect::<Vec<_>>();
        if !additions.is_empty() {
            let addition_requests = additions
                .iter()
                .map(|(_, request)| (*request).clone())
                .collect::<Vec<_>>();
            match texture_engine::import_new_textures(
                working.clone(),
                NEW_UTX_TEMPLATE,
                package_name,
                &addition_requests,
            ) {
                Ok((updated, outcomes)) => {
                    working = updated;
                    for ((pending, _), outcome) in additions.into_iter().zip(outcomes) {
                        completed += 1;
                        if outcome.added {
                            added += 1;
                            progress_reporter.report(TextureImportProgress {
                                completed,
                                total,
                                phase: "Textura adicionada pelo motor UTX.".into(),
                                file_name: pending.file_name.clone(),
                            });
                        } else {
                            replaced += 1;
                            progress_reporter.report(TextureImportProgress {
                                completed,
                                total,
                                phase: "Textura substituída pelo motor UTX.".into(),
                                file_name: pending.file_name.clone(),
                            });
                        }
                    }
                }
                Err(error) => {
                    log.write(&format!(
                        "ENGINE ADDITIONS ERROR | {package_name} | {} texture(s) | {error}",
                        additions.len()
                    ));
                    for (pending, _) in additions {
                        skipped += 1;
                        log.error("UTX engine new texture", &pending.file_path, &error);
                        if errors.len() < MAX_TEXTURE_IMPORT_ERRORS {
                            errors.push(format!("{}: {error}", pending.file_name));
                        }
                        completed += 1;
                        progress_reporter.report(TextureImportProgress {
                            completed,
                            total,
                            phase: format!("Nova textura ignorada: {error}"),
                            file_name: pending.file_name.clone(),
                        });
                    }
                }
            }
        }

        if replaced > 0 || added > 0 {
            cache.package = Package::parse(working)?;
            if commit_to_disk {
                fs::write(&cache.file_path, cache.encrypted_bytes(&cache.package.data)).map_err(
                    |write_error| format!("Não foi possível gravar o pacote UTX: {write_error}"),
                )?;
            }
        }
    }

    Ok(TextureImportSummary {
        replaced,
        added,
        skipped,
        failed,
        errors,
        log_path: None,
    })
}

/// Imports an exported folder tree without requiring the user to recreate its
/// groups manually. Textures in the selected root stay ungrouped, while every
/// immediate child folder becomes the destination group for all textures below
/// it. This mirrors the directory layout produced by the bulk exporter.
pub fn cached_import_texture_directory_with_progress<F>(
    cache: &mut CachedUtx,
    directory: &str,
    mut report_progress: F,
) -> UtxResult<TextureImportSummary>
where
    F: FnMut(TextureImportProgress),
{
    let mut log = ImportDebugLog::start(Path::new(&cache.file_path), directory);
    let groups = match collect_texture_import_groups(Path::new(directory)) {
        Ok(groups) => groups,
        Err(error) => {
            log.error("directory discovery", directory, &error);
            return Err(error);
        }
    };
    let total = groups.iter().map(|(_, files)| files.len()).sum::<usize>();
    let mut summary = TextureImportSummary {
        replaced: 0,
        added: 0,
        skipped: 0,
        failed: 0,
        errors: Vec::new(),
        log_path: None,
    };
    let mut completed_before_group = 0;

    for (group_name, texture_paths) in groups {
        let group_total = texture_paths.len();
        let group_label = group_name.clone();
        match cached_import_textures_with_progress_and_commit(
            cache,
            &group_name,
            texture_paths,
            false,
            |mut progress| {
                progress.completed += completed_before_group;
                progress.total = total;
                progress.phase = format!("{group_label} · {}", progress.phase);
                report_progress(progress);
            },
            &mut log,
        ) {
            Ok(group_summary) => {
                summary.replaced += group_summary.replaced;
                summary.added += group_summary.added;
                summary.skipped += group_summary.skipped;
                summary.failed += group_summary.failed;
                for error in group_summary.errors {
                    if summary.errors.len() >= MAX_TEXTURE_IMPORT_ERRORS {
                        break;
                    }
                    summary.errors.push(format!("{group_name}: {error}"));
                }
            }
            Err(error) => {
                summary.failed += group_total;
                log.error("group import", &group_name, &error);
                if summary.errors.len() < MAX_TEXTURE_IMPORT_ERRORS {
                    summary.errors.push(format!("{group_name}: {error}"));
                }
                report_progress(TextureImportProgress {
                    completed: completed_before_group + group_total,
                    total,
                    phase: format!("{group_name} não pôde ser importado: {error}"),
                    file_name: String::new(),
                });
            }
        }
        completed_before_group += group_total;
    }

    if summary.replaced > 0 || summary.added > 0 {
        if let Err(error) = fs::write(&cache.file_path, cache.encrypted_bytes(&cache.package.data))
        {
            let message = format!("Não foi possível gravar o pacote UTX: {error}");
            log.error("package write", &cache.file_path, &message);
            return Err(message);
        }
    }

    summary.log_path = log.path_string();
    log.finish(&summary);
    Ok(summary)
}

fn collect_texture_import_groups(root: &Path) -> UtxResult<Vec<(String, Vec<String>)>> {
    if !root.is_dir() {
        return Err("A pasta selecionada para importação não existe ou não é acessível.".into());
    }

    let mut root_files = Vec::new();
    let mut child_directories = Vec::new();
    for entry in read_directory_entries(root)? {
        let path = entry.path();
        if path.is_file() && is_importable_texture_path(&path) {
            root_files.push(path.to_string_lossy().into_owned());
        } else if path.is_dir() {
            child_directories.push(path);
        }
    }
    root_files.sort();

    let mut groups = Vec::new();
    if !root_files.is_empty() {
        groups.push(("Pacote principal".into(), root_files));
    }
    for directory in child_directories {
        let group_name = directory
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or("Não foi possível determinar o nome de um grupo de importação.")?
            .to_owned();
        let mut files = collect_textures_recursively(&directory)?;
        files.sort();
        if !files.is_empty() {
            groups.push((group_name, files));
        }
    }
    if groups.is_empty() {
        return Err("A pasta selecionada não contém arquivos .tga ou .dds.".into());
    }
    Ok(groups)
}

fn collect_textures_recursively(directory: &Path) -> UtxResult<Vec<String>> {
    let mut files = Vec::new();
    for entry in read_directory_entries(directory)? {
        let path = entry.path();
        if path.is_file() && is_importable_texture_path(&path) {
            files.push(path.to_string_lossy().into_owned());
        } else if path.is_dir() {
            files.extend(collect_textures_recursively(&path)?);
        }
    }
    Ok(files)
}

fn read_directory_entries(directory: &Path) -> UtxResult<Vec<fs::DirEntry>> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("Não foi possível ler a pasta de importação: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Não foi possível ler um item da pasta de importação: {error}"))?;
    let mut entries = entries;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn is_importable_texture_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("tga") || extension.eq_ignore_ascii_case("dds")
        })
}

fn read_imported_textures_parallel(
    texture_paths: Vec<String>,
) -> UtxResult<Vec<(String, UtxResult<ImportedTexture>)>> {
    let total = texture_paths.len();
    let worker_count = thread::available_parallelism()
        .map(|available| available.get())
        .unwrap_or(1)
        .min(total)
        .min(8);
    if worker_count <= 1 {
        return Ok(texture_paths
            .into_iter()
            .map(|path| {
                let result = read_imported_texture(Path::new(&path));
                (path, result)
            })
            .collect());
    }

    let queue = Arc::new(Mutex::new(
        texture_paths
            .into_iter()
            .enumerate()
            .collect::<VecDeque<_>>(),
    ));
    let (sender, receiver) = mpsc::channel();
    let mut results = Vec::with_capacity(total);
    results.resize_with(total, || None);

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let sender = sender.clone();
            scope.spawn(move || loop {
                let Some((index, path)) = queue.lock().ok().and_then(|mut queue| queue.pop_front())
                else {
                    break;
                };
                let result = read_imported_texture(Path::new(&path));
                if sender.send((index, path, result)).is_err() {
                    break;
                }
            });
        }
        drop(sender);
        for (index, path, result) in receiver {
            if let Some(slot) = results.get_mut(index) {
                *slot = Some((path, result));
            }
        }
    });

    let mut ordered = Vec::with_capacity(total);
    for (index, result) in results.into_iter().enumerate() {
        ordered.push(result.ok_or_else(|| {
            format!(
                "A preparação paralela da textura {} não foi concluída.",
                index + 1
            )
        })?);
    }
    Ok(ordered)
}

fn display_file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_owned()
}

pub fn list_entries(file_path: &str) -> UtxResult<Vec<UtxEntry>> {
    let decrypted = decrypt_raw(Path::new(file_path))?;
    Package::parse(decrypted.working)?.scan_entries()
}

/// Creates a new, unencrypted UTX from the Unreal Editor-generated template
/// embedded in the application. The texture seed exports stay only in memory;
/// the generated file starts with its package metadata and no visible assets.
pub fn create_new(file_path: &str) -> UtxResult<()> {
    let destination = Path::new(file_path);
    let package_name = validate_new_utx_destination(destination)?;
    let rewritten = texture_engine::create_empty_package(
        NEW_UTX_TEMPLATE,
        NEW_UTX_TEMPLATE_PACKAGE_NAME,
        package_name,
    )?;
    Package::parse(rewritten.clone())
        .and_then(|package| package.scan_entries())
        .map_err(|error| format!("O UTX criado a partir do modelo ficou inválido: {error}"))?;

    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Não foi possível criar o arquivo UTX: {error}"))?;
    output
        .write_all(&rewritten)
        .and_then(|_| output.flush())
        .map_err(|error| format!("Não foi possível gravar o novo UTX: {error}"))
}

pub fn export_entry(file_path: &str, export_index: usize, output_path: &str) -> UtxResult<()> {
    let decrypted = decrypt_raw(Path::new(file_path))?;
    let package = Package::parse(decrypted.working)?;
    let entry = package.entry_to_model(export_index)?;
    validate_export_extension(Path::new(output_path), entry.format)?;
    fs::write(output_path, package.export_bytes(&entry)?)
        .map_err(|error| format!("Não foi possível exportar a textura: {error}"))?;
    write_texture_metadata_file(&entry, Path::new(output_path))
}

pub fn export_entries(
    file_path: &str,
    export_indices: Vec<usize>,
    output_dir: &str,
) -> UtxResult<ExportSummary> {
    let decrypted = decrypt_raw(Path::new(file_path))?;
    let package = Package::parse(decrypted.working)?;
    let output_dir = Path::new(output_dir);
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("Não foi possível criar a pasta de destino: {error}"))?;

    let mut exported = 0;
    let mut failed = 0;
    for export_index in export_indices {
        let result = (|| -> UtxResult<()> {
            let entry = package.entry_to_model(export_index)?;
            let package_folder = package_prefix(&entry.name);
            let destination_dir = package_folder
                .map_or_else(|| output_dir.to_path_buf(), |name| output_dir.join(name));
            fs::create_dir_all(&destination_dir)
                .map_err(|error| format!("Não foi possível criar a pasta do pacote: {error}"))?;
            let destination = destination_dir.join(format!(
                "{}.{}",
                file_name_for_export(&entry.name),
                entry.format.export_extension()
            ));
            fs::write(&destination, package.export_bytes(&entry)?)
                .map_err(|error| format!("Erro de escrita: {error}"))?;
            write_texture_metadata_file(&entry, &destination)
        })();
        if result.is_ok() {
            exported += 1;
        } else {
            failed += 1;
        }
    }
    Ok(ExportSummary { exported, failed })
}

/// Extracts every texture from one or more UTX packages without opening them
/// in the editor cache. Each source package receives its own output directory
/// so packages that contain equally named groups/textures never overwrite one
/// another.
pub fn extract_packages_with_progress<F>(
    file_paths: Vec<String>,
    output_dir: &str,
    mode: UtxExtractMode,
    mut report_progress: F,
) -> UtxResult<UtxExtractSummary>
where
    F: FnMut(UtxExtractProgress),
{
    if file_paths.is_empty() {
        return Err("Selecione ao menos um arquivo UTX para extrair.".into());
    }

    let output_dir = Path::new(output_dir);
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("Não foi possível criar a pasta de destino: {error}"))?;

    report_progress(UtxExtractProgress {
        completed: 0,
        total: 0,
        package_name: String::new(),
        file_name: "Lendo pacotes UTX…".into(),
    });

    let mut errors = Vec::new();
    let mut failed = 0;
    let mut seen_paths = HashSet::new();
    let mut package_directory_counts = HashMap::<String, usize>::new();
    let mut packages = Vec::new();

    for file_path in file_paths {
        if !seen_paths.insert(file_path.clone()) {
            continue;
        }

        let source_path = Path::new(&file_path);
        let display_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Pacote UTX")
            .to_owned();
        let base_directory = source_path
            .file_stem()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("{display_name}: o nome do arquivo UTX não é válido."))?
            .to_owned();
        let package_directory = {
            let next = package_directory_counts
                .entry(base_directory.to_ascii_lowercase())
                .or_insert(0);
            *next += 1;
            if *next == 1 {
                base_directory
            } else {
                format!("{} ({next})", base_directory)
            }
        };

        let parsed = (|| -> UtxResult<(Package, Vec<UtxEntry>)> {
            let decrypted = decrypt_raw(source_path)?;
            let package = Package::parse(decrypted.working)?;
            let entries = package.scan_entries()?;
            Ok((package, entries))
        })();

        match parsed {
            Ok((package, entries)) => {
                packages.push((display_name, package_directory, package, entries))
            }
            Err(error) => {
                failed += 1;
                push_extract_error(&mut errors, format!("{display_name}: {error}"));
            }
        }
    }

    let total = packages
        .iter()
        .map(|(_, _, _, entries)| entries.len())
        .sum();
    let mut exported = 0;
    let mut skipped = 0;
    let mut completed = 0;

    for (display_name, package_directory, package, entries) in packages {
        for entry in entries {
            let texture_name = file_name_for_export(&entry.name);
            report_progress(UtxExtractProgress {
                completed,
                total,
                package_name: display_name.clone(),
                file_name: texture_name.clone(),
            });

            let result = (|| -> UtxResult<()> {
                let mut destination_dir = output_dir.join(&package_directory);
                if let Some(group) = package_prefix(&entry.name) {
                    destination_dir.push(group);
                }
                fs::create_dir_all(&destination_dir)
                    .map_err(|error| format!("Não foi possível criar a pasta do grupo: {error}"))?;

                let extension = match mode {
                    UtxExtractMode::Original => entry.format.export_extension(),
                    UtxExtractMode::Png => "png",
                };
                let destination = destination_dir.join(format!("{texture_name}.{extension}"));

                match mode {
                    UtxExtractMode::Original => {
                        fs::write(&destination, package.export_bytes(&entry)?)
                            .map_err(|error| format!("Erro de escrita: {error}"))?
                    }
                    UtxExtractMode::Png => {
                        if !entry.format.is_previewable() {
                            return Err(format!(
                                "O formato {} ainda não pode ser convertido para PNG.",
                                format_label(entry.format)
                            ));
                        }
                        let png = encode_png(
                            &package.mip0_pixels(entry.export_index)?,
                            entry.width,
                            entry.height,
                            entry.format,
                        )?;
                        fs::write(&destination, png)
                            .map_err(|error| format!("Erro de escrita: {error}"))?;
                    }
                }
                write_texture_metadata_file(&entry, &destination)
            })();

            match result {
                Ok(()) => exported += 1,
                Err(error)
                    if matches!(mode, UtxExtractMode::Png) && !entry.format.is_previewable() =>
                {
                    skipped += 1;
                    push_extract_error(
                        &mut errors,
                        format!("{display_name} / {texture_name}: {error}"),
                    );
                }
                Err(error) => {
                    failed += 1;
                    push_extract_error(
                        &mut errors,
                        format!("{display_name} / {texture_name}: {error}"),
                    );
                }
            }

            completed += 1;
            report_progress(UtxExtractProgress {
                completed,
                total,
                package_name: display_name.clone(),
                file_name: texture_name,
            });
        }
    }

    Ok(UtxExtractSummary {
        packages: seen_paths.len(),
        exported,
        skipped,
        failed,
        errors,
        output_directory: output_dir.to_string_lossy().into_owned(),
    })
}

fn push_extract_error(errors: &mut Vec<String>, error: String) {
    if errors.len() < MAX_UTX_EXTRACT_ERRORS {
        errors.push(error);
    }
}

pub fn preview_texture(file_path: &str, export_index: usize) -> UtxResult<TexturePreview> {
    let decrypted = decrypt_raw(Path::new(file_path))?;
    let package = Package::parse(decrypted.working)?;
    let entry = package.entry_to_model(export_index)?;
    if !entry.format.is_previewable() {
        return Err(format!(
            "A pré-visualização não é suportada para {}.",
            format_label(entry.format)
        ));
    }
    let pixels = package.mip0_pixels(export_index)?;
    encode_preview(&pixels, entry.width, entry.height, entry.format)
}

pub fn replace_entry(
    file_path: &str,
    export_index: usize,
    replacement_path: &str,
) -> UtxResult<()> {
    let source = Path::new(file_path);
    let replacement_path = Path::new(replacement_path);
    let decrypted = decrypt_raw(source)?;
    let request = texture_engine_import_request(read_imported_texture(replacement_path)?)?;
    let working =
        texture_engine::replace_texture(decrypted.working.clone(), export_index, &request)?;
    fs::write(source, re_encrypt(working, &decrypted))
        .map_err(|error| format!("Não foi possível gravar o pacote UTX: {error}"))
}

/// Applies validated replacements in a single decrypt → patch → encrypt cycle.
/// A malformed image never changes its corresponding export; successful images
/// still go through when another selection fails.
pub fn import_entries(
    file_path: &str,
    replacements: Vec<ReplacementRequest>,
) -> UtxResult<ImportSummary> {
    if replacements.is_empty() {
        return Ok(ImportSummary {
            imported: 0,
            skipped: 0,
            failed: 0,
        });
    }
    let source = Path::new(file_path);
    let decrypted = decrypt_raw(source)?;
    let mut working = decrypted.working.clone();
    let mut imported = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for request in replacements {
        let replacement_path = Path::new(&request.replacement_path);
        match read_imported_texture(replacement_path)
            .and_then(texture_engine_import_request)
            .and_then(|texture| {
                texture_engine::replace_texture(working.clone(), request.export_index, &texture)
            }) {
            Ok(updated) => {
                working = updated;
                imported += 1;
            }
            Err(error) if error.contains("Não foi possível ler") => failed += 1,
            Err(_) => skipped += 1,
        }
    }
    if imported > 0 {
        fs::write(source, re_encrypt(working, &decrypted))
            .map_err(|error| format!("Não foi possível gravar o pacote UTX: {error}"))?;
    }
    Ok(ImportSummary {
        imported,
        skipped,
        failed,
    })
}

fn read_imported_texture(path: &Path) -> UtxResult<ImportedTexture> {
    let name = imported_texture_name(path)?;
    let source = fs::read(path)
        .map_err(|error| format!("Não foi possível ler a textura selecionada: {error}"))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    let (format, width, height, pixels) = if extension.eq_ignore_ascii_case("tga") {
        let (width, height, pixels) = tga_pixels(&source)?;
        (UtxFormat::Rgba8, width, height, pixels)
    } else if extension.eq_ignore_ascii_case("dds") {
        dds_pixels(&source)?
    } else {
        return Err("Formato não suportado. Selecione arquivos .tga ou .dds.".into());
    };
    validate_import_dimensions(width, height)?;
    Ok(ImportedTexture {
        name,
        format,
        width,
        height,
        pixels,
        metadata: parse_texture_metadata_file(&path.with_extension("txt"))?,
    })
}

fn imported_texture_name(path: &Path) -> UtxResult<String> {
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or("Não foi possível obter o nome da textura pelo arquivo selecionado.")?;
    if name.contains('.') {
        return Err(
            "O nome do arquivo não pode conter ponto; use, por exemplo, MeuBotao.tga.".into(),
        );
    }
    Ok(name.to_owned())
}

fn validate_import_dimensions(width: i32, height: i32) -> UtxResult<()> {
    let valid = |dimension: i32| {
        u32::try_from(dimension)
            .ok()
            .is_some_and(|dimension| dimension.is_power_of_two())
    };
    if !valid(width) || !valid(height) {
        return Err(
            "A textura deve ter largura e altura em potências de dois, como 32×32 ou 256×128."
                .into(),
        );
    }
    Ok(())
}

fn tga_pixels(data: &[u8]) -> UtxResult<(i32, i32, Vec<u8>)> {
    if data.len() < 18 {
        return Err("Arquivo TGA inválido ou corrompido.".into());
    }
    if data[1] != 0 || !matches!(data[2], 2 | 10) || !matches!(data[16], 24 | 32) {
        return Err("O RGBA8 requer um TGA true-color de 24 ou 32 bits, sem paleta.".into());
    }
    let (width, height) = tga_dimensions(data)?;
    let pixel_count = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or("Textura TGA muito grande.")?;
    let bytes_per_pixel = usize::from(data[16] / 8);
    let mut input = tga_pixel_start(data)?;
    let mut pixels = Vec::with_capacity(
        pixel_count
            .checked_mul(4)
            .ok_or("Textura TGA muito grande.")?,
    );

    if data[2] == 2 {
        for _ in 0..pixel_count {
            let end = input
                .checked_add(bytes_per_pixel)
                .ok_or("Offset TGA inválido.")?;
            append_tga_pixel(
                &mut pixels,
                data.get(input..end)
                    .ok_or("Dados de pixels TGA truncados.")?,
                bytes_per_pixel,
            )?;
            input = end;
        }
    } else {
        while pixels.len() / 4 < pixel_count {
            let header = *data.get(input).ok_or("Dados RLE TGA truncados.")?;
            input += 1;
            let count = usize::from((header & 0x7f) + 1);
            let written = pixels.len() / 4;
            if count > pixel_count.saturating_sub(written) {
                return Err("Pacote RLE TGA excede o tamanho esperado.".into());
            }
            if header & 0x80 != 0 {
                let end = input
                    .checked_add(bytes_per_pixel)
                    .ok_or("Offset TGA inválido.")?;
                let pixel = data
                    .get(input..end)
                    .ok_or("Dados RLE TGA truncados.")?
                    .to_vec();
                input = end;
                for _ in 0..count {
                    append_tga_pixel(&mut pixels, &pixel, bytes_per_pixel)?;
                }
            } else {
                for _ in 0..count {
                    let end = input
                        .checked_add(bytes_per_pixel)
                        .ok_or("Offset TGA inválido.")?;
                    append_tga_pixel(
                        &mut pixels,
                        data.get(input..end).ok_or("Dados RLE TGA truncados.")?,
                        bytes_per_pixel,
                    )?;
                    input = end;
                }
            }
        }
    }
    Ok((width, height, pixels))
}

fn append_tga_pixel(output: &mut Vec<u8>, pixel: &[u8], bytes_per_pixel: usize) -> UtxResult<()> {
    if pixel.len() != bytes_per_pixel {
        return Err("Dados de pixels TGA truncados.".into());
    }
    output.extend_from_slice(&[
        pixel[0],
        pixel[1],
        pixel[2],
        if bytes_per_pixel == 4 { pixel[3] } else { 255 },
    ]);
    Ok(())
}

fn dds_pixels(data: &[u8]) -> UtxResult<(UtxFormat, i32, i32, Vec<u8>)> {
    if data.len() < 128 || data.get(..4) != Some(b"DDS ".as_slice()) {
        return Err("O DXT requer um arquivo DDS válido.".into());
    }
    let height = i32::from_le_bytes(
        data.get(12..16)
            .ok_or("Cabeçalho DDS truncado.")?
            .try_into()
            .map_err(|_| "Cabeçalho DDS inválido.")?,
    );
    let width = i32::from_le_bytes(
        data.get(16..20)
            .ok_or("Cabeçalho DDS truncado.")?
            .try_into()
            .map_err(|_| "Cabeçalho DDS inválido.")?,
    );
    let format = match data.get(84..88).ok_or("Cabeçalho DDS truncado.")? {
        b"DXT1" => UtxFormat::Dxt1,
        b"DXT3" => UtxFormat::Dxt3,
        b"DXT5" => UtxFormat::Dxt5,
        _ => return Err("O DDS deve estar compactado como DXT1, DXT3 ou DXT5.".into()),
    };
    let size = dxt_mip_size(width, height, format)?;
    let end = 128usize
        .checked_add(size)
        .ok_or("Textura DDS muito grande.")?;
    let pixels = data
        .get(128..end)
        .ok_or("Dados de pixels DDS truncados.")?
        .to_vec();
    Ok((format, width, height, pixels))
}

fn dxt_mip_size(width: i32, height: i32, format: UtxFormat) -> UtxResult<usize> {
    let width = usize::try_from(width).map_err(|_| "Dimensões DXT inválidas.")?;
    let height = usize::try_from(height).map_err(|_| "Dimensões DXT inválidas.")?;
    if width == 0 || height == 0 {
        return Err("Dimensões DXT inválidas.".into());
    }
    let block_bytes = if format == UtxFormat::Dxt1 {
        8usize
    } else if matches!(format, UtxFormat::Dxt3 | UtxFormat::Dxt5) {
        16usize
    } else {
        return Err("Formato DXT inválido.".into());
    };
    width
        .div_ceil(4)
        .checked_mul(height.div_ceil(4))
        .and_then(|blocks| blocks.checked_mul(block_bytes))
        .ok_or("Textura DDS muito grande.".into())
}

#[derive(Clone, Copy)]
struct PropertyPatch {
    offset: usize,
    size: usize,
}

struct TextureLayout {
    format: PropertyPatch,
    width: Option<PropertyPatch>,
    height: Option<PropertyPatch>,
    u_bits: Option<PropertyPatch>,
    v_bits: Option<PropertyPatch>,
    anim_next: Option<i32>,
    split9_x1: Option<PropertyPatch>,
    split9_x2: Option<PropertyPatch>,
    split9_x3: Option<PropertyPatch>,
    split9_y1: Option<PropertyPatch>,
    split9_y2: Option<PropertyPatch>,
    split9_y3: Option<PropertyPatch>,
    mip_count_offset: usize,
    mip_width_offset: usize,
    mip_payload_offset: usize,
}

struct SerializedTexture {
    bytes: Vec<u8>,
    mip_width_offset: usize,
    width_offset_value: usize,
}

struct CreatedTextureGroup {
    object_reference: i32,
    serialized_data: Vec<u8>,
}

fn validate_texture_group_name(group_name: &str) -> UtxResult<()> {
    if group_name.eq_ignore_ascii_case("Pacote principal") {
        return Ok(());
    }
    if !group_name.is_ascii() || group_name.contains('.') {
        return Err(
            "O nome do grupo deve usar apenas ASCII e não pode conter ponto; use, por exemplo, CandidateWnd."
                .into(),
        );
    }
    Ok(())
}

fn validate_new_utx_destination(destination: &Path) -> UtxResult<&str> {
    if destination.exists() {
        return Err("Já existe um arquivo com esse nome. Escolha outro nome ou local.".into());
    }
    if !destination
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("utx"))
    {
        return Err("O novo pacote deve usar a extensão .utx.".into());
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or("Escolha uma pasta válida para salvar o novo UTX.")?;
    if !parent.is_dir() {
        return Err("A pasta escolhida para salvar o UTX não existe.".into());
    }
    let package_name = destination
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or("Informe um nome válido para o pacote UTX.")?;
    if !package_name.is_ascii()
        || !package_name
            .bytes()
            .all(|character| character.is_ascii_alphanumeric() || character == b'_')
    {
        return Err(
            "O nome do UTX deve usar apenas letras, números e sublinhado; use, por exemplo, L2UI_Custom."
                .into(),
        );
    }
    Ok(package_name)
}

fn embedded_template_package() -> UtxResult<Package> {
    Package::parse(NEW_UTX_TEMPLATE.to_vec())
        .map_err(|error| format!("O modelo interno de UTX é inválido: {error}"))
}

fn embedded_template_for_target(target: &Package) -> UtxResult<Option<Package>> {
    let template = embedded_template_package()?;
    Ok(target
        .is_compatible_with_embedded_template(&template)
        .then_some(template))
}

fn create_empty_package_from_template(
    template: &Package,
    package_name: &str,
) -> UtxResult<Vec<u8>> {
    let mut names = template.names.clone();
    let template_name_index = names
        .iter()
        .position(|entry| {
            entry
                .name
                .eq_ignore_ascii_case(NEW_UTX_TEMPLATE_PACKAGE_NAME)
        })
        .ok_or("O modelo interno de UTX não possui o nome de pacote esperado.")?;
    names[template_name_index].name = package_name.to_owned();

    let name_table = serialize_name_table(&names)?;
    let header = template
        .data
        .get(..template.name_offset)
        .ok_or("O cabeçalho do modelo UTX está truncado.")?;
    let import_table = serialize_import_table(&template.imports);
    let import_offset = header
        .len()
        .checked_add(name_table.len())
        .ok_or("O novo UTX excede o limite de tamanho.")?;
    let export_offset = import_offset
        .checked_add(import_table.len())
        .ok_or("O novo UTX excede o limite de tamanho.")?;
    let mut rewritten = Vec::with_capacity(export_offset);
    rewritten.extend_from_slice(header);
    rewritten.extend_from_slice(&name_table);
    rewritten.extend_from_slice(&import_table);
    write_i32_at(
        &mut rewritten,
        12,
        checked_i32(names.len(), "Muitos nomes no novo UTX.")?,
    )?;
    write_i32_at(
        &mut rewritten,
        16,
        checked_i32(header.len(), "O novo UTX excede o limite de tamanho.")?,
    )?;
    write_i32_at(&mut rewritten, 20, 0)?;
    write_i32_at(
        &mut rewritten,
        24,
        checked_i32(export_offset, "O novo UTX excede o limite de tamanho.")?,
    )?;
    write_i32_at(
        &mut rewritten,
        28,
        checked_i32(template.imports.len(), "Muitas importações no novo UTX.")?,
    )?;
    write_i32_at(
        &mut rewritten,
        32,
        checked_i32(import_offset, "O novo UTX excede o limite de tamanho.")?,
    )?;
    Ok(rewritten)
}

fn name_index_or_insert(names: &mut Vec<NameEntry>, name: &str) -> UtxResult<i32> {
    if let Some(index) = names
        .iter()
        .position(|entry| entry.name.eq_ignore_ascii_case(name))
    {
        return checked_i32(index, "A tabela de nomes excede o limite do formato.");
    }
    let index = checked_i32(names.len(), "A tabela de nomes excede o limite do formato.")?;
    let flags = names
        .first()
        .map(|entry| entry.flags)
        .unwrap_or(0x0007_0000);
    names.push(NameEntry {
        name: name.to_owned(),
        flags,
    });
    Ok(index)
}

fn import_reference(index: usize) -> UtxResult<i32> {
    checked_i32(
        index
            .checked_add(1)
            .ok_or("Referência de importação inválida.")?,
        "Muitas importações no pacote.",
    )
    .map(|reference| -reference)
}

fn export_reference(index: usize) -> UtxResult<i32> {
    checked_i32(
        index
            .checked_add(1)
            .ok_or("Referência de exportação inválida.")?,
        "Muitas exportações no pacote.",
    )
}

fn ensure_core_package_import(
    names: &mut Vec<NameEntry>,
    imports: &mut Vec<ImportEntry>,
) -> UtxResult<i32> {
    let core_name_index = name_index_or_insert(names, "Core")?;
    let package_name_index = name_index_or_insert(names, "Package")?;
    let class_name_index = name_index_or_insert(names, "Class")?;
    let core_reference = if let Some(index) = imports
        .iter()
        .position(|entry| entry.package == 0 && entry.name_index == core_name_index)
    {
        import_reference(index)?
    } else {
        imports.push(ImportEntry {
            class_package: core_name_index,
            class_name: package_name_index,
            package: 0,
            name_index: core_name_index,
        });
        import_reference(imports.len() - 1)?
    };
    if let Some(index) = imports
        .iter()
        .position(|entry| entry.package == core_reference && entry.name_index == package_name_index)
    {
        return import_reference(index);
    }
    imports.push(ImportEntry {
        class_package: core_name_index,
        class_name: class_name_index,
        package: core_reference,
        name_index: package_name_index,
    });
    import_reference(imports.len() - 1)
}

fn create_texture_group(
    package: &Package,
    group_name: &str,
    names: &mut Vec<NameEntry>,
    imports: &mut Vec<ImportEntry>,
    exports: &mut Vec<ExportEntry>,
) -> UtxResult<CreatedTextureGroup> {
    let name_index = name_index_or_insert(names, group_name)?;
    let group_template = package.group_template();
    let (class, super_class, flags, serialized_data) = if let Some(template) = group_template {
        (
            template.class,
            template.super_class,
            template.flags,
            package.export_data(&template)?.to_vec(),
        )
    } else {
        (
            ensure_core_package_import(names, imports)?,
            0,
            0x0007_0004,
            vec![1],
        )
    };
    let reference = export_reference(exports.len())?;
    exports.push(ExportEntry {
        class,
        super_class,
        package: 0,
        name_index,
        flags,
        size: checked_i32(
            serialized_data.len(),
            "O objeto de grupo é grande demais para este formato.",
        )?,
        offset: 0,
    });
    Ok(CreatedTextureGroup {
        object_reference: reference,
        serialized_data,
    })
}

fn append_texture_entries(
    working: &[u8],
    package: &Package,
    embedded_template: Option<&Package>,
    package_name: &str,
    textures: &[&ImportedTexture],
) -> UtxResult<Vec<u8>> {
    if textures.is_empty() {
        return Ok(working.to_vec());
    }
    let mut names = package.names.clone();
    let mut imports = package.imports.clone();
    let mut exports = package.exports.clone();
    let (destination_outer, group_data) = match package.group_outer_for_name(package_name)? {
        Some(outer) => (outer, Vec::new()),
        None => {
            let created = create_texture_group(
                package,
                package_name,
                &mut names,
                &mut imports,
                &mut exports,
            )?;
            let group_index = object_index(created.object_reference)?;
            exports[group_index].offset =
                checked_i32(working.len(), "O pacote excede o limite de 2 GB.")?;
            (created.object_reference, created.serialized_data)
        }
    };
    let mut export_data = group_data;

    for texture in textures {
        let requires_animation = texture.metadata.animation.is_some();
        let (template_package, template_index) = match package.template_texture_for_import(
            package_name,
            texture.metadata.split9.is_some(),
            requires_animation,
        ) {
            Ok((index, _)) => (package, index),
            Err(error) => match embedded_template {
                Some(template) => template
                    .template_texture_for_import(
                        package_name,
                        texture.metadata.split9.is_some(),
                        requires_animation,
                    )
                    .map(|(index, _)| (template, index))?,
                None => return Err(error),
            },
        };
        let template = template_package.export_at(template_index)?.clone();
        let template_raw = template_package.export_data(&template)?;
        let export_offset = working
            .len()
            .checked_add(export_data.len())
            .ok_or("Tamanho de pacote inválido.")?;
        let serialized = build_texture_export(template_raw, template_package, texture)?;
        let mut serialized_data = serialized.bytes;
        let width_offset = export_offset
            .checked_add(serialized.width_offset_value)
            .ok_or("Offset de textura inválido.")?;
        write_i32_at(
            &mut serialized_data,
            serialized.mip_width_offset,
            checked_i32(width_offset, "O pacote excede o limite de tamanho.")?,
        )?;
        let name_index = name_index_or_insert(&mut names, &texture.name)?;
        exports.push(ExportEntry {
            class: template.class,
            super_class: template.super_class,
            package: destination_outer,
            name_index,
            flags: template.flags,
            size: checked_i32(
                serialized_data.len(),
                "A textura é grande demais para este formato.",
            )?,
            offset: checked_i32(export_offset, "O pacote excede o limite de 2 GB.")?,
        });
        export_data.extend_from_slice(&serialized_data);
    }
    let name_table = serialize_name_table(&names)?;
    let import_table = serialize_import_table(&imports);
    let export_table = serialize_export_table(&exports);
    let name_offset = working
        .len()
        .checked_add(export_data.len())
        .ok_or("Tamanho de pacote inválido.")?;
    let import_offset = name_offset
        .checked_add(name_table.len())
        .ok_or("Tamanho de pacote inválido.")?;
    let export_offset_table = import_offset
        .checked_add(import_table.len())
        .ok_or("Tamanho de pacote inválido.")?;
    let mut rewritten = Vec::with_capacity(
        export_offset_table
            .checked_add(export_table.len())
            .ok_or("Tamanho de pacote inválido.")?,
    );
    rewritten.extend_from_slice(working);
    rewritten.extend_from_slice(&export_data);
    rewritten.extend_from_slice(&name_table);
    rewritten.extend_from_slice(&import_table);
    rewritten.extend_from_slice(&export_table);
    write_i32_at(
        &mut rewritten,
        12,
        checked_i32(names.len(), "Muitos nomes no pacote.")?,
    )?;
    write_i32_at(
        &mut rewritten,
        16,
        checked_i32(name_offset, "Pacote grande demais.")?,
    )?;
    write_i32_at(
        &mut rewritten,
        20,
        checked_i32(exports.len(), "Muitas exportações no pacote.")?,
    )?;
    write_i32_at(
        &mut rewritten,
        24,
        checked_i32(export_offset_table, "Pacote grande demais.")?,
    )?;
    write_i32_at(
        &mut rewritten,
        28,
        checked_i32(imports.len(), "Muitas importações no pacote.")?,
    )?;
    write_i32_at(
        &mut rewritten,
        32,
        checked_i32(import_offset, "Pacote grande demais.")?,
    )?;
    Ok(rewritten)
}

fn build_texture_export(
    template_raw: &[u8],
    package: &Package,
    texture: &ImportedTexture,
) -> UtxResult<SerializedTexture> {
    let layout = texture_layout(template_raw, package)?;
    let format = texture
        .format
        .value()
        .ok_or("Formato de textura inválido para importação.")?;
    let mut output = template_raw
        .get(..layout.mip_count_offset)
        .ok_or("Modelo de textura truncado.")?
        .to_vec();
    patch_property(&mut output, layout.format, i32::from(format))?;
    if let Some(width) = layout.width {
        patch_property(&mut output, width, texture.width)?;
    }
    if let Some(height) = layout.height {
        patch_property(&mut output, height, texture.height)?;
    }
    if let Some(u_bits) = layout.u_bits {
        patch_property(&mut output, u_bits, dimension_bits(texture.width)?)?;
    }
    if let Some(v_bits) = layout.v_bits {
        patch_property(&mut output, v_bits, dimension_bits(texture.height)?)?;
    }
    if let Some(split9) = texture.metadata.split9 {
        patch_split9_properties(&mut output, &layout, split9)?;
    }
    output.push(1);
    output.extend_from_slice(
        template_raw
            .get(layout.mip_count_offset + 1..layout.mip_payload_offset)
            .ok_or("Modelo de textura truncado.")?,
    );
    write_compact(
        &mut output,
        checked_i32(
            texture.pixels.len(),
            "A textura é grande demais para este formato.",
        )?,
    );
    output.extend_from_slice(&texture.pixels);
    let width_offset_value = output.len();
    output.extend_from_slice(&texture.width.to_le_bytes());
    output.extend_from_slice(&texture.height.to_le_bytes());
    output.push(dimension_bits(texture.width)? as u8);
    output.push(dimension_bits(texture.height)? as u8);
    Ok(SerializedTexture {
        bytes: output,
        mip_width_offset: layout.mip_width_offset,
        width_offset_value,
    })
}

fn texture_layout(raw: &[u8], package: &Package) -> UtxResult<TextureLayout> {
    let mut reader = Reader::new(raw);
    let mut format = None;
    let mut width = None;
    let mut height = None;
    let mut u_bits = None;
    let mut v_bits = None;
    let mut anim_next = None;
    let mut split9_x1 = None;
    let mut split9_x2 = None;
    let mut split9_x3 = None;
    let mut split9_y1 = None;
    let mut split9_y2 = None;
    let mut split9_y3 = None;
    loop {
        let name = package.name(reader.read_compact()?).to_ascii_lowercase();
        if name == "none" {
            break;
        }
        let info = reader.read_u8()?;
        let property_type = info & 0x0f;
        let size_type = (info >> 4) & 0x07;
        let is_array = info & 0x80 != 0;
        if property_type == 10 {
            reader.read_compact()?;
        }
        let size = property_size(&mut reader, size_type)?;
        if is_array && property_type != 3 {
            reader.read_compact()?;
        }
        let patch = PropertyPatch {
            offset: reader.position(),
            size,
        };
        match name.as_str() {
            "format" => format = Some(patch),
            "usize" => width = Some(patch),
            "vsize" => height = Some(patch),
            "ubits" => u_bits = Some(patch),
            "vbits" => v_bits = Some(patch),
            "animnext" => {
                let value = raw
                    .get(
                        patch.offset
                            ..patch
                                .offset
                                .checked_add(patch.size)
                                .ok_or("Offset de propriedade inválido.")?,
                    )
                    .ok_or("Modelo de textura truncado.")?;
                anim_next = Some(Reader::new(value).read_compact()?);
            }
            "split9x1" => split9_x1 = Some(patch),
            "split9x2" => split9_x2 = Some(patch),
            "split9x3" => split9_x3 = Some(patch),
            "split9y1" => split9_y1 = Some(patch),
            "split9y2" => split9_y2 = Some(patch),
            "split9y3" => split9_y3 = Some(patch),
            _ => {}
        }
        reader.skip(size)?;
    }
    skip_unreal_extra(&mut reader, package.version, package.licensee)?;
    let mip_count_offset = reader.position();
    reader.read_u8()?;
    let mip_width_offset = reader.position();
    reader.skip(4)?;
    Ok(TextureLayout {
        format: format
            .ok_or("O modelo não possui a propriedade Format necessária para criar a textura.")?,
        width,
        height,
        u_bits,
        v_bits,
        anim_next,
        split9_x1,
        split9_x2,
        split9_x3,
        split9_y1,
        split9_y2,
        split9_y3,
        mip_count_offset,
        mip_width_offset,
        mip_payload_offset: reader.position(),
    })
}

fn patch_property(output: &mut [u8], patch: PropertyPatch, value: i32) -> UtxResult<()> {
    let target = output
        .get_mut(
            patch.offset
                ..patch
                    .offset
                    .checked_add(patch.size)
                    .ok_or("Offset de propriedade inválido.")?,
        )
        .ok_or("Modelo de textura truncado.")?;
    match patch.size {
        1 => {
            target[0] = u8::try_from(value).map_err(|_| "Valor de propriedade fora do limite.")?;
        }
        2 => {
            target.copy_from_slice(
                &u16::try_from(value)
                    .map_err(|_| "Valor de propriedade fora do limite.")?
                    .to_le_bytes(),
            );
        }
        4 => target.copy_from_slice(&value.to_le_bytes()),
        _ => {
            return Err(
                "O modelo usa um tamanho de propriedade não suportado para importação.".into(),
            )
        }
    }
    Ok(())
}

fn patch_split9_properties(
    output: &mut [u8],
    layout: &TextureLayout,
    split9: Split9,
) -> UtxResult<()> {
    for (name, patch, value) in [
        ("Split9X1", layout.split9_x1, split9.x1),
        ("Split9X2", layout.split9_x2, split9.x2),
        ("Split9X3", layout.split9_x3, split9.x3),
        ("Split9Y1", layout.split9_y1, split9.y1),
        ("Split9Y2", layout.split9_y2, split9.y2),
        ("Split9Y3", layout.split9_y3, split9.y3),
    ] {
        match patch {
            Some(patch) => patch_property(output, patch, value)?,
            None if value == 0 => {}
            None => {
                return Err(format!(
                    "O modelo estrutural não possui a propriedade {name} necessária para o valor {value}."
                ));
            }
        }
    }
    Ok(())
}

fn dimension_bits(value: i32) -> UtxResult<i32> {
    let value = u32::try_from(value).map_err(|_| "Dimensão de textura inválida.")?;
    if !value.is_power_of_two() {
        return Err("A textura deve ter dimensões em potências de dois.".into());
    }
    Ok(value.ilog2() as i32)
}

fn apply_replacement(
    working: &mut [u8],
    package: &Package,
    export_index: usize,
    replacement: &[u8],
    metadata: &TextureMetadata,
) -> UtxResult<()> {
    let entry = package.entry_to_model(export_index)?;
    if !entry.format.is_previewable() {
        return Err(format!(
            "Substituição não suportada para {}.",
            format_label(entry.format)
        ));
    }
    let mip = package.mip0_location(export_index)?;
    let source = if entry.format == UtxFormat::Rgba8 {
        let (width, height, pixels) = tga_pixels(replacement)?;
        if width != entry.width || height != entry.height {
            return Err(format!(
                "Tamanho incompatível: esperado {}×{}, recebido {width}×{height}.",
                entry.width, entry.height
            ));
        }
        pixels
    } else {
        validate_dds(replacement, entry.width, entry.height, entry.format)?;
        replacement
            .get(128..)
            .ok_or("Dados de pixels DDS truncados.")?
            .to_vec()
    };
    let source = source.get(..mip.size).ok_or_else(|| {
        format!(
            "Tamanho de pixels incompatível: esperado {} bytes.",
            mip.size
        )
    })?;
    let destination_end = mip
        .absolute_pixel_offset
        .checked_add(mip.size)
        .ok_or("Offset de textura inválido.")?;
    let destination = working
        .get_mut(mip.absolute_pixel_offset..destination_end)
        .ok_or("Os pixels estão fora do pacote.")?;
    destination.copy_from_slice(source);

    let export_offset = read_offset(package.export_at(export_index)?.offset, "dados de textura")?;
    let width_offset_position = export_offset
        .checked_add(mip.width_offset_position)
        .ok_or("Offset de textura inválido.")?;
    let width_offset_value = mip
        .absolute_pixel_offset
        .checked_add(mip.size)
        .ok_or("Offset de textura inválido.")?;
    write_i32_at(
        working,
        width_offset_position,
        checked_i32(width_offset_value, "O pacote excede o limite de tamanho.")?,
    )?;

    if entry.has_split9 {
        if let Some(split9) = metadata.split9 {
            patch_split9(working, package, export_index, split9)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Split9 {
    x1: i32,
    x2: i32,
    x3: i32,
    y1: i32,
    y2: i32,
    y3: i32,
}

fn parse_texture_metadata_file(path: &Path) -> UtxResult<TextureMetadata> {
    if !path.is_file() {
        return Ok(TextureMetadata::default());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Não foi possível ler os dados complementares: {error}"))?;
    let mut values = std::collections::HashMap::new();
    let mut settings = TextureSettings::default();
    let mut animation = ImportedAnimation::default();
    let mut section = "";
    for line in content.lines() {
        let line = line.trim();
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|name| name.strip_suffix(']'))
        {
            section = name.trim();
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            if section.eq_ignore_ascii_case("Split9") {
                if let Ok(value) = value.parse::<i32>() {
                    values.insert(key, value);
                }
            } else if section.eq_ignore_ascii_case("Texture") {
                match key.as_str() {
                    "alpha" | "balphatexture" => settings.alpha = parse_metadata_bool(value),
                    "masked" | "bmasked" => settings.masked = parse_metadata_bool(value),
                    "uclamp" => settings.u_clamp = value.parse::<i32>().ok(),
                    "vclamp" => settings.v_clamp = value.parse::<i32>().ok(),
                    "uclampmode" => settings.u_clamp_mode = value.parse::<i32>().ok(),
                    "vclampmode" => settings.v_clamp_mode = value.parse::<i32>().ok(),
                    _ => {}
                }
            } else if section.eq_ignore_ascii_case("Animations") {
                match key.as_str() {
                    "animnext" => animation.anim_next = Some(value.to_owned()),
                    "maxframerate" => animation.max_frame_rate = parse_metadata_float(value),
                    "minframerate" => animation.min_frame_rate = parse_metadata_float(value),
                    "onetimeanimloop" => animation.one_time_anim_loop = parse_metadata_bool(value),
                    "primecount" => animation.prime_count = value.parse::<i32>().ok(),
                    "totalframenum" => animation.total_frame_num = value.parse::<i32>().ok(),
                    _ => {}
                }
            }
        }
    }
    let split9 = values.contains_key("split9x1").then(|| Split9 {
        x1: *values.get("split9x1").unwrap_or(&0),
        x2: *values.get("split9x2").unwrap_or(&0),
        x3: *values.get("split9x3").unwrap_or(&0),
        y1: *values.get("split9y1").unwrap_or(&0),
        y2: *values.get("split9y2").unwrap_or(&0),
        y3: *values.get("split9y3").unwrap_or(&0),
    });
    Ok(TextureMetadata {
        settings,
        split9,
        animation: animation.has_values().then_some(animation),
    })
}

fn parse_metadata_float(value: &str) -> Option<f32> {
    value
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_metadata_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "sim" => Some(true),
        "false" | "0" | "no" | "não" | "nao" => Some(false),
        _ => None,
    }
}

fn patch_split9(
    working: &mut [u8],
    package: &Package,
    export_index: usize,
    split9: Split9,
) -> UtxResult<()> {
    let export = package.export_at(export_index)?;
    let raw = package.export_data(export)?;
    let mut reader = Reader::new(raw);
    let mut offsets = std::collections::HashMap::new();
    loop {
        let name = package.name(reader.read_compact()?).to_ascii_lowercase();
        if name == "none" {
            break;
        }
        let info = reader.read_u8()?;
        let property_type = info & 0x0f;
        let size_type = (info >> 4) & 0x07;
        let is_array = info & 0x80 != 0;
        if property_type == 10 {
            reader.read_compact()?;
        }
        let size = property_size(&mut reader, size_type)?;
        if is_array && property_type != 3 {
            reader.read_compact()?;
        }
        if size == 4 && name.starts_with("split9") {
            offsets.insert(name, reader.position());
        }
        reader.skip(size)?;
    }
    for (name, value) in [
        ("split9x1", split9.x1),
        ("split9x2", split9.x2),
        ("split9x3", split9.x3),
        ("split9y1", split9.y1),
        ("split9y2", split9.y2),
        ("split9y3", split9.y3),
    ] {
        if let Some(local) = offsets.get(name) {
            let position = read_offset(export.offset, "dados Split9")?
                .checked_add(*local)
                .ok_or("Offset Split9 inválido.")?;
            write_i32_at(working, position, value)?;
        }
    }
    Ok(())
}

fn write_texture_metadata_file(entry: &UtxEntry, texture_path: &Path) -> UtxResult<()> {
    if !entry.settings.has_values() && !entry.has_split9 && entry.animation.is_none() {
        return Ok(());
    }
    let mut content = String::new();
    if entry.settings.has_values() {
        content.push_str("[Texture]\n");
        if let Some(value) = entry.settings.alpha {
            content.push_str(&format!("Alpha={}\n", if value { "True" } else { "False" }));
        }
        if let Some(value) = entry.settings.masked {
            content.push_str(&format!(
                "Masked={}\n",
                if value { "True" } else { "False" }
            ));
        }
        if let Some(value) = entry.settings.u_clamp {
            content.push_str(&format!("UClamp={value}\n"));
        }
        if let Some(value) = entry.settings.v_clamp {
            content.push_str(&format!("VClamp={value}\n"));
        }
        if let Some(value) = entry.settings.u_clamp_mode {
            content.push_str(&format!("UClampMode={value}\n"));
        }
        if let Some(value) = entry.settings.v_clamp_mode {
            content.push_str(&format!("VClampMode={value}\n"));
        }
    }
    if entry.has_split9 {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&format!(
            "[Split9]\nSplit9X1={}\nSplit9X2={}\nSplit9X3={}\nSplit9Y1={}\nSplit9Y2={}\nSplit9Y3={}\n",
            entry.split9_x1,
            entry.split9_x2,
            entry.split9_x3,
            entry.split9_y1,
            entry.split9_y2,
            entry.split9_y3
        ));
    }
    if let Some(animation) = &entry.animation {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str("[Animations]\n");
        if animation.properties.anim_next {
            content.push_str(&format!(
                "AnimNext={}\n",
                animation.anim_next.as_deref().unwrap_or_default()
            ));
        }
        if animation.properties.max_frame_rate {
            content.push_str(&format!(
                "MaxFrameRate={:.6}\n",
                animation.values.max_frame_rate
            ));
        }
        if animation.properties.min_frame_rate {
            content.push_str(&format!(
                "MinFrameRate={:.6}\n",
                animation.values.min_frame_rate
            ));
        }
        if animation.properties.one_time_anim_loop {
            content.push_str(&format!(
                "OneTimeAnimLoop={}\n",
                if animation.values.one_time_anim_loop {
                    "True"
                } else {
                    "False"
                }
            ));
        }
        if animation.properties.prime_count {
            content.push_str(&format!("PrimeCount={}\n", animation.values.prime_count));
        }
        if animation.properties.total_frame_num {
            content.push_str(&format!(
                "TotalFrameNum={}\n",
                animation.values.total_frame_num
            ));
        }
    }
    fs::write(texture_path.with_extension("txt"), content)
        .map_err(|error| format!("Não foi possível salvar os dados complementares: {error}"))
}

fn write_sized_property(
    output: &mut Vec<u8>,
    name_index: i32,
    property_type: u8,
    value: &[u8],
) -> UtxResult<()> {
    write_compact(output, name_index);
    match value.len() {
        1 => output.push(property_type),
        2 => output.push(0x10 | property_type),
        4 => output.push(0x20 | property_type),
        12 => output.push(0x30 | property_type),
        16 => output.push(0x40 | property_type),
        length if length <= u8::MAX as usize => {
            output.push(0x50 | property_type);
            output.push(length as u8);
        }
        _ => return Err("Propriedade de animação grande demais.".into()),
    }
    output.extend_from_slice(value);
    Ok(())
}

struct RewrittenTexture {
    bytes: Vec<u8>,
    mip_count_offset: usize,
    original_mip_count_offset: usize,
}

fn rewrite_texture_animation_properties(
    raw: &[u8],
    package: &Package,
    values: AnimationValues,
) -> UtxResult<RewrittenTexture> {
    let layout = texture_layout(raw, package)?;
    let mut reader = Reader::new(raw);
    let mut output = Vec::with_capacity(raw.len().saturating_add(8));
    let mut has_anim_next = false;
    let mut has_max_frame_rate = false;
    let mut has_min_frame_rate = false;
    let mut has_one_time_anim_loop = false;
    let mut has_prime_count = false;
    let mut has_total_frame_num = false;
    let terminator_start = loop {
        let property_start = reader.position();
        let name_index = reader.read_compact()?;
        let name = package.name(name_index).to_ascii_lowercase();
        if name == "none" {
            break property_start;
        }
        let info = reader.read_u8()?;
        let info_offset = reader.position() - 1;
        let property_type = info & 0x0f;
        let size_type = (info >> 4) & 0x07;
        let is_array = info & 0x80 != 0;
        if property_type == 10 {
            reader.read_compact()?;
        }
        let size = property_size(&mut reader, size_type)?;
        if is_array && property_type != 3 {
            reader.read_compact()?;
        }
        let value_start = reader.position();
        reader.skip(size)?;
        let property_end = reader.position();

        if name == "animnext" {
            if property_type != 5 {
                return Err("O modelo estrutural possui AnimNext em um formato inválido.".into());
            }
            let mut anim_next = Vec::new();
            write_compact(&mut anim_next, values.anim_next);
            write_sized_property(&mut output, name_index, property_type, &anim_next)?;
            has_anim_next = true;
            continue;
        }

        let output_start = output.len();
        output.extend_from_slice(
            raw.get(property_start..property_end)
                .ok_or("Dados de textura truncados.")?,
        );
        let output_value_start = output_start
            .checked_add(value_start - property_start)
            .ok_or("Offset de propriedade inválido.")?;
        match name.as_str() {
            "maxframerate" => {
                if property_type != 4 || size != 4 {
                    return Err(
                        "O modelo estrutural possui MaxFrameRate em um formato inválido.".into(),
                    );
                }
                output[output_value_start..output_value_start + 4]
                    .copy_from_slice(&values.max_frame_rate.to_bits().to_le_bytes());
                has_max_frame_rate = true;
            }
            "minframerate" => {
                if property_type != 4 || size != 4 {
                    return Err(
                        "O modelo estrutural possui MinFrameRate em um formato inválido.".into(),
                    );
                }
                output[output_value_start..output_value_start + 4]
                    .copy_from_slice(&values.min_frame_rate.to_bits().to_le_bytes());
                has_min_frame_rate = true;
            }
            "onetimeanimloop" | "banimloop" => {
                if property_type != 3 {
                    return Err(
                        "O modelo estrutural possui OneTimeAnimLoop em um formato inválido.".into(),
                    );
                }
                let output_info_offset = output_start
                    .checked_add(info_offset - property_start)
                    .ok_or("Offset de propriedade inválido.")?;
                output[output_info_offset] =
                    (info & 0x7f) | if values.one_time_anim_loop { 0x80 } else { 0 };
                has_one_time_anim_loop = true;
            }
            "primecount" => {
                match (property_type, size) {
                    (1, 1) => {
                        output[output_value_start] = u8::try_from(values.prime_count)
                            .map_err(|_| "PrimeCount deve estar entre 0 e 255.")?;
                    }
                    (2, 4) => {
                        output[output_value_start..output_value_start + 4]
                            .copy_from_slice(&values.prime_count.to_le_bytes());
                    }
                    _ => {
                        return Err(
                            "O modelo estrutural possui PrimeCount em um formato inválido.".into(),
                        );
                    }
                }
                has_prime_count = true;
            }
            "totalframenum" => {
                if property_type != 2 || size != 4 {
                    return Err(
                        "O modelo estrutural possui TotalFrameNum em um formato inválido.".into(),
                    );
                }
                output[output_value_start..output_value_start + 4]
                    .copy_from_slice(&values.total_frame_num.to_le_bytes());
                has_total_frame_num = true;
            }
            _ => {}
        }
    };
    if !has_anim_next {
        return Err(
            "A textura não possui AnimNext; use uma textura animada como modelo estrutural.".into(),
        );
    }
    if values.max_frame_rate != 0.0 && !has_max_frame_rate {
        return Err("O modelo estrutural não possui MaxFrameRate.".into());
    }
    if values.min_frame_rate != 0.0 && !has_min_frame_rate {
        return Err("O modelo estrutural não possui MinFrameRate.".into());
    }
    if values.one_time_anim_loop && !has_one_time_anim_loop {
        return Err(
            "O modelo estrutural não possui OneTimeAnimLoop; adicione essa propriedade ao TlpAnim no Unreal Editor."
                .into(),
        );
    }
    if values.prime_count != 0 && !has_prime_count {
        return Err(
            "O modelo estrutural não possui PrimeCount; adicione essa propriedade ao TlpAnim no Unreal Editor."
                .into(),
        );
    }
    if values.total_frame_num != 0 && !has_total_frame_num {
        return Err("O modelo estrutural não possui TotalFrameNum.".into());
    }
    output.extend_from_slice(
        raw.get(terminator_start..layout.mip_count_offset)
            .ok_or("Modelo de textura truncado.")?,
    );
    let mip_count_offset = output.len();
    output.extend_from_slice(
        raw.get(layout.mip_count_offset..)
            .ok_or("Modelo de textura truncado.")?,
    );
    Ok(RewrittenTexture {
        bytes: output,
        mip_count_offset,
        original_mip_count_offset: layout.mip_count_offset,
    })
}

fn resolve_animation_reference(
    package: &Package,
    export_index: usize,
    requested_path: &str,
) -> UtxResult<i32> {
    let requested = requested_path
        .trim()
        .strip_prefix("Texture'")
        .unwrap_or(requested_path.trim())
        .trim_end_matches('\'')
        .trim();
    if requested.is_empty() {
        return Ok(0);
    }
    let current = package.entry_to_model(export_index)?;
    let requested_leaf = texture_leaf_name(requested);
    let mut same_group = None;
    let mut leaf_matches = Vec::new();
    for candidate_index in 0..package.exports.len() {
        let Ok(candidate) = package.entry_to_model(candidate_index) else {
            continue;
        };
        if candidate.name.eq_ignore_ascii_case(requested)
            || requested
                .strip_suffix(&format!(".{}", candidate.name))
                .is_some()
        {
            return export_reference(candidate_index);
        }
        if texture_leaf_name(&candidate.name).eq_ignore_ascii_case(requested_leaf) {
            if package_prefix(&candidate.name) == package_prefix(&current.name) {
                same_group = Some(candidate_index);
            }
            leaf_matches.push(candidate_index);
        }
    }
    if let Some(candidate_index) = same_group {
        return export_reference(candidate_index);
    }
    if leaf_matches.len() == 1 {
        return export_reference(leaf_matches[0]);
    }
    for (import_index, _) in package.imports.iter().enumerate() {
        if package
            .import_path(import_index, 0)
            .is_ok_and(|path| path.eq_ignore_ascii_case(requested))
        {
            return import_reference(import_index);
        }
    }
    Err(format!(
        "A textura indicada em AnimNext não foi encontrada no pacote: {requested}."
    ))
}

fn merged_animation_values(
    package: &Package,
    export_index: usize,
    animation: &ImportedAnimation,
) -> UtxResult<AnimationValues> {
    let raw = package.export_data(package.export_at(export_index)?)?;
    let mut values = read_texture_properties(&mut Reader::new(raw), package)?.animation;
    if let Some(anim_next) = &animation.anim_next {
        values.anim_next = resolve_animation_reference(package, export_index, anim_next)?;
    }
    if let Some(value) = animation.max_frame_rate {
        values.max_frame_rate = value;
    }
    if let Some(value) = animation.min_frame_rate {
        values.min_frame_rate = value;
    }
    if let Some(value) = animation.one_time_anim_loop {
        values.one_time_anim_loop = value;
    }
    if let Some(value) = animation.prime_count {
        values.prime_count = value;
    }
    if let Some(value) = animation.total_frame_num {
        values.total_frame_num = value;
    }
    Ok(values)
}

fn apply_animation_metadata_batch(
    working: Vec<u8>,
    package: &Package,
    updates: Vec<(usize, ImportedAnimation)>,
) -> UtxResult<Vec<u8>> {
    if updates.is_empty() {
        return Ok(working);
    }
    let mut exports = package.exports.clone();
    let mut rewritten = Vec::with_capacity(updates.len());
    let mut seen = HashSet::new();
    for (export_index, animation) in updates {
        if !seen.insert(export_index) {
            continue;
        }
        let values = merged_animation_values(package, export_index, &animation)?;
        let export = package.export_at(export_index)?;
        let raw = package.export_data(export)?;
        let mip = package.mip0_location(export_index)?;
        rewritten.push((
            export_index,
            rewrite_texture_animation_properties(raw, package, values)?,
            mip,
        ));
    }
    let mut output = working;
    for (export_index, mut data, mip) in rewritten {
        let export = exports
            .get_mut(export_index)
            .ok_or("Exportação de textura inválida.")?;
        let pointer_offset = data
            .mip_count_offset
            .checked_add(
                mip.width_offset_position
                    .checked_sub(data.original_mip_count_offset)
                    .ok_or("Offset de mip inválido.")?,
            )
            .ok_or("Offset de mip inválido.")?;
        let pixel_offset = data
            .mip_count_offset
            .checked_add(
                mip.pixel_offset
                    .checked_sub(data.original_mip_count_offset)
                    .ok_or("Offset de mip inválido.")?,
            )
            .ok_or("Offset de mip inválido.")?;
        let width_offset_value = output
            .len()
            .checked_add(pixel_offset)
            .and_then(|offset| offset.checked_add(mip.size))
            .ok_or("Offset de mip inválido.")?;
        write_i32_at(
            &mut data.bytes,
            pointer_offset,
            checked_i32(width_offset_value, "O pacote excede o limite de tamanho.")?,
        )?;
        export.offset = checked_i32(output.len(), "O pacote excede o limite de 2 GB.")?;
        export.size = checked_i32(
            data.bytes.len(),
            "A textura é grande demais para este formato.",
        )?;
        output.extend_from_slice(&data.bytes);
    }
    let name_table = serialize_name_table(&package.names)?;
    let import_table = serialize_import_table(&package.imports);
    let export_table = serialize_export_table(&exports);
    let name_offset = output.len();
    let import_offset = name_offset
        .checked_add(name_table.len())
        .ok_or("Tamanho de pacote inválido.")?;
    let export_offset = import_offset
        .checked_add(import_table.len())
        .ok_or("Tamanho de pacote inválido.")?;
    output.extend_from_slice(&name_table);
    output.extend_from_slice(&import_table);
    output.extend_from_slice(&export_table);
    write_i32_at(
        &mut output,
        12,
        checked_i32(package.names.len(), "Muitos nomes no pacote.")?,
    )?;
    write_i32_at(
        &mut output,
        16,
        checked_i32(name_offset, "Pacote grande demais.")?,
    )?;
    write_i32_at(
        &mut output,
        20,
        checked_i32(exports.len(), "Muitas exportações no pacote.")?,
    )?;
    write_i32_at(
        &mut output,
        24,
        checked_i32(export_offset, "Pacote grande demais.")?,
    )?;
    write_i32_at(
        &mut output,
        28,
        checked_i32(package.imports.len(), "Muitas importações no pacote.")?,
    )?;
    write_i32_at(
        &mut output,
        32,
        checked_i32(import_offset, "Pacote grande demais.")?,
    )?;
    Ok(output)
}

fn validate_export_extension(path: &Path, format: UtxFormat) -> UtxResult<()> {
    let wanted = format.export_extension();
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| !extension.eq_ignore_ascii_case(wanted))
    {
        return Err(format!("Esta textura deve ser exportada como .{wanted}."));
    }
    Ok(())
}

fn format_label(format: UtxFormat) -> &'static str {
    match format {
        UtxFormat::P8 => "P8",
        UtxFormat::Rgba7 => "RGBA7",
        UtxFormat::Rgb16 => "RGB16",
        UtxFormat::Dxt1 => "DXT1",
        UtxFormat::Rgb8 => "RGB8",
        UtxFormat::Rgba8 => "RGBA8",
        UtxFormat::NoData => "NoData",
        UtxFormat::Dxt3 => "DXT3",
        UtxFormat::Dxt5 => "DXT5",
        UtxFormat::L8 => "L8",
        UtxFormat::G16 => "G16",
        UtxFormat::Unknown => "Unknown",
    }
}

fn package_prefix(name: &str) -> Option<&str> {
    name.split_once('.')
        .map(|(prefix, _)| prefix)
        .filter(|prefix| !prefix.is_empty())
}

fn texture_leaf_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn texture_in_package(name: &str, package_name: &str) -> bool {
    if package_name.eq_ignore_ascii_case("Pacote principal") {
        !name.contains('.')
    } else {
        package_prefix(name).is_some_and(|prefix| prefix.eq_ignore_ascii_case(package_name))
    }
}

fn texture_import_key(package_name: &str, texture_name: &str) -> String {
    format!(
        "{}\0{}",
        package_name.to_ascii_lowercase(),
        texture_name.to_ascii_lowercase()
    )
}

fn file_name_for_export(name: &str) -> String {
    name.rsplit('.')
        .next()
        .unwrap_or(name)
        .chars()
        .map(|character| {
            if matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
struct DecryptedPackage {
    working: Vec<u8>,
    encrypted: bool,
    xor_key: u8,
    header: Vec<u8>,
}

fn decrypt_raw(file_path: &Path) -> UtxResult<DecryptedPackage> {
    let raw =
        fs::read(file_path).map_err(|error| format!("Não foi possível ler o pacote: {error}"))?;
    if raw.len() >= 28 {
        let header_units: Vec<u16> = raw[..28]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        let header_text = String::from_utf16_lossy(&header_units);
        if header_text.starts_with("Lineage2Ver") {
            let version: u32 = header_text
                .chars()
                .skip(11)
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>()
                .parse()
                .map_err(|_| "Cabeçalho de criptografia Lineage 2 inválido.".to_string())?;
            let xor_key = match version {
                111 => 0x2c,
                121 => compute_key_121(
                    file_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default(),
                ),
                _ => {
                    return Err(format!(
                        "A criptografia Lineage 2 v{version} não é suportada."
                    ))
                }
            };
            return Ok(DecryptedPackage {
                working: raw[28..].iter().map(|byte| byte ^ xor_key).collect(),
                encrypted: true,
                xor_key,
                header: raw[..28].to_vec(),
            });
        }
    }
    Ok(DecryptedPackage {
        working: raw,
        encrypted: false,
        xor_key: 0,
        header: Vec::new(),
    })
}

fn re_encrypt(working: Vec<u8>, decrypted: &DecryptedPackage) -> Vec<u8> {
    if !decrypted.encrypted {
        return working;
    }
    let mut output = decrypted.header.clone();
    output.extend(working.into_iter().map(|byte| byte ^ decrypted.xor_key));
    output
}

fn compute_key_121(file_name: &str) -> u8 {
    file_name
        .to_lowercase()
        .encode_utf16()
        .fold(0u32, |sum, unit| sum.wrapping_add(unit as u32)) as u8
}

#[derive(Debug, Clone)]
struct NameEntry {
    name: String,
    flags: i32,
}
#[derive(Debug, Clone)]
struct ImportEntry {
    class_package: i32,
    class_name: i32,
    package: i32,
    name_index: i32,
}
#[derive(Debug, Clone)]
struct ExportEntry {
    class: i32,
    super_class: i32,
    package: i32,
    name_index: i32,
    flags: i32,
    size: i32,
    offset: i32,
}

#[derive(Clone)]
struct Package {
    data: Vec<u8>,
    version: i32,
    licensee: i32,
    name_offset: usize,
    names: Vec<NameEntry>,
    imports: Vec<ImportEntry>,
    exports: Vec<ExportEntry>,
}

impl Package {
    fn parse(data: Vec<u8>) -> UtxResult<Self> {
        let mut reader = Reader::new(&data);
        if reader.read_i32()? != PACKAGE_MAGIC {
            return Err("O arquivo não é um pacote Unreal válido (assinatura incorreta).".into());
        }
        let version_licensee = reader.read_i32()?;
        let version = version_licensee & 0xffff;
        let licensee = (version_licensee >> 16) & 0xffff;
        reader.skip(4)?;
        let name_count = read_count(reader.read_i32()?, "nome")?;
        let name_offset = read_offset(reader.read_i32()?, "tabela de nomes")?;
        let export_count = read_count(reader.read_i32()?, "exportação")?;
        let export_offset = read_offset(reader.read_i32()?, "tabela de exportações")?;
        let import_count = read_count(reader.read_i32()?, "importação")?;
        let import_offset = read_offset(reader.read_i32()?, "tabela de importações")?;

        reader.seek(name_offset)?;
        let mut names = Vec::with_capacity(name_count);
        for _ in 0..name_count {
            let name = reader.read_unreal_string()?;
            let flags = reader.read_i32()?;
            names.push(NameEntry { name, flags });
        }
        reader.seek(import_offset)?;
        let mut imports = Vec::with_capacity(import_count);
        for _ in 0..import_count {
            let class_package = reader.read_compact()?;
            let class_name = reader.read_compact()?;
            imports.push(ImportEntry {
                class_package,
                class_name,
                package: reader.read_i32()?,
                name_index: reader.read_compact()?,
            });
        }
        let standard_exports =
            read_export_table(&data, export_offset, export_count, names.len(), false);
        let legacy_exports =
            || read_export_table(&data, export_offset, export_count, names.len(), true);
        let exports = match standard_exports {
            Ok((exports, end)) if end == data.len() => exports,
            Ok((exports, _)) => match legacy_exports() {
                Ok((legacy, end)) if end == data.len() => legacy,
                _ => exports,
            },
            Err(standard_error) => legacy_exports()
                .map(|(exports, _)| exports)
                .map_err(|_| standard_error)?,
        };
        Ok(Self {
            data,
            version,
            licensee,
            name_offset,
            names,
            imports,
            exports,
        })
    }

    fn name(&self, index: i32) -> &str {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.names.get(index))
            .map(|entry| entry.name.as_str())
            .unwrap_or("")
    }

    fn find_texture_in_package(
        &self,
        package_name: &str,
        texture_name: &str,
    ) -> UtxResult<Option<usize>> {
        for export_index in 0..self.exports.len() {
            let entry = match self.entry_to_model(export_index) {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if texture_in_package(&entry.name, package_name)
                && texture_leaf_name(&entry.name).eq_ignore_ascii_case(texture_name)
            {
                return Ok(Some(export_index));
            }
        }
        Ok(None)
    }

    fn texture_import_index(&self) -> UtxResult<HashMap<String, UtxEntry>> {
        let mut textures = HashMap::new();
        for (export_index, export) in self.exports.iter().enumerate() {
            if !self
                .full_class_name(export)
                .is_ok_and(|name| name.eq_ignore_ascii_case("Engine.Texture"))
            {
                continue;
            }
            let entry = self.entry_to_model(export_index)?;
            let group_name = package_prefix(&entry.name).unwrap_or("Pacote principal");
            textures
                .entry(texture_import_key(
                    group_name,
                    texture_leaf_name(&entry.name),
                ))
                .or_insert(entry);
        }
        Ok(textures)
    }

    fn group_outer_for_name(&self, group_name: &str) -> UtxResult<Option<i32>> {
        if group_name.eq_ignore_ascii_case("Pacote principal") {
            return Ok(Some(0));
        }
        for export_index in 0..self.exports.len() {
            match self.entry_to_model(export_index) {
                Ok(entry) if texture_in_package(&entry.name, group_name) => {}
                _ => continue,
            }
            let export = self.export_at(export_index)?;
            return Ok(Some(export.package));
        }
        for (export_index, export) in self.exports.iter().enumerate() {
            if !self
                .full_class_name(export)
                .is_ok_and(|class_name| class_name.eq_ignore_ascii_case("Core.Package"))
            {
                continue;
            }
            if self
                .inner_name(export)
                .is_ok_and(|name| name.eq_ignore_ascii_case(group_name))
            {
                return Ok(Some(export_reference(export_index)?));
            }
        }
        Ok(None)
    }

    fn group_template(&self) -> Option<ExportEntry> {
        self.exports.iter().find_map(|export| {
            self.full_class_name(export)
                .ok()
                .filter(|class_name| class_name.eq_ignore_ascii_case("Core.Package"))
                .map(|_| export.clone())
        })
    }

    fn template_texture_for_import(
        &self,
        package_name: &str,
        requires_split9: bool,
        requires_animation: bool,
    ) -> UtxResult<(usize, UtxEntry)> {
        let mut fallback = None;
        for export_index in 0..self.exports.len() {
            let entry = match self.entry_to_model(export_index) {
                Ok(entry) if entry.has_split9 == requires_split9 => entry,
                _ => continue,
            };
            let animated = self.texture_has_active_anim_next(export_index)?;
            if animated != requires_animation {
                continue;
            }
            if texture_in_package(&entry.name, package_name) {
                return Ok((export_index, entry));
            }
            fallback.get_or_insert((export_index, entry));
        }
        if let Some(template) = fallback {
            return Ok(template);
        }
        if requires_split9 && requires_animation {
            return Err(
                "O UTX não possui uma textura Split9 animada para usar como modelo estrutural."
                    .into(),
            );
        }
        if requires_split9 {
            return Err(
                "O UTX não possui uma textura Split9 sem AnimNext para usar como modelo estrutural."
                    .into(),
            );
        }
        if requires_animation {
            return Err(
                "O UTX não possui uma textura animada para usar como modelo estrutural.".into(),
            );
        }
        Err(
            "O UTX não possui uma textura comum sem AnimNext para usar como modelo estrutural."
                .into(),
        )
    }

    fn is_compatible_with_embedded_template(&self, template: &Package) -> bool {
        let Some(package_name_index) = template.names.iter().position(|entry| {
            entry
                .name
                .eq_ignore_ascii_case(NEW_UTX_TEMPLATE_PACKAGE_NAME)
        }) else {
            return false;
        };
        if self.names.len() < template.names.len() || self.imports.len() < template.imports.len() {
            return false;
        }
        let names_match = template.names.iter().enumerate().all(|(index, entry)| {
            index == package_name_index
                || self.names.get(index).is_some_and(|candidate| {
                    candidate.name.eq_ignore_ascii_case(&entry.name)
                        && candidate.flags == entry.flags
                })
        });
        let imports_match =
            template
                .imports
                .iter()
                .zip(&self.imports)
                .all(|(expected, candidate)| {
                    candidate.class_package == expected.class_package
                        && candidate.class_name == expected.class_name
                        && candidate.package == expected.package
                        && candidate.name_index == expected.name_index
                });
        names_match && imports_match
    }

    fn texture_has_active_anim_next(&self, export_index: usize) -> UtxResult<bool> {
        let export = self.export_at(export_index)?;
        let layout = texture_layout(self.export_data(export)?, self)?;
        Ok(layout.anim_next.is_some_and(|reference| reference != 0))
    }

    fn export_at(&self, index: usize) -> UtxResult<&ExportEntry> {
        self.exports
            .get(index)
            .ok_or_else(|| "A textura selecionada não existe mais no pacote.".into())
    }
    fn export_data(&self, export: &ExportEntry) -> UtxResult<&[u8]> {
        let start = read_offset(export.offset, "dados da exportação")?;
        let size = read_count(export.size, "tamanho da exportação")?;
        self.data
            .get(
                start
                    ..start
                        .checked_add(size)
                        .ok_or("Offset de exportação inválido.")?,
            )
            .ok_or_else(|| "Os dados da exportação estão fora do arquivo.".into())
    }
    fn inner_name(&self, export: &ExportEntry) -> UtxResult<String> {
        self.inner_name_depth(export, 0)
    }
    fn inner_name_depth(&self, export: &ExportEntry, depth: usize) -> UtxResult<String> {
        if depth > 128 {
            return Err("A hierarquia de objetos contém uma referência cíclica.".into());
        }
        let name = self.name(export.name_index).to_string();
        if export.package > 0 {
            let parent = self
                .exports
                .get(object_index(export.package)?)
                .ok_or("Pacote pai inválido.")?;
            return Ok(format!(
                "{}.{}",
                self.inner_name_depth(parent, depth + 1)?,
                name
            ));
        }
        Ok(name)
    }
    fn full_class_name(&self, export: &ExportEntry) -> UtxResult<String> {
        if export.class == 0 {
            return Ok("Core.Class".into());
        }
        if export.class > 0 {
            return self.inner_name(
                self.exports
                    .get(object_index(export.class)?)
                    .ok_or("Classe de exportação inválida.")?,
            );
        }
        self.import_path(object_index(export.class)?, 0)
    }
    fn import_path(&self, index: usize, depth: usize) -> UtxResult<String> {
        if depth > 128 {
            return Err("A hierarquia de imports contém uma referência cíclica.".into());
        }
        let import = self
            .imports
            .get(index)
            .ok_or("Referência de import inválida.")?;
        let name = self.name(import.name_index).to_string();
        if import.package == 0 {
            return Ok(name);
        }
        if import.package < 0 {
            return Ok(format!(
                "{}.{}",
                self.import_path(object_index(import.package)?, depth + 1)?,
                name
            ));
        }
        let parent = self
            .exports
            .get(object_index(import.package)?)
            .ok_or("Export pai inválido.")?;
        Ok(format!(
            "{}.{}",
            self.inner_name_depth(parent, depth + 1)?,
            name
        ))
    }
    fn object_reference_path(&self, reference: i32) -> UtxResult<String> {
        if reference > 0 {
            return self.inner_name(
                self.exports
                    .get(object_index(reference)?)
                    .ok_or("Referência de animação inválida.")?,
            );
        }
        if reference < 0 {
            return self.import_path(object_index(reference)?, 0);
        }
        Err("Referência de animação nula inválida.".into())
    }
    fn entry_to_model(&self, export_index: usize) -> UtxResult<UtxEntry> {
        let export = self.export_at(export_index)?;
        if !self
            .full_class_name(export)?
            .eq_ignore_ascii_case("Engine.Texture")
        {
            return Err("A entrada selecionada não é uma textura Engine.Texture.".into());
        }
        let raw = self.export_data(export)?;
        let mut reader = Reader::new(raw);
        let mut texture = read_texture_properties(&mut reader, self)?;
        let mip = self.mip0_from_reader(&mut reader)?;
        if texture.width <= 0 {
            texture.width = mip.width;
        }
        if texture.height <= 0 {
            texture.height = mip.height;
        }
        if texture.format == UtxFormat::Unknown {
            texture.format = detect_format(mip.size, texture.width, texture.height);
        }
        let animation = if texture.animation.is_active() {
            Some(ExportedAnimation {
                anim_next: (texture.animation.anim_next != 0)
                    .then(|| self.object_reference_path(texture.animation.anim_next).ok())
                    .flatten(),
                values: texture.animation,
                properties: texture.animation_properties,
            })
        } else {
            None
        };
        Ok(UtxEntry {
            name: self.inner_name(export)?,
            format: texture.format,
            export_index,
            width: texture.width,
            height: texture.height,
            has_alpha: texture.settings.alpha.unwrap_or(false),
            has_split9: texture.has_split9,
            split9_x1: texture.split9.x1,
            split9_x2: texture.split9.x2,
            split9_x3: texture.split9.x3,
            split9_y1: texture.split9.y1,
            split9_y2: texture.split9.y2,
            split9_y3: texture.split9.y3,
            settings: texture.settings,
            animation,
        })
    }
    fn scan_entries(&self) -> UtxResult<Vec<UtxEntry>> {
        let mut entries = Vec::new();
        for index in 0..self.exports.len() {
            if self
                .full_class_name(&self.exports[index])
                .is_ok_and(|name| name.eq_ignore_ascii_case("Engine.Texture"))
            {
                if let Ok(entry) = self.entry_to_model(index) {
                    entries.push(entry);
                }
            }
        }
        entries.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        });
        Ok(entries)
    }
    fn mip0_location(&self, export_index: usize) -> UtxResult<MipLocation> {
        let raw = self.export_data(self.export_at(export_index)?)?;
        let mut reader = Reader::new(raw);
        read_texture_properties(&mut reader, self)?;
        let mut location = self.mip0_from_reader(&mut reader)?;
        location.absolute_pixel_offset =
            read_offset(self.export_at(export_index)?.offset, "dados de textura")?
                .checked_add(location.pixel_offset)
                .ok_or("Offset de textura inválido.")?;
        Ok(location)
    }
    fn mip0_pixels(&self, export_index: usize) -> UtxResult<Vec<u8>> {
        let location = self.mip0_location(export_index)?;
        let export = self.export_at(export_index)?;
        let raw = self.export_data(export)?;
        raw.get(location.pixel_offset..location.pixel_offset + location.size)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| "Dados de mip truncados.".into())
    }
    fn mip0_from_reader(&self, reader: &mut Reader<'_>) -> UtxResult<MipLocation> {
        skip_unreal_extra(reader, self.version, self.licensee)?;
        let mip_count = reader.read_u8()?;
        if mip_count == 0 {
            return Err("A textura não contém mip maps.".into());
        }
        let width_offset_position = reader.position();
        reader.skip(4)?;
        let size = read_count(reader.read_compact()?, "mip")?;
        let pixel_offset = reader.position();
        reader.skip(size)?;
        let width = reader.read_i32()?;
        let height = reader.read_i32()?;
        reader.skip(2)?;
        Ok(MipLocation {
            pixel_offset,
            absolute_pixel_offset: 0,
            width_offset_position,
            size,
            width,
            height,
        })
    }
    fn export_bytes(&self, entry: &UtxEntry) -> UtxResult<Vec<u8>> {
        let pixels = self.mip0_pixels(entry.export_index)?;
        if entry.format == UtxFormat::Rgba8 {
            build_tga(&pixels, entry.width, entry.height)
        } else if entry.format.is_dxt() {
            build_dds(&pixels, entry.width, entry.height, entry.format)
        } else {
            Ok(pixels)
        }
    }
}

struct MipLocation {
    pixel_offset: usize,
    absolute_pixel_offset: usize,
    width_offset_position: usize,
    size: usize,
    width: i32,
    height: i32,
}
#[derive(Clone, Copy)]
struct TextureProperties {
    format: UtxFormat,
    width: i32,
    height: i32,
    settings: TextureSettings,
    has_split9: bool,
    split9: Split9,
    animation: AnimationValues,
    animation_properties: AnimationPropertyPresence,
}

fn read_export_table(
    data: &[u8],
    export_offset: usize,
    export_count: usize,
    name_count: usize,
    legacy_zero_size_offset: bool,
) -> UtxResult<(Vec<ExportEntry>, usize)> {
    let mut reader = Reader::new(data);
    reader.seek(export_offset)?;
    let mut exports = Vec::with_capacity(export_count);
    for _ in 0..export_count {
        let class = reader.read_compact()?;
        let super_class = reader.read_compact()?;
        let package = reader.read_i32()?;
        let name_index = reader.read_compact()?;
        if usize::try_from(name_index)
            .ok()
            .is_none_or(|index| index >= name_count)
        {
            return Err("Índice de nome inválido na tabela de exportações.".into());
        }
        let flags = reader.read_i32()?;
        let size = reader.read_compact()?;
        if size < 0 {
            return Err("Tamanho de exportação inválido.".into());
        }
        let offset = if size > 0 || legacy_zero_size_offset {
            reader.read_compact()?
        } else {
            0
        };
        exports.push(ExportEntry {
            class,
            super_class,
            package,
            name_index,
            flags,
            size,
            offset,
        });
    }
    Ok((exports, reader.position()))
}

fn read_texture_properties(
    reader: &mut Reader<'_>,
    package: &Package,
) -> UtxResult<TextureProperties> {
    let mut format = UtxFormat::Unknown;
    let mut width = 0;
    let mut height = 0;
    let mut u_bits = 0;
    let mut v_bits = 0;
    let mut settings = TextureSettings::default();
    let mut has_split9 = false;
    let mut split9 = Split9 {
        x1: 0,
        x2: 0,
        x3: 0,
        y1: 0,
        y2: 0,
        y3: 0,
    };
    let mut animation = AnimationValues::default();
    let mut animation_properties = AnimationPropertyPresence::default();
    loop {
        let name = package.name(reader.read_compact()?).to_ascii_lowercase();
        if name == "none" {
            break;
        }
        let info = reader.read_u8()?;
        let property_type = info & 0x0f;
        let size_type = (info >> 4) & 0x07;
        let is_array = info & 0x80 != 0;
        if property_type == 10 {
            reader.read_compact()?;
        }
        let size = property_size(reader, size_type)?;
        if is_array && property_type != 3 {
            reader.read_compact()?;
        }
        let start = reader.position();
        match name.as_str() {
            "format" if size >= 1 => format = UtxFormat::from_value(reader.read_u8()?),
            "usize" if size == 2 => width = reader.read_u16()? as i32,
            "usize" if size == 4 => width = reader.read_i32()?,
            "vsize" if size == 2 => height = reader.read_u16()? as i32,
            "vsize" if size == 4 => height = reader.read_i32()?,
            "ubits" if size >= 1 => u_bits = reader.read_u8()? as i32,
            "vbits" if size >= 1 => v_bits = reader.read_u8()? as i32,
            "balphatexture" if property_type == 3 => settings.alpha = Some(is_array),
            "bmasked" if property_type == 3 => settings.masked = Some(is_array),
            "uclamp" => settings.u_clamp = read_property_integer(reader, size)?,
            "vclamp" => settings.v_clamp = read_property_integer(reader, size)?,
            "uclampmode" => settings.u_clamp_mode = read_property_integer(reader, size)?,
            "vclampmode" => settings.v_clamp_mode = read_property_integer(reader, size)?,
            "bsplit9texture" if property_type == 3 => has_split9 = is_array,
            "split9x1" if size == 4 => split9.x1 = reader.read_i32()?,
            "split9x2" if size == 4 => split9.x2 = reader.read_i32()?,
            "split9x3" if size == 4 => split9.x3 = reader.read_i32()?,
            "split9y1" if size == 4 => split9.y1 = reader.read_i32()?,
            "split9y2" if size == 4 => split9.y2 = reader.read_i32()?,
            "split9y3" if size == 4 => split9.y3 = reader.read_i32()?,
            "animnext" if size > 0 => {
                let value = reader
                    .data
                    .get(
                        start
                            ..start
                                .checked_add(size)
                                .ok_or("Offset de animação inválido.")?,
                    )
                    .ok_or("Dados de animação truncados.")?;
                animation.anim_next = Reader::new(value).read_compact()?;
                animation_properties.anim_next = true;
            }
            "maxframerate" if size == 4 => {
                animation.max_frame_rate = f32::from_bits(reader.read_i32()? as u32);
                animation_properties.max_frame_rate = true;
            }
            "minframerate" if size == 4 => {
                animation.min_frame_rate = f32::from_bits(reader.read_i32()? as u32);
                animation_properties.min_frame_rate = true;
            }
            "onetimeanimloop" | "banimloop" if property_type == 3 => {
                animation.one_time_anim_loop = is_array;
                animation_properties.one_time_anim_loop = true;
            }
            "primecount" if size == 1 => {
                animation.prime_count = reader.read_u8()? as i32;
                animation_properties.prime_count = true;
            }
            "primecount" if size == 4 => {
                animation.prime_count = reader.read_i32()?;
                animation_properties.prime_count = true;
            }
            "totalframenum" if size == 4 => {
                animation.total_frame_num = reader.read_i32()?;
                animation_properties.total_frame_num = true;
            }
            _ => {}
        }
        let consumed = reader.position() - start;
        if consumed < size {
            reader.skip(size - consumed)?;
        }
    }
    if width <= 0 && u_bits > 0 {
        width = 1_i32.checked_shl(u_bits as u32).unwrap_or(0);
    }
    if height <= 0 && v_bits > 0 {
        height = 1_i32.checked_shl(v_bits as u32).unwrap_or(0);
    }
    Ok(TextureProperties {
        format,
        width,
        height,
        settings,
        has_split9,
        split9,
        animation,
        animation_properties,
    })
}

fn read_property_integer(reader: &mut Reader<'_>, size: usize) -> UtxResult<Option<i32>> {
    match size {
        1 => Ok(Some(reader.read_u8()? as i32)),
        2 => Ok(Some(reader.read_u16()? as i32)),
        4 => Ok(Some(reader.read_i32()?)),
        _ => Ok(None),
    }
}

fn property_size(reader: &mut Reader<'_>, size_type: u8) -> UtxResult<usize> {
    match size_type {
        0 => Ok(1),
        1 => Ok(2),
        2 => Ok(4),
        3 => Ok(12),
        4 => Ok(16),
        5 => Ok(reader.read_u8()? as usize),
        6 => Ok(reader.read_u16()? as usize),
        7 => read_count(reader.read_i32()?, "propriedade"),
        _ => Err("Tipo de tamanho de propriedade inválido.".into()),
    }
}

fn skip_unreal_extra(reader: &mut Reader<'_>, version: i32, licensee: i32) -> UtxResult<()> {
    if licensee <= 10 {
        return Ok(());
    }
    if licensee <= 28 {
        return reader.skip(4);
    }
    if licensee <= 32 {
        return Ok(());
    }
    if licensee <= 35 {
        reader.skip(1067)?;
        for _ in 0..17 {
            reader.read_unreal_string()?;
        }
        return reader.skip(4);
    }
    if licensee == 36 {
        reader.skip(1058)?;
        for _ in 0..17 {
            reader.read_unreal_string()?;
        }
        return reader.skip(4);
    }
    reader.skip(if licensee <= 39 && version != 129 {
        36
    } else {
        92
    })?;
    let count = read_count(reader.read_compact()?, "metadados")?;
    for _ in 0..count {
        reader.read_unreal_string()?;
        let extra = reader.read_u8()?;
        for _ in 0..extra {
            reader.read_unreal_string()?;
        }
    }
    reader.read_unreal_string()?;
    reader.skip(4)
}

fn detect_format(size: usize, width: i32, height: i32) -> UtxFormat {
    let (width, height) = match (usize::try_from(width), usize::try_from(height)) {
        (Ok(width), Ok(height)) if width > 0 && height > 0 => (width, height),
        _ => return UtxFormat::Unknown,
    };
    let blocks = width.div_ceil(4).saturating_mul(height.div_ceil(4));
    if size == blocks.saturating_mul(8) {
        UtxFormat::Dxt1
    } else if size == blocks.saturating_mul(16) {
        UtxFormat::Dxt3
    } else if size == width.saturating_mul(height).saturating_mul(4) {
        UtxFormat::Rgba8
    } else if size == width.saturating_mul(height) {
        UtxFormat::P8
    } else {
        UtxFormat::Unknown
    }
}

fn build_tga(pixels: &[u8], width: i32, height: i32) -> UtxResult<Vec<u8>> {
    let width = u16::try_from(width).map_err(|_| "Largura inválida para TGA.")?;
    let height = u16::try_from(height).map_err(|_| "Altura inválida para TGA.")?;
    let expected = usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|count| count.checked_mul(4))
        .ok_or("Textura muito grande.")?;
    if pixels.len() < expected {
        return Err("Dados RGBA8 truncados.".into());
    }
    let mut output = Vec::with_capacity(18 + expected);
    output.extend_from_slice(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    output.extend_from_slice(&[32, 0x28]);
    output.extend_from_slice(&pixels[..expected]);
    Ok(output)
}

fn build_dds(pixels: &[u8], width: i32, height: i32, format: UtxFormat) -> UtxResult<Vec<u8>> {
    if width <= 0 || height <= 0 {
        return Err("Dimensões DXT inválidas.".into());
    }
    let block_bytes = if format == UtxFormat::Dxt1 {
        8usize
    } else {
        16usize
    };
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height).ok().map(|height| {
                width
                    .div_ceil(4)
                    .saturating_mul(height.div_ceil(4).saturating_mul(block_bytes))
            })
        })
        .ok_or("Dimensões DXT inválidas.")?;
    if pixels.len() < expected {
        return Err("Dados DXT truncados.".into());
    }
    let mut output = vec![0u8; 128];
    output[..4].copy_from_slice(b"DDS ");
    write_i32_at(&mut output, 4, 124)?;
    write_i32_at(&mut output, 8, 0x0008_1007)?;
    write_i32_at(&mut output, 12, height)?;
    write_i32_at(&mut output, 16, width)?;
    write_i32_at(
        &mut output,
        20,
        i32::try_from(expected).map_err(|_| "Textura DXT muito grande.")?,
    )?;
    write_i32_at(&mut output, 76, 32)?;
    write_i32_at(&mut output, 80, 4)?;
    output[84..88].copy_from_slice(match format {
        UtxFormat::Dxt3 => b"DXT3",
        UtxFormat::Dxt5 => b"DXT5",
        _ => b"DXT1",
    });
    write_i32_at(&mut output, 108, 0x1000)?;
    output.extend_from_slice(&pixels[..expected]);
    Ok(output)
}

fn tga_dimensions(data: &[u8]) -> UtxResult<(i32, i32)> {
    if data.len() < 18 {
        return Err("Arquivo TGA inválido ou corrompido.".into());
    }
    let width = u16::from_le_bytes([data[12], data[13]]) as i32;
    let height = u16::from_le_bytes([data[14], data[15]]) as i32;
    if width <= 0 || height <= 0 {
        return Err("O TGA não possui dimensões válidas.".into());
    }
    Ok((width, height))
}

fn tga_pixel_start(data: &[u8]) -> UtxResult<usize> {
    if data.len() < 18 {
        return Err("Arquivo TGA inválido ou corrompido.".into());
    }
    let color_map_bytes = if data[1] == 1 {
        usize::from(u16::from_le_bytes([data[5], data[6]]))
            .checked_mul((data[7] as usize).div_ceil(8))
            .ok_or("Mapa de cores TGA inválido.")?
    } else {
        0
    };
    let start = 18usize
        .checked_add(data[0] as usize)
        .and_then(|value| value.checked_add(color_map_bytes))
        .ok_or("Offset de pixels TGA inválido.")?;
    if start > data.len() {
        return Err("Offset de pixels TGA inválido.".into());
    }
    Ok(start)
}

fn validate_dds(data: &[u8], width: i32, height: i32, format: UtxFormat) -> UtxResult<()> {
    if data.len() < 128 || &data[..4] != b"DDS " {
        return Err("O DXT requer um arquivo DDS válido.".into());
    }
    let file_height = i32::from_le_bytes(data[12..16].try_into().unwrap());
    let file_width = i32::from_le_bytes(data[16..20].try_into().unwrap());
    if file_width != width || file_height != height {
        return Err(format!(
            "Tamanho incompatível: esperado {width}×{height}, recebido {file_width}×{file_height}."
        ));
    }
    let expected = match format {
        UtxFormat::Dxt1 => b"DXT1".as_slice(),
        UtxFormat::Dxt3 => b"DXT3".as_slice(),
        UtxFormat::Dxt5 => b"DXT5".as_slice(),
        _ => return Err("Formato DXT inválido.".into()),
    };
    if &data[84..88] != expected {
        return Err(format!(
            "Formato DDS incompatível: esperado {}.",
            std::str::from_utf8(expected).unwrap_or("DXT")
        ));
    }
    Ok(())
}

fn encode_preview(
    pixels: &[u8],
    width: i32,
    height: i32,
    format: UtxFormat,
) -> UtxResult<TexturePreview> {
    let png_data = encode_png(pixels, width, height, format)?;
    Ok(TexturePreview {
        data_url: format!("data:image/png;base64,{}", BASE64.encode(png_data)),
        width,
        height,
    })
}

fn encode_png(pixels: &[u8], width: i32, height: i32, format: UtxFormat) -> UtxResult<Vec<u8>> {
    if width <= 0 || height <= 0 {
        return Err("A textura não possui dimensões válidas.".into());
    }
    let width_u = usize::try_from(width).map_err(|_| "Dimensões inválidas.")?;
    let height_u = usize::try_from(height).map_err(|_| "Dimensões inválidas.")?;
    let rgba = match format {
        UtxFormat::Rgba8 => bgra_to_rgba(pixels, width_u, height_u)?,
        UtxFormat::Dxt1 => decode_dxt1(pixels, width_u, height_u)?,
        UtxFormat::Dxt3 => decode_dxt3(pixels, width_u, height_u)?,
        UtxFormat::Dxt5 => decode_dxt5(pixels, width_u, height_u)?,
        _ => return Err("Formato de prévia não suportado.".into()),
    };
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|error| format!("Falha ao gerar a prévia PNG: {error}"))?;
        writer
            .write_image_data(&rgba)
            .map_err(|error| format!("Falha ao gerar a prévia PNG: {error}"))?;
    }
    Ok(png_data)
}

fn bgra_to_rgba(pixels: &[u8], width: usize, height: usize) -> UtxResult<Vec<u8>> {
    let needed = width
        .checked_mul(height)
        .and_then(|value| value.checked_mul(4))
        .ok_or("Textura muito grande.")?;
    let pixels = pixels.get(..needed).ok_or("Dados RGBA8 truncados.")?;
    let mut rgba = Vec::with_capacity(needed);
    for pixel in pixels.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    Ok(rgba)
}

fn decode_dxt1(source: &[u8], width: usize, height: usize) -> UtxResult<Vec<u8>> {
    decode_dxt(source, width, height, 1)
}
fn decode_dxt3(source: &[u8], width: usize, height: usize) -> UtxResult<Vec<u8>> {
    decode_dxt(source, width, height, 3)
}
fn decode_dxt5(source: &[u8], width: usize, height: usize) -> UtxResult<Vec<u8>> {
    decode_dxt(source, width, height, 5)
}

fn decode_dxt(source: &[u8], width: usize, height: usize, kind: u8) -> UtxResult<Vec<u8>> {
    let block_bytes = if kind == 1 { 8 } else { 16 };
    let block_width = width.div_ceil(4);
    let block_height = height.div_ceil(4);
    let required = block_width
        .checked_mul(block_height)
        .and_then(|blocks| blocks.checked_mul(block_bytes))
        .ok_or("Textura DXT muito grande.")?;
    if source.len() < required {
        return Err("Dados DXT truncados.".into());
    }
    let mut output = vec![
        0u8;
        width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("Textura muito grande.")?
    ];
    let mut offset = 0;
    for block_y in 0..block_height {
        for block_x in 0..block_width {
            let block = &source[offset..offset + block_bytes];
            offset += block_bytes;
            let (alpha, color_offset) = match kind {
                1 => ([255u8; 16], 0),
                3 => {
                    let mut alpha = [0u8; 16];
                    for index in 0..16 {
                        alpha[index] = if index % 2 == 0 {
                            (block[index / 2] & 0x0f) * 17
                        } else {
                            (block[index / 2] >> 4) * 17
                        };
                    }
                    (alpha, 8)
                }
                _ => {
                    let mut palette = [0u8; 8];
                    palette[0] = block[0];
                    palette[1] = block[1];
                    if palette[0] > palette[1] {
                        for index in 2..8 {
                            palette[index] = (((8 - index) as u16 * palette[0] as u16
                                + (index - 1) as u16 * palette[1] as u16)
                                / 7) as u8;
                        }
                    } else {
                        for index in 2..6 {
                            palette[index] = (((6 - index) as u16 * palette[0] as u16
                                + (index - 1) as u16 * palette[1] as u16)
                                / 5) as u8;
                        }
                        palette[6] = 0;
                        palette[7] = 255;
                    }
                    let bits = block[2..8]
                        .iter()
                        .enumerate()
                        .fold(0u64, |bits, (index, byte)| {
                            bits | ((*byte as u64) << (index * 8))
                        });
                    let mut alpha = [0u8; 16];
                    for index in 0..16 {
                        alpha[index] = palette[((bits >> (index * 3)) & 7) as usize];
                    }
                    (alpha, 8)
                }
            };
            let c0 = u16::from_le_bytes([block[color_offset], block[color_offset + 1]]);
            let c1 = u16::from_le_bytes([block[color_offset + 2], block[color_offset + 3]]);
            let palette = dxt_palette(c0, c1);
            let bits = u32::from_le_bytes([
                block[color_offset + 4],
                block[color_offset + 5],
                block[color_offset + 6],
                block[color_offset + 7],
            ]);
            for pixel_y in 0..4 {
                for pixel_x in 0..4 {
                    let x = block_x * 4 + pixel_x;
                    let y = block_y * 4 + pixel_y;
                    if x >= width || y >= height {
                        continue;
                    }
                    let index = pixel_y * 4 + pixel_x;
                    let color_index = ((bits >> (index * 2)) & 3) as usize;
                    let target = (y * width + x) * 4;
                    let transparent = kind == 1 && c0 <= c1 && color_index == 3;
                    if transparent {
                        output[target..target + 4].fill(0);
                    } else {
                        output[target..target + 3].copy_from_slice(&palette[color_index]);
                        output[target + 3] = alpha[index];
                    }
                }
            }
        }
    }
    Ok(output)
}

fn dxt_palette(c0: u16, c1: u16) -> [[u8; 3]; 4] {
    let unpack = |color: u16| {
        [
            ((color >> 11 & 31) * 255 / 31) as u8,
            ((color >> 5 & 63) * 255 / 63) as u8,
            ((color & 31) * 255 / 31) as u8,
        ]
    };
    let first = unpack(c0);
    let second = unpack(c1);
    let mut colors = [first, second, [0; 3], [0; 3]];
    for channel in 0..3 {
        if c0 > c1 {
            colors[2][channel] = ((2 * first[channel] as u16 + second[channel] as u16) / 3) as u8;
            colors[3][channel] = ((first[channel] as u16 + 2 * second[channel] as u16) / 3) as u8;
        } else {
            colors[2][channel] = ((first[channel] as u16 + second[channel] as u16) / 2) as u8;
        }
    }
    colors
}

fn read_count(value: i32, label: &str) -> UtxResult<usize> {
    usize::try_from(value).map_err(|_| format!("Contagem de {label} inválida no pacote."))
}
fn read_offset(value: i32, label: &str) -> UtxResult<usize> {
    usize::try_from(value).map_err(|_| format!("Offset de {label} inválido no pacote."))
}
fn object_index(reference: i32) -> UtxResult<usize> {
    if reference == 0 {
        return Err("Referência de objeto nula inválida.".into());
    }
    usize::try_from(
        reference
            .unsigned_abs()
            .checked_sub(1)
            .ok_or("Referência de objeto inválida.")?,
    )
    .map_err(|_| "Referência de objeto fora do limite.".into())
}

fn checked_i32(value: usize, message: &str) -> UtxResult<i32> {
    i32::try_from(value).map_err(|_| message.to_string())
}

fn write_compact(output: &mut Vec<u8>, value: i32) {
    let negative = value < 0;
    let mut magnitude = if negative {
        value.wrapping_neg() as u32
    } else {
        value as u32
    };
    let mut first = (magnitude & 0x3f) as u8;
    magnitude >>= 6;
    if negative {
        first |= 0x80;
    }
    if magnitude > 0 {
        first |= 0x40;
    }
    output.push(first);
    for index in 1..=3 {
        if magnitude == 0 {
            break;
        }
        let mut byte = (magnitude & 0x7f) as u8;
        magnitude >>= 7;
        if magnitude > 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if index == 3 && magnitude > 0 {
            output.push((magnitude & 0x1f) as u8);
            break;
        }
    }
}

fn write_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn serialize_name_table(entries: &[NameEntry]) -> UtxResult<Vec<u8>> {
    let mut output = Vec::new();
    for entry in entries {
        if entry.name.is_ascii() {
            write_compact(
                &mut output,
                checked_i32(entry.name.len() + 1, "Nome de textura muito longo.")?,
            );
            output.extend_from_slice(entry.name.as_bytes());
            output.push(0);
        } else {
            let units = entry.name.encode_utf16().collect::<Vec<_>>();
            let length = checked_i32(
                units
                    .len()
                    .checked_add(1)
                    .ok_or("Nome de textura muito longo.")?,
                "Nome de textura muito longo.",
            )?;
            write_compact(&mut output, -length);
            for unit in units {
                output.extend_from_slice(&unit.to_le_bytes());
            }
            output.extend_from_slice(&0u16.to_le_bytes());
        }
        write_i32(&mut output, entry.flags);
    }
    Ok(output)
}

fn serialize_import_table(entries: &[ImportEntry]) -> Vec<u8> {
    let mut output = Vec::new();
    for entry in entries {
        write_compact(&mut output, entry.class_package);
        write_compact(&mut output, entry.class_name);
        write_i32(&mut output, entry.package);
        write_compact(&mut output, entry.name_index);
    }
    output
}

fn serialize_export_table(entries: &[ExportEntry]) -> Vec<u8> {
    let mut output = Vec::new();
    for entry in entries {
        write_compact(&mut output, entry.class);
        write_compact(&mut output, entry.super_class);
        write_i32(&mut output, entry.package);
        write_compact(&mut output, entry.name_index);
        write_i32(&mut output, entry.flags);
        write_compact(&mut output, entry.size);
        if entry.size > 0 {
            write_compact(&mut output, entry.offset);
        }
    }
    output
}

fn write_i32_at(buffer: &mut [u8], position: usize, value: i32) -> UtxResult<()> {
    buffer
        .get_mut(position..position.checked_add(4).ok_or("Offset inválido.")?)
        .ok_or("Cabeçalho do pacote truncado.")?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    fn position(&self) -> usize {
        self.position
    }
    fn seek(&mut self, position: usize) -> UtxResult<()> {
        if position > self.data.len() {
            return Err("Leitura fora dos limites do pacote.".into());
        }
        self.position = position;
        Ok(())
    }
    fn skip(&mut self, size: usize) -> UtxResult<()> {
        self.seek(self.position.checked_add(size).ok_or("Offset inválido.")?)
    }
    fn read_u8(&mut self) -> UtxResult<u8> {
        let byte = *self
            .data
            .get(self.position)
            .ok_or("Dados do pacote truncados.")?;
        self.position += 1;
        Ok(byte)
    }
    fn read_u16(&mut self) -> UtxResult<u16> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    fn read_i32(&mut self) -> UtxResult<i32> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn read_compact(&mut self) -> UtxResult<i32> {
        let first = self.read_u8()?;
        let negative = first & 0x80 != 0;
        let mut output = (first & 0x3f) as u32;
        if first & 0x40 != 0 {
            for index in 1..=4 {
                let byte = self.read_u8()?;
                if index == 4 {
                    output |= ((byte & 0x1f) as u32) << 27;
                    break;
                }
                output |= ((byte & 0x7f) as u32) << (6 + (index - 1) * 7);
                if byte & 0x80 == 0 {
                    break;
                }
            }
        }
        if negative {
            Ok(if output == 0 {
                i32::MIN
            } else {
                (output as i32).wrapping_neg()
            })
        } else {
            i32::try_from(output)
                .map_err(|_| "Inteiro compacto fora do intervalo suportado.".into())
        }
    }
    fn read_unreal_string(&mut self) -> UtxResult<String> {
        let length = self.read_compact()?;
        if length == 0 {
            return Ok(String::new());
        }
        if length > 0 {
            let length = length as usize;
            let bytes = self.read_exact(length)?;
            return Ok(String::from_utf8_lossy(&bytes[..length.saturating_sub(1)]).into_owned());
        }
        let bytes =
            self.read_exact(length.checked_abs().ok_or("String Unreal inválida.")? as usize * 2)?;
        let units = bytes[..bytes.len().saturating_sub(2)]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        Ok(String::from_utf16_lossy(&units))
    }
    fn read_exact(&mut self, size: usize) -> UtxResult<&'a [u8]> {
        let end = self.position.checked_add(size).ok_or("Offset inválido.")?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or("Dados do pacote truncados.")?;
        self.position = end;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn accepts_unicode_file_names_for_existing_texture_replacements() {
        assert_eq!(
            imported_texture_name(Path::new("C:\\icons\\펫액션_01.dds")).unwrap(),
            "펫액션_01"
        );
    }

    #[test]
    fn embedded_template_has_independent_split9_and_animation_seeds() {
        let package = embedded_template_package().unwrap();
        let (animation_export_index, animation_entry) = (0..package.exports.len())
            .find_map(|index| {
                package
                    .entry_to_model(index)
                    .ok()
                    .filter(|entry| entry.name.ends_with(".TlpAnim"))
                    .map(|entry| (index, entry))
            })
            .expect("The embedded template must include TlpAnim");
        assert!(animation_entry.animation.is_some());
        let raw = package
            .export_data(package.export_at(animation_export_index).unwrap())
            .unwrap();
        let properties = read_texture_properties(&mut Reader::new(raw), &package).unwrap();
        assert_ne!(properties.animation.anim_next, 0);
        assert_ne!(properties.animation.max_frame_rate, 0.0);
        assert_ne!(properties.animation.min_frame_rate, 0.0);
        assert!(properties.animation.one_time_anim_loop);
        assert_ne!(properties.animation.prime_count, 0);
        assert_ne!(properties.animation.total_frame_num, 0);

        let split9_entry = (0..package.exports.len())
            .find_map(|index| {
                package
                    .entry_to_model(index)
                    .ok()
                    .filter(|entry| entry.name.ends_with(".TlpSpt9"))
            })
            .expect("The embedded template must include TlpSpt9");
        assert!(split9_entry.has_split9);
        assert_ne!(split9_entry.split9_x1, 0);
        assert_ne!(split9_entry.split9_x2, 0);
        assert_ne!(split9_entry.split9_x3, 0);
        assert_ne!(split9_entry.split9_y1, 0);
        assert_ne!(split9_entry.split9_y2, 0);
        assert_ne!(split9_entry.split9_y3, 0);
    }

    #[test]
    fn texture_metadata_exports_and_reads_alpha_masked_and_clamp() {
        let root = env::temp_dir().join(format!(
            "unreal_tools_metadata_{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let texture_path = root.join("button.tga");
        let entry = UtxEntry {
            name: "Buttons.button".into(),
            format: UtxFormat::Rgba8,
            export_index: 0,
            width: 32,
            height: 32,
            has_alpha: true,
            has_split9: false,
            split9_x1: 0,
            split9_x2: 0,
            split9_x3: 0,
            split9_y1: 0,
            split9_y2: 0,
            split9_y3: 0,
            settings: TextureSettings {
                alpha: Some(true),
                masked: Some(false),
                u_clamp: Some(64),
                v_clamp: Some(32),
                u_clamp_mode: Some(2),
                v_clamp_mode: Some(3),
            },
            animation: None,
        };
        write_texture_metadata_file(&entry, &texture_path).unwrap();
        let content = fs::read_to_string(texture_path.with_extension("txt")).unwrap();
        assert_eq!(
            content,
            "[Texture]\nAlpha=True\nMasked=False\nUClamp=64\nVClamp=32\nUClampMode=2\nVClampMode=3\n"
        );
        let metadata = parse_texture_metadata_file(&texture_path.with_extension("txt")).unwrap();
        assert_eq!(metadata.settings.alpha, Some(true));
        assert_eq!(metadata.settings.masked, Some(false));
        assert_eq!(metadata.settings.u_clamp, Some(64));
        assert_eq!(metadata.settings.v_clamp, Some(32));
        assert_eq!(metadata.settings.u_clamp_mode, Some(2));
        assert_eq!(metadata.settings.v_clamp_mode, Some(3));
        let request = texture_engine_import_request(ImportedTexture {
            name: "button".into(),
            format: UtxFormat::Rgba8,
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 255],
            metadata,
        })
        .unwrap();
        assert_eq!(request.alpha, Some(true));
        assert_eq!(request.masked, Some(false));
        assert_eq!(request.clamp.unwrap().u_clamp, Some(64));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn existing_texture_preserves_only_metadata_that_it_can_patch_in_place() {
        let mut texture = ImportedTexture {
            name: "button".into(),
            format: UtxFormat::Rgba8,
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 255],
            metadata: TextureMetadata {
                settings: TextureSettings {
                    alpha: Some(true),
                    masked: Some(false),
                    u_clamp: Some(64),
                    v_clamp: Some(64),
                    u_clamp_mode: Some(2),
                    v_clamp_mode: Some(3),
                },
                split9: Some(Split9 {
                    x1: 1,
                    x2: 2,
                    x3: 3,
                    y1: 4,
                    y2: 5,
                    y3: 6,
                }),
                animation: Some(ImportedAnimation {
                    prime_count: Some(1),
                    ..ImportedAnimation::default()
                }),
            },
        };
        let existing = UtxEntry {
            name: "Buttons.button".into(),
            format: UtxFormat::Rgba8,
            export_index: 0,
            width: 1,
            height: 1,
            has_alpha: false,
            has_split9: false,
            split9_x1: 0,
            split9_x2: 0,
            split9_x3: 0,
            split9_y1: 0,
            split9_y2: 0,
            split9_y3: 0,
            settings: TextureSettings {
                alpha: Some(false),
                u_clamp: Some(32),
                ..TextureSettings::default()
            },
            animation: None,
        };

        let preserved = preserve_unsupported_existing_metadata(&mut texture, &existing);

        assert_eq!(
            preserved,
            vec![
                "Masked",
                "VClamp",
                "UClampMode",
                "VClampMode",
                "Split9",
                "Animations"
            ]
        );
        assert_eq!(texture.metadata.settings.alpha, Some(true));
        assert_eq!(texture.metadata.settings.u_clamp, Some(64));
        assert_eq!(texture.metadata.settings.masked, None);
        assert_eq!(texture.metadata.settings.v_clamp, None);
        assert_eq!(texture.metadata.settings.u_clamp_mode, None);
        assert_eq!(texture.metadata.settings.v_clamp_mode, None);
        assert!(texture.metadata.split9.is_none());
        assert!(texture.metadata.animation.is_none());
    }

    #[test]
    fn existing_animated_texture_preserves_only_missing_animation_fields() {
        let mut texture = ImportedTexture {
            name: "animated".into(),
            format: UtxFormat::Rgba8,
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 255],
            metadata: TextureMetadata {
                animation: Some(ImportedAnimation {
                    anim_next: Some("Button.animated_next".into()),
                    max_frame_rate: Some(12.0),
                    min_frame_rate: Some(2.0),
                    one_time_anim_loop: Some(true),
                    prime_count: Some(3),
                    total_frame_num: Some(4),
                }),
                ..TextureMetadata::default()
            },
        };
        let existing = UtxEntry {
            name: "Button.animated".into(),
            format: UtxFormat::Rgba8,
            export_index: 0,
            width: 1,
            height: 1,
            has_alpha: false,
            has_split9: false,
            split9_x1: 0,
            split9_x2: 0,
            split9_x3: 0,
            split9_y1: 0,
            split9_y2: 0,
            split9_y3: 0,
            settings: TextureSettings::default(),
            animation: Some(ExportedAnimation {
                anim_next: Some("Button.animated_next".into()),
                values: AnimationValues {
                    anim_next: 1,
                    min_frame_rate: 1.0,
                    prime_count: 1,
                    ..AnimationValues::default()
                },
                properties: AnimationPropertyPresence {
                    anim_next: true,
                    min_frame_rate: true,
                    prime_count: true,
                    ..AnimationPropertyPresence::default()
                },
            }),
        };

        let preserved = preserve_unsupported_existing_metadata(&mut texture, &existing);
        let animation = texture.metadata.animation.unwrap();

        assert_eq!(
            preserved,
            vec!["MaxFrameRate", "OneTimeAnimLoop", "TotalFrameNum"]
        );
        assert_eq!(animation.anim_next.as_deref(), Some("Button.animated_next"));
        assert_eq!(animation.min_frame_rate, Some(2.0));
        assert_eq!(animation.prime_count, Some(3));
        assert_eq!(animation.max_frame_rate, None);
        assert_eq!(animation.one_time_anim_loop, None);
        assert_eq!(animation.total_frame_num, None);
    }

    #[test]
    fn decodes_a_single_dxt1_pixel() {
        let pixels = [0x00, 0xf8, 0x1f, 0x00, 0, 0, 0, 0];
        let preview = encode_preview(&pixels, 1, 1, UtxFormat::Dxt1).unwrap();
        assert!(preview.data_url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn creates_new_utx_from_embedded_editor_template() {
        let root = env::temp_dir().join(format!(
            "unreal-tools-utx-new-template-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let output = root.join("Walker.utx");
        let output_path = output.to_string_lossy().into_owned();

        create_new(&output_path).unwrap();
        let raw = fs::read(&output).unwrap();
        assert_eq!(&raw[..4], &PACKAGE_MAGIC.to_le_bytes());
        let package = Package::parse(raw).unwrap();
        assert!(package.names.iter().any(|entry| entry.name == "Walker"));
        assert!(!package
            .names
            .iter()
            .any(|entry| entry.name == NEW_UTX_TEMPLATE_PACKAGE_NAME));
        let entries = package.scan_entries().unwrap();
        assert!(entries.is_empty());
        assert!(package.exports.is_empty());
        assert!(package.is_compatible_with_embedded_template(&embedded_template_package().unwrap()));

        let common = root.join("new_common.tga");
        let split9 = root.join("new_split9.tga");
        let animation_first = root.join("animated_first.tga");
        let animation_second = root.join("animated_second.tga");
        fs::write(&common, sample_tga(10, 20, 30)).unwrap();
        fs::write(&split9, sample_tga(40, 50, 60)).unwrap();
        fs::write(&animation_first, sample_tga(70, 80, 90)).unwrap();
        fs::write(&animation_second, sample_tga(100, 110, 120)).unwrap();
        fs::write(
            split9.with_extension("txt"),
            "[Split9]\nSplit9X1=1\nSplit9X2=2\nSplit9X3=3\nSplit9Y1=4\nSplit9Y2=5\nSplit9Y3=6\n",
        )
        .unwrap();
        fs::write(
            animation_first.with_extension("txt"),
            "[Animations]\nAnimNext=animated_second\nMaxFrameRate=12.5\nMinFrameRate=2.0\nOneTimeAnimLoop=False\nPrimeCount=0\nTotalFrameNum=4\n",
        )
        .unwrap();
        let (mut cache, _) = open_cached(&output_path).unwrap();
        let summary = cached_import_textures_with_progress(
            &mut cache,
            "CandidateWnd",
            vec![
                common.to_string_lossy().into_owned(),
                split9.to_string_lossy().into_owned(),
                animation_first.to_string_lossy().into_owned(),
                animation_second.to_string_lossy().into_owned(),
            ],
            |_| {},
        )
        .unwrap();
        assert_eq!(summary.added, 4);
        assert_eq!(summary.skipped, 0);
        let imported = cached_list_entries(&cache).unwrap();
        assert_eq!(imported.len(), 4);
        assert!(imported
            .iter()
            .any(|entry| entry.name == "CandidateWnd.new_common" && !entry.has_split9));
        assert!(imported
            .iter()
            .any(|entry| entry.name == "CandidateWnd.new_split9" && entry.has_split9));
        assert!(!imported
            .iter()
            .any(|entry| entry.name.contains("Common") || entry.name.contains("TlpSpt9")));
        let animated = imported
            .iter()
            .find(|entry| entry.name == "CandidateWnd.animated_first")
            .unwrap();
        let animation = animated.animation.as_ref().unwrap();
        assert_eq!(
            animation.anim_next.as_deref(),
            Some("CandidateWnd.animated_second")
        );
        assert_eq!(animation.values.max_frame_rate, 12.5);
        assert_eq!(animation.values.min_frame_rate, 2.0);
        assert!(!animation.values.one_time_anim_loop);
        assert_eq!(animation.values.prime_count, 0);
        assert_eq!(animation.values.total_frame_num, 4);
        let animation_export = root.join("animated_export.tga");
        cached_export_entry(
            &cache,
            animated.export_index,
            &animation_export.to_string_lossy(),
        )
        .unwrap();
        let animation_metadata =
            fs::read_to_string(animation_export.with_extension("txt")).unwrap();
        assert!(animation_metadata.contains("[Animations]"));
        assert!(animation_metadata.contains("AnimNext=CandidateWnd.animated_second"));
        assert!(animation_metadata.contains("MaxFrameRate=12.500000"));
        let split9_entry = imported
            .iter()
            .find(|entry| entry.name == "CandidateWnd.new_split9")
            .unwrap();
        let split9_export = root.join("split9_export.tga");
        cached_export_entry(
            &cache,
            split9_entry.export_index,
            &split9_export.to_string_lossy(),
        )
        .unwrap();
        let split9_metadata = fs::read_to_string(split9_export.with_extension("txt")).unwrap();
        assert!(split9_metadata.contains("[Split9]\nSplit9X1=1"));
        assert!(create_new(&output_path).is_err());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_table_omits_zero_size_offsets_and_reads_legacy_tables() {
        let entries = vec![
            ExportEntry {
                class: -1,
                super_class: 0,
                package: 0,
                name_index: 0,
                flags: 0,
                size: 0,
                offset: 0,
            },
            ExportEntry {
                class: -2,
                super_class: 0,
                package: 1,
                name_index: 1,
                flags: 0,
                size: 4,
                offset: 128,
            },
        ];
        let standard = serialize_export_table(&entries);
        let parsed = read_export_table(&standard, 0, entries.len(), 2, false).unwrap();
        assert_eq!(parsed.0[1].class, -2);
        assert_eq!(parsed.0[1].name_index, 1);
        assert_eq!(parsed.1, standard.len());

        let mut legacy = Vec::new();
        for entry in &entries {
            write_compact(&mut legacy, entry.class);
            write_compact(&mut legacy, entry.super_class);
            write_i32(&mut legacy, entry.package);
            write_compact(&mut legacy, entry.name_index);
            write_i32(&mut legacy, entry.flags);
            write_compact(&mut legacy, entry.size);
            write_compact(&mut legacy, entry.offset);
        }
        let incorrectly_shifted = read_export_table(&legacy, 0, entries.len(), 2, false).unwrap();
        assert_ne!(incorrectly_shifted.0[1].class, -2);
        let legacy_parsed = read_export_table(&legacy, 0, entries.len(), 2, true).unwrap();
        assert_eq!(legacy_parsed.0[1].class, -2);
        assert_eq!(legacy_parsed.1, legacy.len());
    }

    #[test]
    fn texture_layout_detects_an_active_anim_next_reference() {
        let package = Package {
            data: Vec::new(),
            version: 0,
            licensee: 0,
            name_offset: 0,
            names: ["None", "Format", "AnimNext"]
                .into_iter()
                .map(|name| NameEntry {
                    name: name.into(),
                    flags: 0,
                })
                .collect(),
            imports: Vec::new(),
            exports: Vec::new(),
        };
        let mut raw = Vec::new();
        write_compact(&mut raw, 2);
        raw.extend_from_slice(&[0x05, 7]);
        write_compact(&mut raw, 1);
        raw.extend_from_slice(&[0x00, 5]);
        write_compact(&mut raw, 0);
        raw.push(1);
        write_i32(&mut raw, 0);

        assert_eq!(texture_layout(&raw, &package).unwrap().anim_next, Some(7));
    }

    #[test]
    fn split9_patch_keeps_omitted_default_properties() {
        let layout = TextureLayout {
            format: PropertyPatch { offset: 0, size: 1 },
            width: None,
            height: None,
            u_bits: None,
            v_bits: None,
            anim_next: None,
            split9_x1: None,
            split9_x2: None,
            split9_x3: None,
            split9_y1: None,
            split9_y2: None,
            split9_y3: None,
            mip_count_offset: 0,
            mip_width_offset: 0,
            mip_payload_offset: 0,
        };
        let mut output = Vec::new();
        let default_split9 = Split9 {
            x1: 0,
            x2: 0,
            x3: 0,
            y1: 0,
            y2: 0,
            y3: 0,
        };
        patch_split9_properties(&mut output, &layout, default_split9).unwrap();
        let error = patch_split9_properties(
            &mut output,
            &layout,
            Split9 {
                x1: 1,
                ..default_split9
            },
        )
        .unwrap_err();
        assert!(error.contains("Split9X1"));
    }

    #[test]
    fn new_texture_export_applies_split9_values() {
        let names = [
            "None",
            "Format",
            "USize",
            "VSize",
            "bSplit9Texture",
            "Split9X1",
            "Split9X2",
            "Split9X3",
            "Split9Y1",
            "Split9Y2",
            "Split9Y3",
        ];
        let package = Package {
            data: Vec::new(),
            version: 0,
            licensee: 0,
            name_offset: 0,
            names: names
                .into_iter()
                .map(|name| NameEntry {
                    name: name.into(),
                    flags: 0,
                })
                .collect(),
            imports: Vec::new(),
            exports: Vec::new(),
        };
        let mut template = Vec::new();
        write_compact(&mut template, 1);
        template.extend_from_slice(&[1, 5]);
        for name_index in [2, 3] {
            write_compact(&mut template, name_index);
            template.push(0x22);
            write_i32(&mut template, 1);
        }
        write_compact(&mut template, 4);
        template.extend_from_slice(&[0x83, 1]);
        for name_index in 5..=10 {
            write_compact(&mut template, name_index);
            template.push(0x22);
            write_i32(&mut template, 0);
        }
        write_compact(&mut template, 0);
        template.push(1);
        write_i32(&mut template, 0);
        write_compact(&mut template, 4);
        template.extend_from_slice(&[0, 0, 255, 255]);
        write_i32(&mut template, 1);
        write_i32(&mut template, 1);
        template.extend_from_slice(&[0, 0]);

        let serialized = build_texture_export(
            &template,
            &package,
            &ImportedTexture {
                name: "new_split9".into(),
                format: UtxFormat::Rgba8,
                width: 1,
                height: 1,
                pixels: vec![0, 0, 255, 255],
                metadata: TextureMetadata {
                    settings: TextureSettings::default(),
                    split9: Some(Split9 {
                        x1: 2,
                        x2: 3,
                        x3: 4,
                        y1: 5,
                        y2: 6,
                        y3: 7,
                    }),
                    animation: None,
                },
            },
        )
        .unwrap();
        let properties =
            read_texture_properties(&mut Reader::new(&serialized.bytes), &package).unwrap();
        assert!(properties.has_split9);
        assert_eq!(properties.split9.x1, 2);
        assert_eq!(properties.split9.x2, 3);
        assert_eq!(properties.split9.x3, 4);
        assert_eq!(properties.split9.y1, 5);
        assert_eq!(properties.split9.y2, 6);
        assert_eq!(properties.split9.y3, 7);
    }

    #[test]
    fn list_export_preview_replace_and_batch_import() {
        let root = env::temp_dir().join(format!(
            "unreal-tools-utx-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let package_path = root.join("test.utx");
        fs::write(&package_path, fixture_package()).unwrap();
        let path = package_path.to_string_lossy().into_owned();
        let entries = list_entries(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].format, UtxFormat::Rgba8);
        let exported = root.join("texture.tga");
        export_entry(&path, entries[0].export_index, &exported.to_string_lossy()).unwrap();
        assert_eq!(
            tga_dimensions(&fs::read(&exported).unwrap()).unwrap(),
            (1, 1)
        );
        assert!(preview_texture(&path, entries[0].export_index)
            .unwrap()
            .data_url
            .starts_with("data:image/png;base64,"));
        let replacement = root.join("texture_replacement.tga");
        fs::write(&replacement, sample_tga(0, 255, 0)).unwrap();
        replace_entry(
            &path,
            entries[0].export_index,
            &replacement.to_string_lossy(),
        )
        .unwrap();
        let imported = import_entries(
            &path,
            vec![ReplacementRequest {
                export_index: entries[0].export_index,
                replacement_path: replacement.to_string_lossy().into_owned(),
            }],
        )
        .unwrap();
        assert_eq!(imported.imported, 1);

        let matching = root.join("sample.tga");
        let added = root.join("fresh_button.tga");
        let added_dxt = root.join("fresh_dxt.dds");
        fs::write(&matching, sample_tga(0, 0, 255)).unwrap();
        fs::write(&added, sample_tga(255, 255, 0)).unwrap();
        fs::write(&added_dxt, sample_dxt1_dds()).unwrap();
        let (mut cache, _) = open_cached(&path).unwrap();
        let mut progress = Vec::new();
        let summary = cached_import_textures_with_progress(
            &mut cache,
            "Pacote principal",
            vec![
                matching.to_string_lossy().into_owned(),
                added.to_string_lossy().into_owned(),
                added_dxt.to_string_lossy().into_owned(),
            ],
            |update| progress.push(update),
        )
        .unwrap();
        assert_eq!(summary.replaced, 1);
        assert_eq!(summary.added, 2);
        assert_eq!(progress.last().map(|update| update.completed), Some(3));
        let entries = cached_list_entries(&cache).unwrap();
        assert_eq!(entries.len(), 3);
        let fresh = entries
            .iter()
            .find(|entry| entry.name == "fresh_button")
            .unwrap();
        assert!(entries
            .iter()
            .any(|entry| entry.name == "fresh_dxt" && entry.format == UtxFormat::Dxt1));
        let fresh_export = root.join("fresh_button_export.tga");
        cached_export_entry(&cache, fresh.export_index, &fresh_export.to_string_lossy()).unwrap();
        assert_eq!(
            tga_dimensions(&fs::read(fresh_export).unwrap()).unwrap(),
            (1, 1)
        );
        let location = cache.package.mip0_location(fresh.export_index).unwrap();
        let export = cache.package.export_at(fresh.export_index).unwrap();
        let raw = cache.package.export_data(export).unwrap();
        let layout = texture_layout(raw, &cache.package).unwrap();
        let width_offset = i32::from_le_bytes(
            raw[layout.mip_width_offset..layout.mip_width_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        assert_eq!(
            width_offset,
            export.offset as usize + location.pixel_offset + location.size
        );

        let grouped = root.join("candidate_button.tga");
        fs::write(&grouped, sample_tga(10, 20, 30)).unwrap();
        let summary = cached_import_textures_with_progress(
            &mut cache,
            "CandidateWnd",
            vec![grouped.to_string_lossy().into_owned()],
            |_| {},
        )
        .unwrap();
        assert_eq!(summary.added, 1);
        let entries = cached_list_entries(&cache).unwrap();
        let grouped_entry = entries
            .iter()
            .find(|entry| entry.name == "CandidateWnd.candidate_button")
            .unwrap();
        let group_reference = cache
            .package
            .export_at(grouped_entry.export_index)
            .unwrap()
            .package;
        let group_export = cache
            .package
            .export_at(object_index(group_reference).unwrap())
            .unwrap();
        assert_eq!(
            cache.package.full_class_name(group_export).unwrap(),
            "Core.Package"
        );
        assert_eq!(group_export.size, 1);
        assert_eq!(cache.package.export_data(group_export).unwrap(), &[1]);

        fs::write(&grouped, sample_tga(100, 110, 120)).unwrap();
        let summary = cached_import_textures_with_progress(
            &mut cache,
            "CandidateWnd",
            vec![grouped.to_string_lossy().into_owned()],
            |_| {},
        )
        .unwrap();
        assert_eq!(summary.replaced, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resilient_replacement_keeps_valid_texture_when_another_is_invalid() {
        let request = |pixels: Vec<u8>| texture_engine::TextureImportRequest {
            name: "sample".into(),
            format: UtxFormat::Rgba8.value().unwrap(),
            width: 1,
            height: 1,
            pixels,
            alpha: None,
            masked: None,
            clamp: None,
            split9: None,
            animation: None,
        };
        let imports = vec![
            RetriedTextureImport {
                file_path: "C:\\import\\valid.tga".into(),
                file_name: "valid.tga".into(),
                export_index: 0,
                request: request(vec![1, 2, 3, 255]),
            },
            RetriedTextureImport {
                file_path: "C:\\import\\invalid.tga".into(),
                file_name: "invalid.tga".into(),
                export_index: 0,
                request: request(vec![1, 2, 3]),
            },
        ];
        let mut log = ImportDebugLog {
            path: None,
            file: None,
        };

        let recovered = import_existing_textures_resilient(
            fixture_package(),
            "Pacote principal",
            &imports,
            &mut log,
        );

        assert_eq!(recovered.applied.len(), 1);
        assert_eq!(recovered.applied[0].file_name, "valid.tga");
        assert_eq!(recovered.failures.len(), 1);
        assert_eq!(recovered.failures[0].0, "invalid.tga");
    }

    #[test]
    fn directory_import_groups_root_and_child_folder_textures() {
        let root = env::temp_dir().join(format!(
            "unreal-tools-utx-directory-import-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let buttons = root.join("Buttons");
        let nested = buttons.join("Nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("root_texture.tga"), sample_tga(1, 2, 3)).unwrap();
        fs::write(buttons.join("button.tga"), sample_tga(4, 5, 6)).unwrap();
        fs::write(nested.join("nested.dds"), sample_dxt1_dds()).unwrap();
        fs::write(root.join("ignored.txt"), "metadata").unwrap();

        let groups = collect_texture_import_groups(&root).unwrap();

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "Pacote principal");
        assert_eq!(groups[0].1.len(), 1);
        assert_eq!(groups[1].0, "Buttons");
        assert_eq!(groups[1].1.len(), 2);
        assert!(groups[1].1.iter().any(|file| file.ends_with("button.tga")));
        assert!(groups[1].1.iter().any(|file| file.ends_with("nested.dds")));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn standalone_extractor_writes_original_and_png_textures() {
        let root = env::temp_dir().join(format!(
            "unreal-tools-utx-extract-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("Sample.utx");
        fs::write(&source, fixture_package()).unwrap();

        let original_output = root.join("original");
        let original = extract_packages_with_progress(
            vec![source.to_string_lossy().into_owned()],
            &original_output.to_string_lossy(),
            UtxExtractMode::Original,
            |_| {},
        )
        .unwrap();
        assert_eq!(original.packages, 1);
        assert_eq!(original.exported, 1);
        assert!(original_output.join("Sample").join("sample.tga").is_file());

        let png_output = root.join("png");
        let png = extract_packages_with_progress(
            vec![source.to_string_lossy().into_owned()],
            &png_output.to_string_lossy(),
            UtxExtractMode::Png,
            |_| {},
        )
        .unwrap();
        assert_eq!(png.exported, 1);
        let png_data = fs::read(png_output.join("Sample").join("sample.png")).unwrap();
        assert_eq!(&png_data[..8], b"\x89PNG\r\n\x1a\n");

        fs::remove_dir_all(root).unwrap();
    }

    fn sample_tga(red: u8, green: u8, blue: u8) -> Vec<u8> {
        let mut tga = vec![0; 18];
        tga[2] = 2;
        tga[12] = 1;
        tga[14] = 1;
        tga[16] = 32;
        tga[17] = 0x28;
        tga.extend_from_slice(&[blue, green, red, 255]);
        tga
    }

    fn sample_dxt1_dds() -> Vec<u8> {
        let mut dds = vec![0; 128];
        dds[..4].copy_from_slice(b"DDS ");
        write_i32_at(&mut dds, 4, 124).unwrap();
        write_i32_at(&mut dds, 12, 4).unwrap();
        write_i32_at(&mut dds, 16, 4).unwrap();
        dds[84..88].copy_from_slice(b"DXT1");
        dds.extend_from_slice(&[0, 0xf8, 0x1f, 0, 0, 0, 0, 0]);
        dds
    }

    fn write_compact(output: &mut Vec<u8>, value: i32) {
        let negative = value < 0;
        let mut magnitude = if negative {
            value.wrapping_neg() as u32
        } else {
            value as u32
        };
        let mut first = (magnitude & 0x3f) as u8;
        magnitude >>= 6;
        if negative {
            first |= 0x80;
        }
        if magnitude > 0 {
            first |= 0x40;
        }
        output.push(first);
        for index in 1..=3 {
            if magnitude == 0 {
                break;
            }
            let mut byte = (magnitude & 0x7f) as u8;
            magnitude >>= 7;
            if magnitude > 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if index == 3 && magnitude > 0 {
                output.push((magnitude & 0x1f) as u8);
                break;
            }
        }
    }
    fn write_i32(buffer: &mut Vec<u8>, value: i32) {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    fn write_name_table(names: &[&str]) -> Vec<u8> {
        let mut output = Vec::new();
        for name in names {
            write_compact(&mut output, name.len() as i32 + 1);
            output.extend_from_slice(name.as_bytes());
            output.push(0);
            write_i32(&mut output, 0);
        }
        output
    }
    fn fixture_package() -> Vec<u8> {
        let names = [
            "None", "Engine", "Texture", "sample", "Format", "USize", "VSize",
        ];
        let name_table = write_name_table(&names);
        let mut import_table = Vec::new();
        write_compact(&mut import_table, 0);
        write_compact(&mut import_table, 0);
        write_i32(&mut import_table, 0);
        write_compact(&mut import_table, 1);
        write_compact(&mut import_table, 0);
        write_compact(&mut import_table, 0);
        write_i32(&mut import_table, -1);
        write_compact(&mut import_table, 2);
        let mut texture = Vec::new();
        write_compact(&mut texture, 4);
        texture.push(1);
        texture.push(5);
        write_compact(&mut texture, 5);
        texture.push(0x22);
        write_i32(&mut texture, 1);
        write_compact(&mut texture, 6);
        texture.push(0x22);
        write_i32(&mut texture, 1);
        write_compact(&mut texture, 0);
        texture.push(1);
        texture.extend_from_slice(&0i32.to_le_bytes());
        write_compact(&mut texture, 4);
        texture.extend_from_slice(&[0, 0, 255, 255]);
        write_i32(&mut texture, 1);
        write_i32(&mut texture, 1);
        texture.extend_from_slice(&[0, 0]);
        let name_offset = 36usize;
        let import_offset = name_offset + name_table.len();
        let export_offset = import_offset + import_table.len();
        let mut export_table = Vec::new();
        write_compact(&mut export_table, -2);
        write_compact(&mut export_table, 0);
        write_i32(&mut export_table, 0);
        write_compact(&mut export_table, 3);
        write_i32(&mut export_table, 0);
        write_compact(&mut export_table, texture.len() as i32);
        write_compact(&mut export_table, (export_offset + 10) as i32);
        let data_offset = export_offset + export_table.len();
        export_table.clear();
        write_compact(&mut export_table, -2);
        write_compact(&mut export_table, 0);
        write_i32(&mut export_table, 0);
        write_compact(&mut export_table, 3);
        write_i32(&mut export_table, 0);
        write_compact(&mut export_table, texture.len() as i32);
        write_compact(&mut export_table, data_offset as i32);
        let mut output = vec![0; 36];
        write_i32_at(&mut output, 0, PACKAGE_MAGIC).unwrap();
        write_i32_at(&mut output, 12, names.len() as i32).unwrap();
        write_i32_at(&mut output, 16, name_offset as i32).unwrap();
        write_i32_at(&mut output, 20, 1).unwrap();
        write_i32_at(&mut output, 24, export_offset as i32).unwrap();
        write_i32_at(&mut output, 28, 2).unwrap();
        write_i32_at(&mut output, 32, import_offset as i32).unwrap();
        output.extend_from_slice(&name_table);
        output.extend_from_slice(&import_table);
        output.extend_from_slice(&export_table);
        output.extend_from_slice(&texture);
        output
    }
}
