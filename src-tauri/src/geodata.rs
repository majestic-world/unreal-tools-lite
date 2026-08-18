//! Safe Lineage 2 geodata converter.
//!
//! The binary layouts and encryption are compatible with the legacy Toolkit
//! implementation. Parsing happens in a bounded reader so a malformed region
//! fails only its own batch item instead of compromising the process.

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use rayon::{prelude::*, ThreadPoolBuilder};
use serde::{Deserialize, Serialize};

type GeoResult<T> = Result<T, String>;

const REGION_SIZE: usize = 256;
const BLOCK_CELLS: usize = 8;
const CELLS_PER_BLOCK: usize = BLOCK_CELLS * BLOCK_CELLS;
const MIN_REGION: i32 = 10;
const MAX_REGION: i32 = 26;
const NSWE_ALL: u8 = 15;
const GEO_CHECKSUM: u32 = (-2_126_429_781_i32) as u32;
const MAX_REPORTED_ERRORS: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GeodataFormat {
    L2j,
    ConvDat,
    L2d,
    L2s,
    L2g,
    Rp,
    PathTxt,
    L2m,
}

impl GeodataFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::L2j => ".l2j",
            Self::ConvDat => "_conv.dat",
            Self::L2d => ".l2d",
            Self::L2s => ".l2s",
            Self::L2g => ".l2g",
            Self::Rp => ".rp",
            Self::PathTxt => "_path.txt",
            Self::L2m => ".l2m",
        }
    }

    fn is_output_supported(self) -> bool {
        matches!(self, Self::L2j | Self::ConvDat | Self::L2g)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeodataProgress {
    pub completed: usize,
    pub total: usize,
    pub file_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeodataSummary {
    pub output_directory: String,
    pub total_files: usize,
    pub converted_files: usize,
    pub copied_files: usize,
    pub skipped_files: usize,
    pub failed_files: usize,
    pub workers: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct Cell {
    height: i16,
    min_height: Option<i16>,
    nswe: u8,
}

impl Cell {
    fn height_mask(self, is_flat: bool) -> i16 {
        if is_flat {
            self.height
        } else {
            encode_height_and_nswe(self.height, self.nswe)
        }
    }

    fn min_height(self) -> i16 {
        self.min_height.unwrap_or(self.height)
    }
}

#[derive(Debug)]
enum Block {
    Flat(Cell),
    Complex(Vec<Cell>),
    MultiLevel(Vec<Vec<Cell>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Flat,
    Complex,
    MultiLevel,
}

#[derive(Debug)]
struct Region {
    x: i32,
    y: i32,
    blocks: Vec<Block>,
}

impl Region {
    fn new(x: i32, y: i32, blocks: Vec<Block>) -> GeoResult<Self> {
        if blocks.len() != REGION_SIZE * REGION_SIZE {
            return Err("A região geodata não possui todos os blocos esperados.".into());
        }
        Ok(Self { x, y, blocks })
    }

    fn block(&self, x: usize, y: usize) -> &Block {
        &self.blocks[x * REGION_SIZE + y]
    }

    fn counts(&self) -> (i32, i32, i32) {
        let mut flat = 0_i32;
        let mut flat_and_complex = 0_i32;
        let mut cells = 0_i32;
        for block in &self.blocks {
            match block {
                Block::Flat(_) => {
                    flat += 1;
                    flat_and_complex += 1;
                }
                Block::Complex(entries) => {
                    flat_and_complex += 1;
                    cells += entries.len() as i32;
                }
                Block::MultiLevel(entries) => {
                    cells += entries
                        .iter()
                        .map(|layers| layers.len() as i32)
                        .sum::<i32>();
                }
            }
        }
        (cells, flat_and_complex, flat)
    }
}

#[derive(Default)]
struct Totals {
    converted_files: usize,
    copied_files: usize,
    skipped_files: usize,
    failed_files: usize,
    errors: Vec<String>,
}

#[derive(Debug)]
struct WorkItem {
    source: PathBuf,
    destination: PathBuf,
    source_format: GeodataFormat,
    copy: bool,
    file_name: String,
}

/// Converts every supported geodata file under `input_directory`. The worker
/// pool deliberately leaves one logical core for the UI/system and never uses
/// more than four workers because a parsed region is memory intensive.
pub fn convert_directory_with_progress<F>(
    input_directory: &str,
    output_directory: &str,
    target_format: GeodataFormat,
    progress: F,
) -> GeoResult<GeodataSummary>
where
    F: Fn(GeodataProgress) + Send + Sync,
{
    if !target_format.is_output_supported() {
        return Err("O formato de saída deve ser L2J, CONV_DAT ou L2G.".into());
    }

    let input_root = Path::new(input_directory);
    if !input_root.is_dir() {
        return Err("Selecione uma pasta de entrada válida para a geodata.".into());
    }
    let input_root = input_root
        .canonicalize()
        .map_err(|error| format!("Não foi possível acessar a pasta de entrada: {error}"))?;
    let output_root = Path::new(output_directory);
    if output_root.as_os_str().is_empty() {
        return Err("Selecione uma pasta de saída válida.".into());
    }
    fs::create_dir_all(output_root)
        .map_err(|error| format!("Não foi possível criar a pasta de saída: {error}"))?;
    let output_root = output_root
        .canonicalize()
        .map_err(|error| format!("Não foi possível acessar a pasta de saída: {error}"))?;
    if output_root.starts_with(&input_root) {
        return Err("A pasta de saída não pode ficar dentro da pasta de entrada.".into());
    }

    let sources = collect_geodata_files(&input_root)?;
    if sources.is_empty() {
        return Err("Nenhum arquivo geodata suportado foi encontrado na pasta de entrada.".into());
    }

    let total = sources.len();
    let totals = Arc::new(Mutex::new(Totals::default()));
    let completed = Arc::new(AtomicUsize::new(0));
    let reserved_destinations = Arc::new(Mutex::new(HashSet::<String>::new()));
    let mut work = Vec::new();

    for source in sources {
        let file_name = display_file_name(&source);
        let source_format = match detect_format(&source) {
            Some(format) => format,
            None => {
                record_skip(&totals, format!("{file_name}: formato não reconhecido."));
                report_completed(&completed, total, &file_name, &progress);
                continue;
            }
        };
        let relative = match source.strip_prefix(&input_root) {
            Ok(path) => path,
            Err(error) => {
                record_failure(
                    &totals,
                    format!("{file_name}: caminho de entrada inválido: {error}"),
                );
                report_completed(&completed, total, &file_name, &progress);
                continue;
            }
        };
        let (x, y) = match coordinates_for_file(&source, source_format) {
            Ok(value) => value,
            Err(error) => {
                record_failure(&totals, format!("{file_name}: {error}"));
                report_completed(&completed, total, &file_name, &progress);
                continue;
            }
        };
        if !valid_coordinates(x, y) {
            record_failure(
                &totals,
                format!("{file_name}: coordenadas {x}_{y} fora do intervalo 10..26."),
            );
            report_completed(&completed, total, &file_name, &progress);
            continue;
        }
        let destination = if source_format == target_format {
            output_root.join(relative)
        } else {
            let directory = relative.parent().unwrap_or_else(|| Path::new(""));
            output_root
                .join(directory)
                .join(format!("{x}_{y}{}", target_format.extension()))
        };
        let destination_key = destination.to_string_lossy().to_ascii_lowercase();
        let reserved = reserved_destinations
            .lock()
            .map_err(|_| "Não foi possível reservar os arquivos de saída.")?
            .insert(destination_key);
        if !reserved {
            record_skip(
                &totals,
                format!("{file_name}: outra entrada deste lote já usa o mesmo destino; mantido o primeiro resultado."),
            );
            report_completed(&completed, total, &file_name, &progress);
            continue;
        }
        work.push(WorkItem {
            source,
            destination,
            source_format,
            copy: source_format == target_format,
            file_name,
        });
    }

    let workers = worker_count();
    let pool = ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .map_err(|error| format!("Não foi possível iniciar a conversão paralela: {error}"))?;
    pool.install(|| {
        work.par_iter().for_each(|item| {
            let result = if item.copy {
                copy_file_replacing(&item.source, &item.destination)
            } else {
                let region = parse_file(&item.source, item.source_format);
                region.and_then(|region| {
                    write_region_replacing(&region, target_format, &item.destination)
                })
            };
            match result {
                Ok(()) if item.copy => record_copy(&totals),
                Ok(()) => record_conversion(&totals),
                Err(error) => record_failure(&totals, format!("{}: {error}", item.file_name)),
            }
            report_completed(&completed, total, &item.file_name, &progress);
        });
    });

    let totals = totals
        .lock()
        .map_err(|_| "Não foi possível consolidar a conversão geodata.")?;
    Ok(GeodataSummary {
        output_directory: display_path(&output_root),
        total_files: total,
        converted_files: totals.converted_files,
        copied_files: totals.copied_files,
        skipped_files: totals.skipped_files,
        failed_files: totals.failed_files,
        workers,
        errors: totals.errors.clone(),
    })
}

fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get().saturating_sub(1).clamp(1, 4))
        .unwrap_or(1)
}

fn collect_geodata_files(root: &Path) -> GeoResult<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_geodata_files_recursive(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_geodata_files_recursive(directory: &Path, files: &mut Vec<PathBuf>) -> GeoResult<()> {
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("Não foi possível ler {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("Não foi possível ler uma entrada: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Não foi possível identificar {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_geodata_files_recursive(&path, files)?;
        } else if file_type.is_file() && detect_format(&path).is_some() {
            files.push(path);
        }
    }
    Ok(())
}

fn detect_format(path: &Path) -> Option<GeodataFormat> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with("_conv.dat") {
        return Some(GeodataFormat::ConvDat);
    }
    if name.ends_with("_path.txt") {
        return Some(GeodataFormat::PathTxt);
    }
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "l2j" => Some(GeodataFormat::L2j),
        "l2d" => Some(GeodataFormat::L2d),
        "l2s" => Some(GeodataFormat::L2s),
        "l2g" => Some(GeodataFormat::L2g),
        "l2m" => Some(GeodataFormat::L2m),
        "rp" => Some(GeodataFormat::Rp),
        _ => None,
    }
}

fn coordinates_for_file(path: &Path, format: GeodataFormat) -> GeoResult<(i32, i32)> {
    if format == GeodataFormat::ConvDat {
        let data = fs::read(path)
            .map_err(|error| format!("Não foi possível ler o cabeçalho CONV_DAT: {error}"))?;
        let x = *data.first().ok_or("Cabeçalho CONV_DAT truncado.")? as i32;
        let y = *data.get(1).ok_or("Cabeçalho CONV_DAT truncado.")? as i32;
        return Ok((x, y));
    }
    coordinates_from_name(path)
}

fn coordinates_from_name(path: &Path) -> GeoResult<(i32, i32)> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or("O nome do arquivo não é válido.")?;
    let mut parts = stem.split('_');
    let x = parts
        .next()
        .ok_or("O nome deve começar com X_Y.")?
        .parse::<i32>()
        .map_err(|_| "O nome deve começar com X_Y.")?;
    let y = parts
        .next()
        .ok_or("O nome deve começar com X_Y.")?
        .parse::<i32>()
        .map_err(|_| "O nome deve começar com X_Y.")?;
    Ok((x, y))
}

fn valid_coordinates(x: i32, y: i32) -> bool {
    (MIN_REGION..=MAX_REGION).contains(&x) && (MIN_REGION..=MAX_REGION).contains(&y)
}

fn parse_file(path: &Path, format: GeodataFormat) -> GeoResult<Region> {
    if format == GeodataFormat::PathTxt {
        return parse_path_text(path);
    }
    let mut data = fs::read(path)
        .map_err(|error| format!("Não foi possível ler {}: {error}", path.display()))?;
    decrypt(&mut data, format)?;
    let (x, y) = if format == GeodataFormat::ConvDat {
        let mut reader = Reader::new(&data);
        let x = reader.read_u8()? as i32;
        let y = reader.read_u8()? as i32;
        reader.skip(16)?;
        (x, y)
    } else {
        coordinates_from_name(path)?
    };
    if !valid_coordinates(x, y) {
        return Err(format!("Coordenadas {x}_{y} fora do intervalo 10..26."));
    }
    let mut reader = Reader::new(&data);
    if format == GeodataFormat::ConvDat {
        reader.skip(18)?;
    } else if matches!(format, GeodataFormat::L2g | GeodataFormat::L2s) {
        reader.skip(4)?;
    }
    parse_binary_region(&mut reader, x, y, format)
}

fn decrypt(data: &mut [u8], format: GeodataFormat) -> GeoResult<()> {
    match format {
        GeodataFormat::L2g => {
            let random_key = read_u32_be(data, 0, "Cabeçalho L2G truncado.")?;
            decrypt_chained(data, GEO_CHECKSUM ^ random_key);
        }
        GeodataFormat::L2s => {
            let random_key = read_u32_be(data, 0, "Cabeçalho L2S truncado.")?;
            let mut checksum = GEO_CHECKSUM;
            for byte in b"127.0.0.1" {
                checksum ^= u32::from(*byte);
                checksum = checksum.rotate_right(1);
            }
            decrypt_chained(data, checksum ^ random_key);
        }
        _ => {}
    }
    Ok(())
}

fn decrypt_chained(data: &mut [u8], checksum: u32) {
    let mut xor_byte = checksum_byte(checksum);
    for byte in data.iter_mut().skip(4) {
        let decrypted = *byte ^ xor_byte;
        *byte = decrypted;
        xor_byte = decrypted;
    }
}

fn parse_binary_region(
    reader: &mut Reader<'_>,
    x: i32,
    y: i32,
    format: GeodataFormat,
) -> GeoResult<Region> {
    let mut blocks = Vec::with_capacity(REGION_SIZE * REGION_SIZE);
    for block_x in 0..REGION_SIZE {
        for block_y in 0..REGION_SIZE {
            let kind = read_block_kind(reader, format)?;
            let block = match kind {
                BlockKind::Flat => parse_flat(reader, format)?,
                BlockKind::Complex => parse_complex(reader, format)?,
                BlockKind::MultiLevel => parse_multi(reader, format)?,
            };
            validate_block(&block, block_x, block_y)?;
            blocks.push(block);
        }
    }
    Region::new(x, y, blocks)
}

fn read_block_kind(reader: &mut Reader<'_>, format: GeodataFormat) -> GeoResult<BlockKind> {
    let value = match format {
        GeodataFormat::ConvDat => reader.read_i16()? as i32,
        _ => reader.read_u8()? as i32,
    };
    match format {
        GeodataFormat::ConvDat => match value {
            0 => Ok(BlockKind::Flat),
            64 => Ok(BlockKind::Complex),
            _ if value >= 64 => Ok(BlockKind::MultiLevel),
            _ => Err(format!("Tipo de bloco CONV_DAT inválido: {value}.")),
        },
        GeodataFormat::L2d | GeodataFormat::Rp => match value as u8 {
            0xd0 => Ok(BlockKind::Flat),
            0xd1 => Ok(BlockKind::Complex),
            0xd2 => Ok(BlockKind::MultiLevel),
            _ => Err(format!("Tipo de bloco L2D/RP inválido: 0x{value:02x}.")),
        },
        _ => match value {
            0 => Ok(BlockKind::Flat),
            1 => Ok(BlockKind::Complex),
            2 => Ok(BlockKind::MultiLevel),
            _ => Err(format!("Tipo de bloco inválido: {value}.")),
        },
    }
}

fn parse_flat(reader: &mut Reader<'_>, format: GeodataFormat) -> GeoResult<Block> {
    let height = reader.read_i16()?;
    let min_height = if format == GeodataFormat::ConvDat {
        Some(reader.read_i16()?)
    } else {
        None
    };
    Ok(Block::Flat(Cell {
        height,
        min_height,
        nswe: NSWE_ALL,
    }))
}

fn parse_complex(reader: &mut Reader<'_>, format: GeodataFormat) -> GeoResult<Block> {
    let mut cells = Vec::with_capacity(CELLS_PER_BLOCK);
    for _ in 0..CELLS_PER_BLOCK {
        let cell = match format {
            GeodataFormat::L2d | GeodataFormat::Rp => Cell {
                nswe: reader.read_u8()? & 0x0f,
                height: reader.read_i16()?,
                min_height: None,
            },
            _ => {
                let raw = reader.read_i16()?;
                Cell {
                    height: decode_height(raw),
                    min_height: None,
                    nswe: decode_nswe(raw),
                }
            }
        };
        cells.push(cell);
    }
    Ok(Block::Complex(cells))
}

fn parse_multi(reader: &mut Reader<'_>, format: GeodataFormat) -> GeoResult<Block> {
    if format == GeodataFormat::L2m {
        return parse_l2m_multi(reader);
    }
    let mut cells = Vec::with_capacity(CELLS_PER_BLOCK);
    for _ in 0..CELLS_PER_BLOCK {
        let count = match format {
            GeodataFormat::ConvDat => reader.read_i16()? as i32,
            _ => reader.read_u8()? as i32,
        };
        let count = validate_layer_count(count)?;
        let mut layers = Vec::with_capacity(count);
        for _ in 0..count {
            layers.push(match format {
                GeodataFormat::L2d | GeodataFormat::Rp => Cell {
                    nswe: reader.read_u8()? & 0x0f,
                    height: reader.read_i16()?,
                    min_height: None,
                },
                _ => {
                    let raw = reader.read_i16()?;
                    Cell {
                        height: decode_height(raw),
                        min_height: None,
                        nswe: decode_nswe(raw),
                    }
                }
            });
        }
        cells.push(layers);
    }
    Ok(Block::MultiLevel(cells))
}

fn parse_l2m_multi(reader: &mut Reader<'_>) -> GeoResult<Block> {
    let headers_start = reader.position();
    let mut offsets = Vec::with_capacity(CELLS_PER_BLOCK);
    let mut layer_counts = Vec::with_capacity(CELLS_PER_BLOCK);
    for index in 0..CELLS_PER_BLOCK {
        let header_position_after = reader
            .position()
            .checked_add(2)
            .ok_or("Offset L2M inválido.")?;
        let header = reader.read_i16()? as u16;
        let count = validate_layer_count((header & 0x1f) as i32)?;
        let offset_words = (header >> 5) as usize;
        let data_position = header_position_after
            .checked_add(offset_words.checked_mul(2).ok_or("Offset L2M inválido.")?)
            .ok_or("Offset L2M inválido.")?;
        offsets.push(data_position);
        layer_counts.push(count);
        if index == CELLS_PER_BLOCK - 1 && reader.position() < headers_start {
            return Err("Cabeçalho L2M inválido.".into());
        }
    }
    let mut cells = Vec::with_capacity(CELLS_PER_BLOCK);
    let mut end = reader.position();
    for (offset, count) in offsets.into_iter().zip(layer_counts) {
        let layer_bytes = count.checked_mul(2).ok_or("Bloco L2M muito grande.")?;
        let block = reader.bytes(offset, layer_bytes)?;
        let mut layers = Vec::with_capacity(count);
        for chunk in block.chunks_exact(2) {
            let raw = i16::from_le_bytes([chunk[0], chunk[1]]);
            layers.push(Cell {
                height: decode_height(raw),
                min_height: None,
                nswe: decode_nswe(raw),
            });
        }
        end = end.max(offset + layer_bytes);
        cells.push(layers);
    }
    reader.seek(end)?;
    Ok(Block::MultiLevel(cells))
}

fn validate_layer_count(count: i32) -> GeoResult<usize> {
    if !(1..=255).contains(&count) {
        return Err(format!("Quantidade de camadas inválida: {count}."));
    }
    Ok(count as usize)
}

fn validate_block(block: &Block, x: usize, y: usize) -> GeoResult<()> {
    match block {
        Block::Flat(_) => Ok(()),
        Block::Complex(cells) if cells.len() == CELLS_PER_BLOCK => Ok(()),
        Block::MultiLevel(cells)
            if cells.len() == CELLS_PER_BLOCK && cells.iter().all(|layers| !layers.is_empty()) =>
        {
            Ok(())
        }
        _ => Err(format!("Bloco {x}_{y} possui dados incompletos.")),
    }
}

fn parse_path_text(path: &Path) -> GeoResult<Region> {
    let (x, y) = coordinates_from_name(path)?;
    if !valid_coordinates(x, y) {
        return Err(format!("Coordenadas {x}_{y} fora do intervalo 10..26."));
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("Não foi possível ler PATH_TXT: {error}"))?;
    let mut grid = vec![None; REGION_SIZE * BLOCK_CELLS * REGION_SIZE * BLOCK_CELLS];
    for (line_number, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || !line.starts_with('[') {
            continue;
        }
        let (cell_x, cell_y, layers) = parse_path_header(line)
            .map_err(|error| format!("Linha {}: {error}", line_number + 1))?;
        if cell_x >= 2048 || cell_y >= 2048 {
            return Err(format!(
                "Linha {}: coordenada de célula fora do intervalo.",
                line_number + 1
            ));
        }
        let values = parse_path_layers(line, layers)
            .map_err(|error| format!("Linha {}: {error}", line_number + 1))?;
        grid[cell_x * 2048 + cell_y] = Some(values);
    }

    let mut blocks = Vec::with_capacity(REGION_SIZE * REGION_SIZE);
    for block_x in 0..REGION_SIZE {
        for block_y in 0..REGION_SIZE {
            let mut cells = Vec::with_capacity(CELLS_PER_BLOCK);
            for cell_x in 0..BLOCK_CELLS {
                for cell_y in 0..BLOCK_CELLS {
                    let x = block_x * BLOCK_CELLS + cell_x;
                    let y = block_y * BLOCK_CELLS + cell_y;
                    let layers = grid[x * 2048 + y].clone().unwrap_or_else(|| {
                        vec![Cell {
                            height: 16_383,
                            min_height: None,
                            nswe: 0,
                        }]
                    });
                    cells.push(layers);
                }
            }
            blocks.push(normalize_path_block(cells)?);
        }
    }
    Region::new(x, y, blocks)
}

fn parse_path_header(line: &str) -> GeoResult<(usize, usize, usize)> {
    let closing = line.find(']').ok_or("Cabeçalho de célula inválido.")?;
    let coordinates = line
        .get(1..closing)
        .ok_or("Cabeçalho de célula inválido.")?;
    let (x, y) = coordinates
        .split_once(',')
        .ok_or("Cabeçalho de célula inválido.")?;
    let layers = line
        .get(closing + 1..)
        .ok_or("Quantidade de camadas ausente.")?
        .split('(')
        .next()
        .unwrap_or_default()
        .trim()
        .parse::<usize>()
        .map_err(|_| "Quantidade de camadas inválida.")?;
    let x = x
        .trim()
        .parse::<usize>()
        .map_err(|_| "Coordenada X inválida.")?;
    let y = y
        .trim()
        .parse::<usize>()
        .map_err(|_| "Coordenada Y inválida.")?;
    Ok((x, y, layers))
}

fn parse_path_layers(line: &str, layer_count: usize) -> GeoResult<Vec<Cell>> {
    if layer_count == 0 {
        return Ok(vec![Cell {
            height: 16_383,
            min_height: None,
            nswe: 0,
        }]);
    }
    if layer_count > 255 {
        return Err("Quantidade de camadas inválida.".into());
    }
    let mut values = Vec::with_capacity(layer_count);
    let mut remainder = line;
    while let Some(start) = remainder.find('(') {
        let segment = &remainder[start + 1..];
        let end = segment.find(')').ok_or("Camada PATH_TXT não terminou.")?;
        let value = &segment[..end];
        let (height, flags) = value.split_once(':').unwrap_or((value, ""));
        let height = height
            .trim()
            .parse::<i16>()
            .map_err(|_| "Altura PATH_TXT inválida.")?;
        let mut nswe = 0;
        let flags = flags.as_bytes();
        for (index, flag) in flags.iter().take(4).enumerate() {
            if *flag == b'1' {
                nswe |= 1 << index;
            }
        }
        values.push(Cell {
            height,
            min_height: None,
            nswe,
        });
        remainder = &segment[end + 1..];
    }
    if values.len() != layer_count {
        return Err(format!(
            "Esperadas {layer_count} camadas, recebidas {}.",
            values.len()
        ));
    }
    Ok(values)
}

fn normalize_path_block(cells: Vec<Vec<Cell>>) -> GeoResult<Block> {
    if cells.len() != CELLS_PER_BLOCK || cells.iter().any(|layers| layers.is_empty()) {
        return Err("Bloco PATH_TXT incompleto.".into());
    }
    if cells.iter().any(|layers| layers.len() > 1) {
        return Ok(Block::MultiLevel(cells));
    }
    let first = cells[0][0];
    if cells
        .iter()
        .all(|layers| layers[0].nswe == NSWE_ALL && layers[0].height == first.height)
    {
        return Ok(Block::Flat(Cell {
            min_height: Some(first.height),
            ..first
        }));
    }
    Ok(Block::Complex(
        cells.into_iter().map(|layers| layers[0]).collect(),
    ))
}

fn write_region_replacing(
    region: &Region,
    format: GeodataFormat,
    destination: &Path,
) -> GeoResult<()> {
    let bytes = write_region(region, format)?;
    write_output_file(destination, &bytes)
}

fn write_region(region: &Region, format: GeodataFormat) -> GeoResult<Vec<u8>> {
    let mut data = Vec::new();
    if format == GeodataFormat::ConvDat {
        let (cells, flat_and_complex, flat) = region.counts();
        data.push(region.x as u8);
        data.push(region.y as u8);
        write_i16(&mut data, 128);
        write_i16(&mut data, 16);
        write_i32(&mut data, cells);
        write_i32(&mut data, flat_and_complex);
        write_i32(&mut data, flat);
    }
    for x in 0..REGION_SIZE {
        for y in 0..REGION_SIZE {
            write_block(&mut data, region.block(x, y), format)?;
        }
    }
    if format == GeodataFormat::L2g {
        return Ok(encrypt_l2g(data, region));
    }
    Ok(data)
}

fn write_block(output: &mut Vec<u8>, block: &Block, format: GeodataFormat) -> GeoResult<()> {
    match (format, block) {
        (GeodataFormat::L2j | GeodataFormat::L2g, Block::Flat(cell)) => {
            output.push(0);
            write_i16(output, cell.height_mask(true));
        }
        (GeodataFormat::L2j | GeodataFormat::L2g, Block::Complex(cells)) => {
            output.push(1);
            for cell in cells {
                write_i16(output, cell.height_mask(false));
            }
        }
        (GeodataFormat::L2j | GeodataFormat::L2g, Block::MultiLevel(cells)) => {
            output.push(2);
            for layers in cells {
                output
                    .push(u8::try_from(layers.len()).map_err(|_| "Camadas demais para L2J/L2G.")?);
                for cell in layers {
                    write_i16(output, cell.height_mask(false));
                }
            }
        }
        (GeodataFormat::ConvDat, Block::Flat(cell)) => {
            write_i16(output, 0);
            write_i16(output, cell.height_mask(true));
            write_i16(output, encode_height_and_nswe(cell.min_height(), cell.nswe));
        }
        (GeodataFormat::ConvDat, Block::Complex(cells)) => {
            write_i16(output, 64);
            for cell in cells {
                write_i16(output, cell.height_mask(false));
            }
        }
        (GeodataFormat::ConvDat, Block::MultiLevel(cells)) => {
            let body_size = cells
                .iter()
                .try_fold(0_usize, |size, layers| {
                    size.checked_add(2)
                        .and_then(|value| value.checked_add(layers.len().checked_mul(2)?))
                })
                .ok_or("Bloco CONV_DAT muito grande.")?;
            let type_value = 64_i32
                .checked_add(
                    i32::try_from(body_size).map_err(|_| "Bloco CONV_DAT muito grande.")? - 128,
                )
                .ok_or("Tipo de bloco CONV_DAT inválido.")?;
            write_i16(
                output,
                i16::try_from(type_value).map_err(|_| "Tipo de bloco CONV_DAT inválido.")?,
            );
            for layers in cells {
                write_i16(
                    output,
                    i16::try_from(layers.len()).map_err(|_| "Camadas demais para CONV_DAT.")?,
                );
                for cell in layers {
                    write_i16(output, cell.height_mask(false));
                }
            }
        }
        _ => return Err("Formato de saída geodata não suportado.".into()),
    }
    Ok(())
}

fn encrypt_l2g(data: Vec<u8>, region: &Region) -> Vec<u8> {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u32)
        .unwrap_or_default()
        ^ std::process::id()
        ^ ((region.x as u32) << 16)
        ^ region.y as u32;
    let random_key = seed.rotate_left(13) ^ 0xa5c3_17d9;
    let checksum = GEO_CHECKSUM ^ random_key;
    let mut output = Vec::with_capacity(data.len() + 4);
    output.extend_from_slice(&random_key.to_be_bytes());
    let mut xor_byte = checksum_byte(checksum);
    for byte in data {
        output.push(byte ^ xor_byte);
        xor_byte = byte;
    }
    output
}

fn copy_file_replacing(source: &Path, destination: &Path) -> GeoResult<()> {
    let data = fs::read(source)
        .map_err(|error| format!("Não foi possível ler o arquivo para cópia: {error}"))?;
    write_output_file(destination, &data)
}

fn write_output_file(destination: &Path, data: &[u8]) -> GeoResult<()> {
    let directory = destination.parent().ok_or("Destino de saída inválido.")?;
    fs::create_dir_all(directory)
        .map_err(|error| format!("Não foi possível criar a pasta de saída: {error}"))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(destination)
        .map_err(|error| {
            format!(
                "Não foi possível abrir {} para gravação: {error}",
                destination.display()
            )
        })?;
    file.write_all(data)
        .and_then(|_| file.flush())
        .map_err(|error| format!("Não foi possível gravar {}: {error}", destination.display()))
}

fn encode_height_and_nswe(height: i16, nswe: u8) -> i16 {
    ((i32::from(height) << 1) as i16 & !0x0f) | i16::from(nswe & 0x0f)
}

fn decode_height(value: i16) -> i16 {
    let height = (value & !0x0f) >> 1;
    height.clamp(-16_384, 16_376)
}

fn decode_nswe(value: i16) -> u8 {
    (value as u8) & 0x0f
}

fn checksum_byte(checksum: u32) -> u8 {
    ((checksum >> 24) as u8) ^ ((checksum >> 16) as u8) ^ ((checksum >> 8) as u8) ^ checksum as u8
}

fn read_u32_be(data: &[u8], offset: usize, message: &str) -> GeoResult<u32> {
    let bytes = data
        .get(offset..offset.checked_add(4).ok_or(message)?)
        .ok_or(message)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn write_i16(output: &mut Vec<u8>, value: i16) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn write_i32(output: &mut Vec<u8>, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn display_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("geodata")
        .to_owned()
}

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

fn report_completed<F>(completed: &AtomicUsize, total: usize, file_name: &str, progress: &F)
where
    F: Fn(GeodataProgress) + Send + Sync,
{
    let completed = completed.fetch_add(1, Ordering::Relaxed) + 1;
    progress(GeodataProgress {
        completed,
        total,
        file_name: file_name.to_owned(),
    });
}

fn record_conversion(totals: &Mutex<Totals>) {
    if let Ok(mut totals) = totals.lock() {
        totals.converted_files += 1;
    }
}

fn record_copy(totals: &Mutex<Totals>) {
    if let Ok(mut totals) = totals.lock() {
        totals.copied_files += 1;
    }
}

fn record_skip(totals: &Mutex<Totals>, error: String) {
    if let Ok(mut totals) = totals.lock() {
        totals.skipped_files += 1;
        push_error(&mut totals.errors, error);
    }
}

fn record_failure(totals: &Mutex<Totals>, error: String) {
    if let Ok(mut totals) = totals.lock() {
        totals.failed_files += 1;
        push_error(&mut totals.errors, error);
    }
}

fn push_error(errors: &mut Vec<String>, error: String) {
    if errors.len() < MAX_REPORTED_ERRORS {
        errors.push(error);
    }
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

    fn seek(&mut self, position: usize) -> GeoResult<()> {
        if position > self.data.len() {
            return Err("Leitura além do fim do arquivo geodata.".into());
        }
        self.position = position;
        Ok(())
    }

    fn skip(&mut self, length: usize) -> GeoResult<()> {
        self.seek(
            self.position
                .checked_add(length)
                .ok_or("Offset geodata inválido.")?,
        )
    }

    fn read_u8(&mut self) -> GeoResult<u8> {
        let byte = *self
            .data
            .get(self.position)
            .ok_or("Arquivo geodata truncado.")?;
        self.position += 1;
        Ok(byte)
    }

    fn read_i16(&mut self) -> GeoResult<i16> {
        let bytes = self.read_exact(2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn read_exact(&mut self, length: usize) -> GeoResult<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or("Offset geodata inválido.")?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or("Arquivo geodata truncado.")?;
        self.position = end;
        Ok(bytes)
    }

    fn bytes(&self, offset: usize, length: usize) -> GeoResult<&'a [u8]> {
        let end = offset
            .checked_add(length)
            .ok_or("Offset geodata inválido.")?;
        self.data
            .get(offset..end)
            .ok_or("Arquivo geodata truncado.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_region(x: i32, y: i32, height: i16) -> Region {
        Region::new(
            x,
            y,
            (0..REGION_SIZE * REGION_SIZE)
                .map(|_| {
                    Block::Flat(Cell {
                        height,
                        min_height: Some(height),
                        nswe: NSWE_ALL,
                    })
                })
                .collect(),
        )
        .unwrap()
    }

    fn flat_binary(tag: u8, height: i16) -> Vec<u8> {
        let mut output = Vec::with_capacity(REGION_SIZE * REGION_SIZE * 3);
        for _ in 0..REGION_SIZE * REGION_SIZE {
            output.push(tag);
            write_i16(&mut output, height);
        }
        output
    }

    fn encrypt_l2s(plain: Vec<u8>, random_key: u32) -> Vec<u8> {
        let mut checksum = GEO_CHECKSUM;
        for byte in b"127.0.0.1" {
            checksum ^= u32::from(*byte);
            checksum = checksum.rotate_right(1);
        }
        let mut output = Vec::with_capacity(plain.len() + 4);
        output.extend_from_slice(&random_key.to_be_bytes());
        let mut xor_byte = checksum_byte(checksum ^ random_key);
        for byte in plain {
            output.push(byte ^ xor_byte);
            xor_byte = byte;
        }
        output
    }

    fn l2m_with_multilevel_block(height: i16) -> Vec<u8> {
        let mut output = Vec::new();
        output.push(2);
        let header = (63_u16 << 5) | 1;
        for _ in 0..CELLS_PER_BLOCK {
            output.extend_from_slice(&header.to_le_bytes());
        }
        for _ in 0..CELLS_PER_BLOCK {
            write_i16(&mut output, encode_height_and_nswe(height, NSWE_ALL));
        }
        for _ in 1..REGION_SIZE * REGION_SIZE {
            output.push(0);
            write_i16(&mut output, height);
        }
        output
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "unreal-tools-geodata-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn height_and_nswe_round_trip() {
        let encoded = encode_height_and_nswe(-320, 13);
        assert_eq!(decode_height(encoded), -320);
        assert_eq!(decode_nswe(encoded), 13);
    }

    #[test]
    fn l2j_conv_dat_and_l2g_round_trip() {
        let region = flat_region(10, 11, 240);
        for format in [
            GeodataFormat::L2j,
            GeodataFormat::ConvDat,
            GeodataFormat::L2g,
        ] {
            let bytes = write_region(&region, format).unwrap();
            let root = test_root("roundtrip");
            fs::create_dir_all(&root).unwrap();
            let path = root.join(format!("10_11{}", format.extension()));
            fs::write(&path, bytes).unwrap();
            let decoded = parse_file(&path, format).unwrap();
            assert_eq!((decoded.x, decoded.y), (10, 11));
            match decoded.block(0, 0) {
                Block::Flat(cell) => assert_eq!(cell.height, 240),
                _ => panic!("expected flat block"),
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn parses_every_supported_source_format() {
        let root = test_root("all-formats");
        fs::create_dir_all(&root).unwrap();
        let region = flat_region(10, 10, 240);
        let cases = [
            (
                "10_10.l2j",
                GeodataFormat::L2j,
                write_region(&region, GeodataFormat::L2j).unwrap(),
            ),
            (
                "10_10_conv.dat",
                GeodataFormat::ConvDat,
                write_region(&region, GeodataFormat::ConvDat).unwrap(),
            ),
            ("10_10.l2d", GeodataFormat::L2d, flat_binary(0xd0, 240)),
            (
                "10_10.l2s",
                GeodataFormat::L2s,
                encrypt_l2s(flat_binary(0, 240), 0x1234_5678),
            ),
            (
                "10_10.l2g",
                GeodataFormat::L2g,
                write_region(&region, GeodataFormat::L2g).unwrap(),
            ),
            ("10_10.rp", GeodataFormat::Rp, flat_binary(0xd0, 240)),
            (
                "10_10.l2m",
                GeodataFormat::L2m,
                l2m_with_multilevel_block(240),
            ),
        ];
        for (name, format, bytes) in cases {
            let path = root.join(name);
            fs::write(&path, bytes).unwrap();
            let decoded = parse_file(&path, format).unwrap();
            assert_eq!((decoded.x, decoded.y), (10, 10));
            if format == GeodataFormat::L2m {
                assert!(matches!(decoded.block(0, 0), Block::MultiLevel(_)));
            }
        }

        let path_text = root.join("10_10_path.txt");
        fs::write(&path_text, "[0,0]1(240:1111)\n").unwrap();
        let decoded = parse_file(&path_text, GeodataFormat::PathTxt).unwrap();
        assert_eq!((decoded.x, decoded.y), (10, 10));
        assert!(matches!(decoded.block(0, 0), Block::Complex(_)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn truncated_region_is_an_error_not_a_panic() {
        let root = test_root("truncated");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("10_10.l2j");
        fs::write(&path, [0_u8, 0]).unwrap();
        assert!(parse_file(&path, GeodataFormat::L2j).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_preserves_tree_overwrites_existing_output_and_keeps_first_batch_conflict() {
        let root = test_root("batch");
        let input = root.join("input");
        let nested = input.join("nested");
        let output = root.join("output");
        fs::create_dir_all(&nested).unwrap();
        let region = flat_region(10, 10, 100);
        fs::write(
            input.join("10_10.l2j"),
            write_region(&region, GeodataFormat::L2j).unwrap(),
        )
        .unwrap();
        fs::write(
            input.join("10_10.l2g"),
            write_region(&region, GeodataFormat::L2g).unwrap(),
        )
        .unwrap();
        fs::write(
            input.join("11_10.l2j"),
            write_region(&flat_region(11, 10, 120), GeodataFormat::L2j).unwrap(),
        )
        .unwrap();
        fs::write(
            nested.join("10_10.l2g"),
            write_region(&region, GeodataFormat::L2g).unwrap(),
        )
        .unwrap();
        fs::create_dir_all(&output).unwrap();
        fs::write(output.join("11_10.l2j"), b"obsolete result").unwrap();
        let summary = convert_directory_with_progress(
            &input.to_string_lossy(),
            &output.to_string_lossy(),
            GeodataFormat::L2j,
            |_| {},
        )
        .unwrap();
        assert_eq!(summary.total_files, 4);
        assert_eq!(summary.copied_files, 1);
        assert_eq!(summary.converted_files, 2);
        assert_eq!(summary.skipped_files, 1);
        assert!(output.join("10_10.l2j").is_file());
        assert!(output.join("11_10.l2j").is_file());
        assert!(output.join("nested").join("10_10.l2j").is_file());
        assert_eq!(
            fs::read(output.join("11_10.l2j")).unwrap(),
            fs::read(input.join("11_10.l2j")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_reports_invalid_coordinates_without_stopping() {
        let root = test_root("invalid_coordinates");
        let input = root.join("input");
        let output = root.join("output");
        fs::create_dir_all(&input).unwrap();
        fs::write(
            input.join("9_10.l2j"),
            write_region(&flat_region(10, 10, 100), GeodataFormat::L2j).unwrap(),
        )
        .unwrap();

        let summary = convert_directory_with_progress(
            &input.to_string_lossy(),
            &output.to_string_lossy(),
            GeodataFormat::L2g,
            |_| {},
        )
        .unwrap();
        assert_eq!(summary.total_files, 1);
        assert_eq!(summary.failed_files, 1);
        assert!(summary.errors[0].contains("fora do intervalo 10..26"));
        fs::remove_dir_all(root).unwrap();
    }
}
