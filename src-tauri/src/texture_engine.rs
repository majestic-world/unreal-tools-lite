//! UE2 texture-property engine for Lineage 2 UTX packages.
//!
//! This module is intentionally independent from Tauri. It is the application's
//! native UTX writer: it owns the UE2 property-tag codec, mutable package
//! model, and editor-oriented changes such as alpha, Split9, and animation
//! settings.
//!
//! Unknown properties are kept byte-for-byte until explicitly changed. That is
//! essential for properties such as `MipZero`, indexed `InternalTime`, and
//! clamp settings that are present in real game packages but are not exposed in
//! the editor yet.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub type TextureEngineResult<T> = Result<T, String>;

pub const UE2_PACKAGE_VERSION: i32 = 118;
pub const UE2_LICENSEE_VERSION: i32 = 0;
const PACKAGE_MAGIC: i32 = 0x9e2a83c1_u32 as i32;

const TYPE_BYTE: u8 = 1;
const TYPE_INT: u8 = 2;
const TYPE_BOOL: u8 = 3;
const TYPE_FLOAT: u8 = 4;
const TYPE_OBJECT: u8 = 5;

/// A UE2 name table owned by the package writer.
///
/// The engine only creates ASCII property names, but existing Unicode names
/// can still be retained and serialized when a package is rebuilt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameTable {
    entries: Vec<NameEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameEntry {
    pub name: String,
    pub flags: i32,
}

impl NameTable {
    pub fn new(entries: Vec<NameEntry>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[NameEntry] {
        &self.entries
    }

    pub fn name(&self, index: i32) -> TextureEngineResult<&str> {
        let index = usize::try_from(index).map_err(|_| "Índice de nome inválido.")?;
        self.entries
            .get(index)
            .map(|entry| entry.name.as_str())
            .ok_or_else(|| "Índice de nome fora da tabela do pacote.".into())
    }

    pub fn index_of(&self, name: &str) -> Option<i32> {
        self.entries
            .iter()
            .position(|entry| entry.name.eq_ignore_ascii_case(name))
            .and_then(|index| i32::try_from(index).ok())
    }

    pub fn intern(&mut self, name: &str) -> TextureEngineResult<i32> {
        if let Some(index) = self.index_of(name) {
            return Ok(index);
        }
        if !name.is_ascii() || name.is_empty() {
            return Err("Nomes criados pelo motor devem ser ASCII e não vazios.".into());
        }
        let index = i32::try_from(self.entries.len())
            .map_err(|_| "A tabela de nomes excede o limite do formato.")?;
        let flags = self
            .entries
            .first()
            .map(|entry| entry.flags)
            .unwrap_or(0x0007_0000);
        self.entries.push(NameEntry {
            name: name.to_owned(),
            flags,
        });
        Ok(index)
    }

    fn index_map(&self) -> HashMap<String, i32> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                i32::try_from(index)
                    .ok()
                    .map(|index| (entry.name.to_ascii_lowercase(), index))
            })
            .collect()
    }

    fn intern_indexed(
        &mut self,
        name: &str,
        indices: &mut HashMap<String, i32>,
    ) -> TextureEngineResult<i32> {
        let key = name.to_ascii_lowercase();
        if let Some(index) = indices.get(&key) {
            return Ok(*index);
        }
        if !name.is_ascii() || name.is_empty() {
            return Err("Nomes criados pelo motor devem ser ASCII e não vazios.".into());
        }
        let index = i32::try_from(self.entries.len())
            .map_err(|_| "A tabela de nomes excede o limite do formato.")?;
        let flags = self
            .entries
            .first()
            .map(|entry| entry.flags)
            .unwrap_or(0x0007_0000);
        self.entries.push(NameEntry {
            name: name.to_owned(),
            flags,
        });
        indices.insert(key, index);
        Ok(index)
    }

    pub fn serialize(&self) -> TextureEngineResult<Vec<u8>> {
        let mut output = Vec::new();
        for entry in &self.entries {
            write_unreal_string(&mut output, &entry.name)?;
            output.extend_from_slice(&entry.flags.to_le_bytes());
        }
        Ok(output)
    }
}

/// Values that can be edited without losing UE2 tag semantics.
#[derive(Debug, Clone, PartialEq)]
enum PropertyValue {
    Byte(u8),
    Int(i32),
    Bool(bool),
    Float(f32),
    Object(i32),
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
struct PropertyRecord {
    name: String,
    name_index: i32,
    raw: Vec<u8>,
    value: PropertyValue,
    replacement: Option<PropertyValue>,
}

impl PropertyRecord {
    fn current_value(&self) -> &PropertyValue {
        self.replacement.as_ref().unwrap_or(&self.value)
    }
}

#[derive(Debug, Clone)]
struct ExistingPropertyLocation {
    name: String,
    property_type: u8,
    info_offset: usize,
    value_offset: usize,
    size: usize,
}

/// Texture property stream, excluding the mip payload that follows `None`.
///
/// `trailing_offset` is where the texture serializer must append the original
/// mip serialization after writing the edited property stream.
#[derive(Debug, Clone)]
pub struct TexturePropertyStream {
    properties: Vec<PropertyRecord>,
    trailing_offset: usize,
    tail_offset: usize,
}

impl TexturePropertyStream {
    pub fn parse(data: &[u8], names: &NameTable) -> TextureEngineResult<Self> {
        let mut reader = Reader::new(data);
        let mut properties = Vec::new();
        loop {
            let property_start = reader.position();
            let name_index = reader.read_compact()?;
            let name = names.name(name_index)?.to_owned();
            if name.eq_ignore_ascii_case("None") {
                return Ok(Self {
                    properties,
                    trailing_offset: property_start,
                    tail_offset: reader.position(),
                });
            }

            let info = reader.read_u8()?;
            let property_type = info & 0x0f;
            let size_type = (info >> 4) & 0x07;
            let is_array = info & 0x80 != 0;
            if property_type == 10 {
                reader.read_compact()?;
            }
            let size = property_size(&mut reader, size_type)?;
            if is_array && property_type != TYPE_BOOL {
                reader.read_compact()?;
            }
            let value_bytes = reader.read_exact(size)?;
            let raw = data
                .get(property_start..reader.position())
                .ok_or("Dados de propriedade truncados.")?
                .to_vec();
            let value = decode_property_value(property_type, info, value_bytes)?;
            properties.push(PropertyRecord {
                name,
                name_index,
                raw,
                value,
                replacement: None,
            });
        }
    }

    pub fn trailing_offset(&self) -> usize {
        self.trailing_offset
    }

    fn tail_offset(&self) -> usize {
        self.tail_offset
    }

    pub fn editor_state(&self) -> TextureEditorState {
        TextureEditorState {
            alpha: self.bool_value("bAlphaTexture"),
            masked: self.bool_value("bMasked"),
            u_clamp: self.integer_value("UClamp"),
            v_clamp: self.integer_value("VClamp"),
            u_clamp_mode: self.integer_value("UClampMode"),
            v_clamp_mode: self.integer_value("VClampMode"),
            split9: self.bool_value("bSplit9Texture"),
            split9_x1: self.int_value("Split9X1").unwrap_or_default(),
            split9_x2: self.int_value("Split9X2").unwrap_or_default(),
            split9_x3: self.int_value("Split9X3").unwrap_or_default(),
            split9_y1: self.int_value("Split9Y1").unwrap_or_default(),
            split9_y2: self.int_value("Split9Y2").unwrap_or_default(),
            split9_y3: self.int_value("Split9Y3").unwrap_or_default(),
            animation: TextureAnimationState {
                anim_next: self.object_value("AnimNext"),
                max_frame_rate: self.float_value("MaxFrameRate"),
                min_frame_rate: self.float_value("MinFrameRate"),
                one_time_anim_loop: self.bool_value("OneTimeAnimLoop"),
                prime_count: self.byte_value("PrimeCount").map(i32::from),
                total_frame_num: self.int_value("TotalFrameNum"),
            },
        }
    }

    /// Applies UI-ready settings. `None` means "leave the current value
    /// untouched"; `Some(false)` is a real UE2 boolean value.
    pub fn apply_editor_edit(
        &mut self,
        names: &mut NameTable,
        edit: &TextureEditorEdit,
    ) -> TextureEngineResult<()> {
        if let Some(alpha) = edit.alpha {
            self.set_value(names, "bAlphaTexture", PropertyValue::Bool(alpha))?;
        }
        if let Some(masked) = edit.masked {
            self.set_value(names, "bMasked", PropertyValue::Bool(masked))?;
        }
        if let Some(clamp) = edit.clamp {
            if let Some(value) = clamp.u_clamp {
                self.set_integer_value(names, "UClamp", value, false)?;
            }
            if let Some(value) = clamp.v_clamp {
                self.set_integer_value(names, "VClamp", value, false)?;
            }
            if let Some(value) = clamp.u_clamp_mode {
                self.set_integer_value(names, "UClampMode", value, true)?;
            }
            if let Some(value) = clamp.v_clamp_mode {
                self.set_integer_value(names, "VClampMode", value, true)?;
            }
        }
        if let Some(split9) = edit.split9 {
            self.set_value(names, "bSplit9Texture", PropertyValue::Bool(split9.enabled))?;
            if split9.enabled {
                for (name, value) in [
                    ("Split9X1", split9.x1),
                    ("Split9X2", split9.x2),
                    ("Split9X3", split9.x3),
                    ("Split9Y1", split9.y1),
                    ("Split9Y2", split9.y2),
                    ("Split9Y3", split9.y3),
                ] {
                    self.set_value(names, name, PropertyValue::Int(value))?;
                }
            }
        }
        if let Some(animation) = &edit.animation {
            if let Some(value) = animation.anim_next {
                self.set_value(names, "AnimNext", PropertyValue::Object(value))?;
            }
            if let Some(value) = animation.max_frame_rate {
                validate_finite(value, "MaxFrameRate")?;
                self.set_value(names, "MaxFrameRate", PropertyValue::Float(value))?;
            }
            if let Some(value) = animation.min_frame_rate {
                validate_finite(value, "MinFrameRate")?;
                self.set_value(names, "MinFrameRate", PropertyValue::Float(value))?;
            }
            if let Some(value) = animation.one_time_anim_loop {
                self.set_value(names, "OneTimeAnimLoop", PropertyValue::Bool(value))?;
            }
            if let Some(value) = animation.prime_count {
                self.set_value(
                    names,
                    "PrimeCount",
                    PropertyValue::Byte(
                        u8::try_from(value).map_err(|_| "PrimeCount deve estar entre 0 e 255.")?,
                    ),
                )?;
            }
            if let Some(value) = animation.total_frame_num {
                self.set_value(names, "TotalFrameNum", PropertyValue::Int(value))?;
            }
        }
        Ok(())
    }

    /// Rebuilds only the UE2 property stream. The caller appends the original
    /// texture tail beginning at [`Self::trailing_offset`].
    pub fn serialize(&self, names: &NameTable) -> TextureEngineResult<Vec<u8>> {
        let none_index = names
            .index_of("None")
            .ok_or("A tabela de nomes não possui o terminador None.")?;
        let mut output = Vec::new();
        for property in &self.properties {
            match &property.replacement {
                Some(value) => write_property(&mut output, property.name_index, value)?,
                None => output.extend_from_slice(&property.raw),
            }
        }
        write_compact(&mut output, none_index);
        Ok(output)
    }

    fn set_value(
        &mut self,
        names: &mut NameTable,
        name: &str,
        value: PropertyValue,
    ) -> TextureEngineResult<()> {
        if let Some(property) = self
            .properties
            .iter_mut()
            .find(|property| property.name.eq_ignore_ascii_case(name))
        {
            property.replacement = Some(value);
            return Ok(());
        }
        let name_index = names.intern(name)?;
        self.properties.push(PropertyRecord {
            name: name.to_owned(),
            name_index,
            raw: Vec::new(),
            value: PropertyValue::Raw(Vec::new()),
            replacement: Some(value),
        });
        Ok(())
    }

    fn set_integer_value(
        &mut self,
        names: &mut NameTable,
        name: &str,
        value: i32,
        prefer_byte: bool,
    ) -> TextureEngineResult<()> {
        if let Some(property) = self
            .properties
            .iter_mut()
            .find(|property| property.name.eq_ignore_ascii_case(name))
        {
            property.replacement = Some(match property.current_value() {
                PropertyValue::Byte(_) => PropertyValue::Byte(
                    u8::try_from(value).map_err(|_| format!("{name} deve estar entre 0 e 255."))?,
                ),
                PropertyValue::Int(_) => PropertyValue::Int(value),
                _ => {
                    return Err(format!(
                        "A propriedade {name} possui um tipo UE2 incompatível para edição."
                    ))
                }
            });
            return Ok(());
        }
        self.set_value(
            names,
            name,
            if prefer_byte {
                PropertyValue::Byte(
                    u8::try_from(value).map_err(|_| format!("{name} deve estar entre 0 e 255."))?,
                )
            } else {
                PropertyValue::Int(value)
            },
        )
    }

    fn value(&self, name: &str) -> Option<&PropertyValue> {
        self.properties
            .iter()
            .find(|property| property.name.eq_ignore_ascii_case(name))
            .map(PropertyRecord::current_value)
    }

    fn bool_value(&self, name: &str) -> Option<bool> {
        match self.value(name) {
            Some(PropertyValue::Bool(value)) => Some(*value),
            _ => None,
        }
    }

    fn byte_value(&self, name: &str) -> Option<u8> {
        match self.value(name) {
            Some(PropertyValue::Byte(value)) => Some(*value),
            _ => None,
        }
    }

    fn int_value(&self, name: &str) -> Option<i32> {
        match self.value(name) {
            Some(PropertyValue::Int(value)) => Some(*value),
            _ => None,
        }
    }

    fn integer_value(&self, name: &str) -> Option<i32> {
        match self.value(name) {
            Some(PropertyValue::Byte(value)) => Some(i32::from(*value)),
            Some(PropertyValue::Int(value)) => Some(*value),
            _ => None,
        }
    }

    fn float_value(&self, name: &str) -> Option<f32> {
        match self.value(name) {
            Some(PropertyValue::Float(value)) => Some(*value),
            _ => None,
        }
    }

    fn object_value(&self, name: &str) -> Option<i32> {
        match self.value(name) {
            Some(PropertyValue::Object(value)) => Some(*value),
            _ => None,
        }
    }
}

/// Read-only state that a future Tauri command can send directly to React.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureEditorState {
    pub alpha: Option<bool>,
    pub masked: Option<bool>,
    pub u_clamp: Option<i32>,
    pub v_clamp: Option<i32>,
    pub u_clamp_mode: Option<i32>,
    pub v_clamp_mode: Option<i32>,
    pub split9: Option<bool>,
    pub split9_x1: i32,
    pub split9_x2: i32,
    pub split9_x3: i32,
    pub split9_y1: i32,
    pub split9_y2: i32,
    pub split9_y3: i32,
    pub animation: TextureAnimationState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureAnimationState {
    pub anim_next: Option<i32>,
    pub max_frame_rate: Option<f32>,
    pub min_frame_rate: Option<f32>,
    pub one_time_anim_loop: Option<bool>,
    pub prime_count: Option<i32>,
    pub total_frame_num: Option<i32>,
}

/// Mutation payload for a future GUI command. All top-level fields are
/// optional so one interaction cannot accidentally overwrite another setting.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureEditorEdit {
    pub alpha: Option<bool>,
    pub masked: Option<bool>,
    pub clamp: Option<TextureClampEdit>,
    pub split9: Option<Split9Edit>,
    pub animation: Option<TextureAnimationEdit>,
}

impl TextureEditorEdit {
    fn has_changes(&self) -> bool {
        self.alpha.is_some()
            || self.masked.is_some()
            || self
                .clamp
                .as_ref()
                .is_some_and(TextureClampEdit::has_changes)
            || self.split9.is_some()
            || self.animation.as_ref().is_some_and(|animation| {
                animation.anim_next.is_some()
                    || animation.max_frame_rate.is_some()
                    || animation.min_frame_rate.is_some()
                    || animation.one_time_anim_loop.is_some()
                    || animation.prime_count.is_some()
                    || animation.total_frame_num.is_some()
            })
    }
}

/// Texture addressing values used by the UE2 sampler. `None` preserves the
/// existing property, which lets a metadata file override a single axis only.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureClampEdit {
    pub u_clamp: Option<i32>,
    pub v_clamp: Option<i32>,
    pub u_clamp_mode: Option<i32>,
    pub v_clamp_mode: Option<i32>,
}

impl TextureClampEdit {
    fn has_changes(&self) -> bool {
        self.u_clamp.is_some()
            || self.v_clamp.is_some()
            || self.u_clamp_mode.is_some()
            || self.v_clamp_mode.is_some()
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Split9Edit {
    pub enabled: bool,
    pub x1: i32,
    pub x2: i32,
    pub x3: i32,
    pub y1: i32,
    pub y2: i32,
    pub y3: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureAnimationEdit {
    pub anim_next: Option<i32>,
    pub max_frame_rate: Option<f32>,
    pub min_frame_rate: Option<f32>,
    pub one_time_anim_loop: Option<bool>,
    pub prime_count: Option<i32>,
    pub total_frame_num: Option<i32>,
}

/// Texture data already decoded by the UTX frontend layer. The package engine
/// owns the binary packaging, while callers remain free to decode TGA/DDS by
/// the most appropriate UI workflow.
#[derive(Debug, Clone)]
pub struct TextureImportRequest {
    pub name: String,
    pub format: u8,
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
    pub alpha: Option<bool>,
    pub masked: Option<bool>,
    pub clamp: Option<TextureClampEdit>,
    pub split9: Option<Split9Edit>,
    pub animation: Option<TextureAnimationImport>,
}

#[derive(Debug, Clone)]
pub struct TextureAnimationImport {
    pub anim_next: Option<String>,
    pub max_frame_rate: Option<f32>,
    pub min_frame_rate: Option<f32>,
    pub one_time_anim_loop: Option<bool>,
    pub prime_count: Option<i32>,
    pub total_frame_num: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureImportOutcome {
    pub export_index: usize,
    pub added: bool,
}

fn decode_property_value(
    property_type: u8,
    info: u8,
    bytes: &[u8],
) -> TextureEngineResult<PropertyValue> {
    match (property_type, bytes.len()) {
        (TYPE_BYTE, 1) => Ok(PropertyValue::Byte(bytes[0])),
        (TYPE_INT, 4) => Ok(PropertyValue::Int(i32::from_le_bytes(
            bytes
                .try_into()
                .map_err(|_| "Propriedade inteira inválida.")?,
        ))),
        (TYPE_BOOL, _) => Ok(PropertyValue::Bool(info & 0x80 != 0)),
        (TYPE_FLOAT, 4) => Ok(PropertyValue::Float(f32::from_le_bytes(
            bytes
                .try_into()
                .map_err(|_| "Propriedade decimal inválida.")?,
        ))),
        (TYPE_OBJECT, size) if size > 0 => {
            let mut reader = Reader::new(bytes);
            Ok(PropertyValue::Object(reader.read_compact()?))
        }
        _ => Ok(PropertyValue::Raw(bytes.to_vec())),
    }
}

fn write_property(
    output: &mut Vec<u8>,
    name_index: i32,
    value: &PropertyValue,
) -> TextureEngineResult<()> {
    match value {
        PropertyValue::Byte(value) => {
            write_sized_property(output, name_index, TYPE_BYTE, &[*value])
        }
        PropertyValue::Int(value) => {
            write_sized_property(output, name_index, TYPE_INT, &value.to_le_bytes())
        }
        PropertyValue::Bool(value) => {
            write_compact(output, name_index);
            // UE2 bools carry their value in bit 7 and use an explicit
            // zero-sized payload. The extra zero is the size value for 0x50.
            output.push(0x50 | TYPE_BOOL | if *value { 0x80 } else { 0 });
            output.push(0);
            Ok(())
        }
        PropertyValue::Float(value) => {
            write_sized_property(output, name_index, TYPE_FLOAT, &value.to_le_bytes())
        }
        PropertyValue::Object(value) => {
            let mut compact = Vec::new();
            write_compact(&mut compact, *value);
            write_sized_property(output, name_index, TYPE_OBJECT, &compact)
        }
        PropertyValue::Raw(_) => {
            Err("Não é possível serializar uma propriedade bruta alterada.".into())
        }
    }
}

fn write_sized_property(
    output: &mut Vec<u8>,
    name_index: i32,
    property_type: u8,
    value: &[u8],
) -> TextureEngineResult<()> {
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
        _ => return Err("Propriedade grande demais para o codificador UE2.".into()),
    }
    output.extend_from_slice(value);
    Ok(())
}

fn property_size(reader: &mut Reader<'_>, size_type: u8) -> TextureEngineResult<usize> {
    match size_type {
        0 => Ok(1),
        1 => Ok(2),
        2 => Ok(4),
        3 => Ok(12),
        4 => Ok(16),
        5 => Ok(reader.read_u8()? as usize),
        6 => Ok(reader.read_u16()? as usize),
        7 => usize::try_from(reader.read_i32()?)
            .map_err(|_| "Tamanho de propriedade inválido.".into()),
        _ => Err("Tipo de tamanho de propriedade inválido.".into()),
    }
}

fn validate_finite(value: f32, name: &str) -> TextureEngineResult<()> {
    value
        .is_finite()
        .then_some(())
        .ok_or_else(|| format!("{name} deve ser um número finito."))
}

fn write_unreal_string(output: &mut Vec<u8>, value: &str) -> TextureEngineResult<()> {
    if value.is_ascii() {
        let length = value
            .len()
            .checked_add(1)
            .ok_or("Nome Unreal grande demais.")?;
        write_compact(
            output,
            i32::try_from(length).map_err(|_| "Nome Unreal grande demais.")?,
        );
        output.extend_from_slice(value.as_bytes());
        output.push(0);
        return Ok(());
    }
    let mut units = value.encode_utf16().collect::<Vec<_>>();
    units.push(0);
    let length = i32::try_from(units.len()).map_err(|_| "Nome Unreal grande demais.")?;
    write_compact(output, -length);
    for unit in units {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
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

    fn seek(&mut self, position: usize) -> TextureEngineResult<()> {
        if position > self.data.len() {
            return Err("Leitura fora dos limites do pacote.".into());
        }
        self.position = position;
        Ok(())
    }

    fn skip(&mut self, size: usize) -> TextureEngineResult<()> {
        self.seek(
            self.position
                .checked_add(size)
                .ok_or("Offset de propriedade inválido.")?,
        )
    }

    fn read_u8(&mut self) -> TextureEngineResult<u8> {
        let value = *self
            .data
            .get(self.position)
            .ok_or("Dados de propriedade truncados.")?;
        self.position += 1;
        Ok(value)
    }

    fn read_u16(&mut self) -> TextureEngineResult<u16> {
        Ok(u16::from_le_bytes(
            self.read_exact(2)?
                .try_into()
                .map_err(|_| "Inteiro de propriedade inválido.")?,
        ))
    }

    fn read_i32(&mut self) -> TextureEngineResult<i32> {
        Ok(i32::from_le_bytes(
            self.read_exact(4)?
                .try_into()
                .map_err(|_| "Inteiro de propriedade inválido.")?,
        ))
    }

    fn read_unreal_string(&mut self) -> TextureEngineResult<String> {
        let length = self.read_compact()?;
        if length == 0 {
            return Ok(String::new());
        }
        if length > 0 {
            let length = length as usize;
            let bytes = self.read_exact(length)?;
            return Ok(String::from_utf8_lossy(&bytes[..length.saturating_sub(1)]).into_owned());
        }
        let units = self
            .read_exact(length.checked_abs().ok_or("String Unreal inválida.")? as usize * 2)?
            .get(..length.unsigned_abs() as usize * 2 - 2)
            .ok_or("String Unreal truncada.")?
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        Ok(String::from_utf16_lossy(&units))
    }

    fn read_compact(&mut self) -> TextureEngineResult<i32> {
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
            i32::try_from(output).map_err(|_| "Inteiro compacto inválido.".into())
        }
    }

    fn read_exact(&mut self, size: usize) -> TextureEngineResult<&'a [u8]> {
        let end = self
            .position
            .checked_add(size)
            .ok_or("Offset de propriedade inválido.")?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or("Dados de propriedade truncados.")?;
        self.position = end;
        Ok(bytes)
    }
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
struct TexturePackage {
    data: Vec<u8>,
    version: i32,
    licensee: i32,
    name_offset: usize,
    names: NameTable,
    imports: Vec<ImportEntry>,
    exports: Vec<ExportEntry>,
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
    mip_count_offset: usize,
    mip_width_offset: usize,
    mip_payload_offset: usize,
}

struct MipLocation {
    mip_count_offset: usize,
    pixel_offset: usize,
    width_offset_position: usize,
    size: usize,
    width: i32,
    height: i32,
}

struct SerializedTexture {
    bytes: Vec<u8>,
    mip_width_offset: usize,
    width_offset_value: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum TextureSeedKind {
    Common,
    Split9,
    Animation,
}

#[derive(Clone, Copy)]
enum TextureSeedSource {
    Package(usize),
    Template(usize),
}

/// Creates a clean UTX from the editor-generated template. The template's
/// exports are intentionally omitted; only the package structure remains.
pub fn create_empty_package(
    template_data: &[u8],
    template_package_name: &str,
    destination_package_name: &str,
) -> TextureEngineResult<Vec<u8>> {
    let template = TexturePackage::parse(template_data.to_vec())?;
    template.create_empty(template_package_name, destination_package_name)
}

/// Adds texture exports through the native package writer. `template_data` is
/// used only when the target does not already contain a compatible structural
/// seed (`Common`, `TlpSpt9`, or `TlpAnim`).
pub fn import_new_textures(
    package_data: Vec<u8>,
    template_data: &[u8],
    package_name: &str,
    textures: &[TextureImportRequest],
) -> TextureEngineResult<(Vec<u8>, Vec<TextureImportOutcome>)> {
    let mut package = TexturePackage::parse(package_data)?;
    let template = TexturePackage::parse(template_data.to_vec())?;
    package.add_textures(&template, package_name, textures)
}

/// Replaces known exports without rebuilding package tables. Callers that
/// already indexed the UTX can provide export indices directly, avoiding the
/// add-or-replace ambiguity of a mixed import and keeping existing serial
/// layouts intact.
pub fn replace_existing_textures(
    package_data: Vec<u8>,
    replacements: &[(usize, TextureImportRequest)],
) -> TextureEngineResult<(Vec<u8>, Vec<TextureImportOutcome>)> {
    let mut package = TexturePackage::parse(package_data)?;
    let mut edits = Vec::new();
    let mut outcomes = Vec::with_capacity(replacements.len());
    for (export_index, texture) in replacements {
        package.replace_pixels(*export_index, texture)?;
        if let Some(edit) = package.import_edit(texture, *export_index)? {
            edits.push((*export_index, edit));
        }
        outcomes.push(TextureImportOutcome {
            export_index: *export_index,
            added: false,
        });
    }
    package.apply_texture_edits_in_place(edits)?;
    Ok((package.data, outcomes))
}

/// Native import flow used by the UTX frontend: matching names are replaced;
/// unmatched names are created in the selected group. All property changes are
/// written by this engine after the final export layout is known.
pub fn import_textures(
    package_data: Vec<u8>,
    template_data: &[u8],
    package_name: &str,
    textures: &[TextureImportRequest],
) -> TextureEngineResult<(Vec<u8>, Vec<TextureImportOutcome>)> {
    let mut package = TexturePackage::parse(package_data)?;
    let template = TexturePackage::parse(template_data.to_vec())?;
    let mut outcomes = vec![None; textures.len()];
    let mut replacements = Vec::new();
    let mut additions = Vec::new();
    let existing_textures = package.texture_index_by_group()?;
    for (request_index, texture) in textures.iter().enumerate() {
        match existing_textures.get(&texture_lookup_key(package_name, &texture.name)) {
            Some(export_index) => {
                package.replace_pixels(*export_index, texture)?;
                replacements.push((*export_index, texture));
                outcomes[request_index] = Some(TextureImportOutcome {
                    export_index: *export_index,
                    added: false,
                });
            }
            None => additions.push((request_index, texture)),
        }
    }
    if !additions.is_empty() {
        let addition_requests = additions
            .iter()
            .map(|(_, texture)| (*texture).clone())
            .collect::<Vec<_>>();
        let (_, added) = package.add_textures(&template, package_name, &addition_requests)?;
        for ((request_index, _), outcome) in additions.into_iter().zip(added) {
            outcomes[request_index] = Some(outcome);
        }
    }
    let mut edits = Vec::new();
    for (export_index, texture) in replacements {
        if let Some(edit) = package.import_edit(texture, export_index)? {
            edits.push((export_index, edit));
        }
    }
    package.apply_texture_edits_in_place(edits)?;
    Ok((
        package.data,
        outcomes
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or("Resultado de importação incompleto.")?,
    ))
}

/// Replaces one existing texture through the native writer while retaining all
/// untouched texture properties and rebuilding package tables when metadata
/// adds or changes UE2 properties.
pub fn replace_texture(
    package_data: Vec<u8>,
    export_index: usize,
    texture: &TextureImportRequest,
) -> TextureEngineResult<Vec<u8>> {
    let mut package = TexturePackage::parse(package_data)?;
    package.replace_pixels(export_index, texture)?;
    if let Some(edit) = package.import_edit(texture, export_index)? {
        package.apply_texture_edits_in_place(vec![(export_index, edit)])?;
    }
    Ok(package.data)
}

/// Clones an existing texture export into the requested group. The texture
/// payload and property stream are copied byte-for-byte; only the export name,
/// outer group, and relocated mip pointer are changed.
pub fn duplicate_texture(
    package_data: Vec<u8>,
    source_export_index: usize,
    group_name: &str,
    texture_name: &str,
) -> TextureEngineResult<(Vec<u8>, usize)> {
    let mut package = TexturePackage::parse(package_data)?;
    let export_index = package.duplicate_texture(source_export_index, group_name, texture_name)?;
    Ok((package.data, export_index))
}

/// Renames one texture export without changing its object index, payload, or
/// property stream. UE2 object references therefore remain intact.
pub fn rename_texture(
    package_data: Vec<u8>,
    export_index: usize,
    texture_name: &str,
) -> TextureEngineResult<Vec<u8>> {
    let mut package = TexturePackage::parse(package_data)?;
    package.rename_texture(export_index, texture_name)?;
    Ok(package.data)
}

/// Returns the editable UE2 properties for one `Engine.Texture` export.
pub fn texture_editor_state(
    package_data: Vec<u8>,
    export_index: usize,
) -> TextureEngineResult<TextureEditorState> {
    let package = TexturePackage::parse(package_data)?;
    let export = package.export(export_index)?;
    if !package.is_texture(export) {
        return Err("A entrada selecionada não é uma textura Engine.Texture.".into());
    }
    let raw = package.export_data(export)?;
    TexturePropertyStream::parse(raw, &package.names).map(|properties| properties.editor_state())
}

/// Applies editor-facing properties without replacing the texture pixels.
pub fn edit_texture_properties(
    package_data: Vec<u8>,
    export_index: usize,
    edit: TextureEditorEdit,
) -> TextureEngineResult<Vec<u8>> {
    let mut package = TexturePackage::parse(package_data)?;
    let export = package.export(export_index)?;
    if !package.is_texture(export) {
        return Err("A entrada selecionada não é uma textura Engine.Texture.".into());
    }
    if edit.has_changes() {
        package.apply_texture_edits(vec![(export_index, edit)])?;
    }
    Ok(package.data)
}

/// Applies the same kind of editor changes to several textures before the
/// caller writes the rebuilt package once. Each change still preserves every
/// untouched UE2 property and the texture pixel data.
pub fn edit_texture_properties_batch(
    mut package_data: Vec<u8>,
    edits: &[(usize, TextureEditorEdit)],
) -> TextureEngineResult<Vec<u8>> {
    for (export_index, edit) in edits {
        package_data = edit_texture_properties(package_data, *export_index, edit.clone())?;
    }
    Ok(package_data)
}

impl TexturePackage {
    fn parse(data: Vec<u8>) -> TextureEngineResult<Self> {
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
            names.push(NameEntry {
                name: reader.read_unreal_string()?,
                flags: reader.read_i32()?,
            });
        }
        reader.seek(import_offset)?;
        let mut imports = Vec::with_capacity(import_count);
        for _ in 0..import_count {
            imports.push(ImportEntry {
                class_package: reader.read_compact()?,
                class_name: reader.read_compact()?,
                package: reader.read_i32()?,
                name_index: reader.read_compact()?,
            });
        }
        let (exports, _) = read_export_table(&data, export_offset, export_count, names.len())?;
        Ok(Self {
            data,
            version,
            licensee,
            name_offset,
            names: NameTable::new(names),
            imports,
            exports,
        })
    }

    fn create_empty(
        &self,
        template_package_name: &str,
        destination_package_name: &str,
    ) -> TextureEngineResult<Vec<u8>> {
        if self.version != UE2_PACKAGE_VERSION || self.licensee != UE2_LICENSEE_VERSION {
            return Err("O template não usa a versão UE2 v118 esperada pelo motor.".into());
        }
        let mut names = self.names.clone();
        let template_index = names
            .index_of(template_package_name)
            .ok_or("O template não possui o nome de pacote esperado.")?;
        let entry = names
            .entries
            .get_mut(usize::try_from(template_index).map_err(|_| "Índice de nome inválido.")?)
            .ok_or("Índice de nome inválido.")?;
        entry.name = destination_package_name.to_owned();
        let name_table = names.serialize()?;
        let header = self
            .data
            .get(..self.name_offset)
            .ok_or("O cabeçalho do template UTX está truncado.")?;
        let import_table = serialize_import_table(&self.imports);
        let import_offset = header
            .len()
            .checked_add(name_table.len())
            .ok_or("O novo UTX excede o limite de tamanho.")?;
        let export_offset = import_offset
            .checked_add(import_table.len())
            .ok_or("O novo UTX excede o limite de tamanho.")?;
        let mut output = Vec::with_capacity(export_offset);
        output.extend_from_slice(header);
        output.extend_from_slice(&name_table);
        output.extend_from_slice(&import_table);
        write_i32_at(
            &mut output,
            12,
            checked_i32(names.entries.len(), "Muitos nomes no UTX.")?,
        )?;
        write_i32_at(
            &mut output,
            16,
            checked_i32(header.len(), "UTX grande demais.")?,
        )?;
        write_i32_at(&mut output, 20, 0)?;
        write_i32_at(
            &mut output,
            24,
            checked_i32(export_offset, "UTX grande demais.")?,
        )?;
        write_i32_at(
            &mut output,
            28,
            checked_i32(self.imports.len(), "Muitos imports no UTX.")?,
        )?;
        write_i32_at(
            &mut output,
            32,
            checked_i32(import_offset, "UTX grande demais.")?,
        )?;
        Ok(output)
    }

    fn add_textures(
        &mut self,
        template: &Self,
        package_name: &str,
        textures: &[TextureImportRequest],
    ) -> TextureEngineResult<(Vec<u8>, Vec<TextureImportOutcome>)> {
        if textures.is_empty() {
            return Ok((self.data.clone(), Vec::new()));
        }
        validate_group_name(package_name)?;
        let mut names = self.names.clone();
        let mut name_indices = names.index_map();
        let mut imports = self.imports.clone();
        let mut exports = self.exports.clone();
        let mut seed_cache = HashMap::new();
        let (outer, group_data) = match self.group_outer_for_name(package_name)? {
            Some(outer) => (outer, Vec::new()),
            None => {
                let created =
                    self.create_group(package_name, &mut names, &mut imports, &mut exports)?;
                let group_index = object_index(created.0)?;
                exports[group_index].offset =
                    checked_i32(self.data.len(), "O pacote excede 2 GB.")?;
                if let Some(index) = names.index_of(package_name) {
                    name_indices.insert(package_name.to_ascii_lowercase(), index);
                }
                (created.0, created.1)
            }
        };
        let mut appended = group_data;
        let mut outcomes = Vec::with_capacity(textures.len());
        for texture in textures {
            let seed_kind = texture_seed_kind(texture);
            let source = if let Some(source) = seed_cache.get(&seed_kind) {
                *source
            } else {
                let requires_split9 = texture.split9.is_some();
                let requires_animation = texture.animation.is_some();
                let source = match self.template_texture_for_import(
                    package_name,
                    requires_split9,
                    requires_animation,
                ) {
                    Ok(index) => TextureSeedSource::Package(index),
                    Err(_) if self.is_compatible_template(template) => {
                        TextureSeedSource::Template(template.template_texture_for_import(
                            package_name,
                            requires_split9,
                            requires_animation,
                        )?)
                    }
                    Err(error) => return Err(error),
                };
                seed_cache.insert(seed_kind, source);
                source
            };
            let (seed_package, seed_index) = match source {
                TextureSeedSource::Package(index) => (&*self, index),
                TextureSeedSource::Template(index) => (template, index),
            };
            let seed = seed_package.export(seed_index)?.clone();
            let serialized =
                build_texture_export(seed_package.export_data(&seed)?, seed_package, texture)?;
            let export_offset = self
                .data
                .len()
                .checked_add(appended.len())
                .ok_or("Tamanho de pacote inválido.")?;
            let mut bytes = serialized.bytes;
            write_i32_at(
                &mut bytes,
                serialized.mip_width_offset,
                checked_i32(
                    export_offset
                        .checked_add(serialized.width_offset_value)
                        .ok_or("Offset de textura inválido.")?,
                    "O pacote excede o limite de tamanho.",
                )?,
            )?;
            let name_index = names.intern_indexed(&texture.name, &mut name_indices)?;
            let export_index = exports.len();
            exports.push(ExportEntry {
                class: seed.class,
                super_class: seed.super_class,
                package: outer,
                name_index,
                flags: seed.flags,
                size: checked_i32(bytes.len(), "A textura é grande demais.")?,
                offset: checked_i32(export_offset, "O pacote excede 2 GB.")?,
            });
            appended.extend_from_slice(&bytes);
            outcomes.push(TextureImportOutcome {
                export_index,
                added: true,
            });
        }
        self.rewrite_tables(names, imports, exports, appended)?;

        let mut edits = Vec::new();
        for (texture, outcome) in textures.iter().zip(&outcomes) {
            if let Some(edit) = self.import_edit(texture, outcome.export_index)? {
                edits.push((outcome.export_index, edit));
            }
        }
        self.apply_texture_edits(edits)?;
        Ok((self.data.clone(), outcomes))
    }

    fn find_texture_in_group(
        &self,
        group_name: &str,
        texture_name: &str,
    ) -> TextureEngineResult<Option<usize>> {
        for (index, export) in self.exports.iter().enumerate() {
            if self.is_texture(export)
                && texture_in_group(&self.inner_name(export)?, group_name)
                && texture_leaf(&self.inner_name(export)?).eq_ignore_ascii_case(texture_name)
            {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    fn texture_index_by_group(&self) -> TextureEngineResult<HashMap<String, usize>> {
        let mut textures = HashMap::new();
        for (index, export) in self.exports.iter().enumerate() {
            if !self.is_texture(export) {
                continue;
            }
            let name = self.inner_name(export)?;
            let group_name = texture_group(&name).unwrap_or("Pacote principal");
            textures
                .entry(texture_lookup_key(group_name, texture_leaf(&name)))
                .or_insert(index);
        }
        Ok(textures)
    }

    fn duplicate_texture(
        &mut self,
        source_export_index: usize,
        group_name: &str,
        texture_name: &str,
    ) -> TextureEngineResult<usize> {
        validate_group_name(group_name)?;
        validate_texture_name(texture_name)?;
        if self
            .find_texture_in_group(group_name, texture_name)?
            .is_some()
        {
            return Err(format!(
                "Já existe uma textura chamada \"{texture_name}\" no grupo \"{group_name}\"."
            ));
        }

        let source = self.export(source_export_index)?.clone();
        if !self.is_texture(&source) {
            return Err("A entrada selecionada não é uma textura Engine.Texture.".into());
        }
        let mip = self.mip0_location(source_export_index)?;
        let mut bytes = self.export_data(&source)?.to_vec();
        let mut names = self.names.clone();
        let mut imports = self.imports.clone();
        let mut exports = self.exports.clone();
        let (outer, group_data) = match self.group_outer_for_name(group_name)? {
            Some(outer) => (outer, Vec::new()),
            None => {
                let created =
                    self.create_group(group_name, &mut names, &mut imports, &mut exports)?;
                let group_index = object_index(created.0)?;
                exports[group_index].offset =
                    checked_i32(self.data.len(), "O pacote excede 2 GB.")?;
                (created.0, created.1)
            }
        };
        let export_offset = self
            .data
            .len()
            .checked_add(group_data.len())
            .ok_or("Tamanho de pacote inválido.")?;
        let width_offset = export_offset
            .checked_add(mip.pixel_offset)
            .and_then(|offset| offset.checked_add(mip.size))
            .ok_or("Offset de textura inválido.")?;
        write_i32_at(
            &mut bytes,
            mip.width_offset_position,
            checked_i32(width_offset, "O pacote excede 2 GB.")?,
        )?;

        let export_index = exports.len();
        exports.push(ExportEntry {
            class: source.class,
            super_class: source.super_class,
            package: outer,
            name_index: names.intern(texture_name)?,
            flags: source.flags,
            size: checked_i32(bytes.len(), "A textura é grande demais.")?,
            offset: checked_i32(export_offset, "O pacote excede 2 GB.")?,
        });
        let mut appended = group_data;
        appended.extend_from_slice(&bytes);
        self.rewrite_tables(names, imports, exports, appended)?;
        Ok(export_index)
    }

    fn rename_texture(
        &mut self,
        export_index: usize,
        texture_name: &str,
    ) -> TextureEngineResult<()> {
        validate_texture_name(texture_name)?;
        let source = self.export(export_index)?.clone();
        if !self.is_texture(&source) {
            return Err("A entrada selecionada não é uma textura Engine.Texture.".into());
        }
        let source_name = self.inner_name(&source)?;
        let group_name = texture_group(&source_name).unwrap_or("Pacote principal");
        if let Some(existing_index) = self.find_texture_in_group(group_name, texture_name)? {
            if existing_index != export_index {
                return Err(format!(
                    "Já existe uma textura chamada \"{texture_name}\" no grupo \"{group_name}\"."
                ));
            }
        }

        let mut names = self.names.clone();
        let mut exports = self.exports.clone();
        let export = exports
            .get_mut(export_index)
            .ok_or("Exportação de textura inválida.")?;
        export.name_index = names.intern(texture_name)?;
        self.rewrite_tables_from_data(self.data.clone(), names, self.imports.clone(), exports)
    }

    fn replace_pixels(
        &mut self,
        export_index: usize,
        texture: &TextureImportRequest,
    ) -> TextureEngineResult<()> {
        let export = self.export(export_index)?.clone();
        if !self.is_texture(&export) {
            return Err("A entrada selecionada não é uma textura Engine.Texture.".into());
        }
        let raw = self.export_data(&export)?;
        let layout = texture_layout(raw, self)?;
        let current_format = *raw
            .get(layout.format.offset)
            .ok_or("A propriedade Format está truncada.")?;
        if current_format != texture.format {
            return Err("O formato da textura substituta é incompatível.".into());
        }
        let mip = self.mip0_location(export_index)?;
        if mip.width != texture.width || mip.height != texture.height {
            return Err(format!(
                "Tamanho incompatível: esperado {}×{}, recebido {}×{}.",
                mip.width, mip.height, texture.width, texture.height
            ));
        }
        if texture.pixels.len() != mip.size {
            return Err(format!(
                "Tamanho de pixels incompatível: esperado {} bytes.",
                mip.size
            ));
        }
        let export_offset = read_offset(export.offset, "dados de textura")?;
        let pixel_offset = export_offset
            .checked_add(mip.pixel_offset)
            .ok_or("Offset de textura inválido.")?;
        self.data
            .get_mut(pixel_offset..pixel_offset + mip.size)
            .ok_or("Os pixels estão fora do pacote.")?
            .copy_from_slice(&texture.pixels);
        write_i32_at(
            &mut self.data,
            export_offset
                .checked_add(mip.width_offset_position)
                .ok_or("Offset de textura inválido.")?,
            checked_i32(pixel_offset + mip.size, "O pacote excede 2 GB.")?,
        )
    }

    /// Patches only values that are already serialized in the texture property
    /// stream. This is the safe path for existing game packages: export sizes,
    /// offsets, and package tables remain byte-for-byte unchanged.
    fn apply_texture_edits_in_place(
        &mut self,
        edits: Vec<(usize, TextureEditorEdit)>,
    ) -> TextureEngineResult<()> {
        for (export_index, edit) in edits {
            if !edit.has_changes() {
                continue;
            }
            let export = self.export(export_index)?.clone();
            let export_offset = read_offset(export.offset, "dados de textura")?;
            let locations = {
                let raw = self.export_data(&export)?;
                self.existing_texture_property_locations(raw)?
            };

            if let Some(value) = edit.alpha {
                self.patch_existing_bool(export_offset, &locations, "bAlphaTexture", value)?;
            }
            if let Some(value) = edit.masked {
                self.patch_existing_bool(export_offset, &locations, "bMasked", value)?;
            }
            if let Some(clamp) = edit.clamp {
                if let Some(value) = clamp.u_clamp {
                    self.patch_existing_integer(export_offset, &locations, "UClamp", value)?;
                }
                if let Some(value) = clamp.v_clamp {
                    self.patch_existing_integer(export_offset, &locations, "VClamp", value)?;
                }
                if let Some(value) = clamp.u_clamp_mode {
                    self.patch_existing_integer(export_offset, &locations, "UClampMode", value)?;
                }
                if let Some(value) = clamp.v_clamp_mode {
                    self.patch_existing_integer(export_offset, &locations, "VClampMode", value)?;
                }
            }
            if let Some(split9) = edit.split9 {
                self.patch_existing_bool(
                    export_offset,
                    &locations,
                    "bSplit9Texture",
                    split9.enabled,
                )?;
                if split9.enabled {
                    for (name, value) in [
                        ("Split9X1", split9.x1),
                        ("Split9X2", split9.x2),
                        ("Split9X3", split9.x3),
                        ("Split9Y1", split9.y1),
                        ("Split9Y2", split9.y2),
                        ("Split9Y3", split9.y3),
                    ] {
                        self.patch_existing_integer(export_offset, &locations, name, value)?;
                    }
                }
            }
            if let Some(animation) = edit.animation {
                if let Some(value) = animation.anim_next {
                    self.patch_existing_object(export_offset, &locations, "AnimNext", value)?;
                }
                if let Some(value) = animation.max_frame_rate {
                    validate_finite(value, "MaxFrameRate")?;
                    self.patch_existing_float(export_offset, &locations, "MaxFrameRate", value)?;
                }
                if let Some(value) = animation.min_frame_rate {
                    validate_finite(value, "MinFrameRate")?;
                    self.patch_existing_float(export_offset, &locations, "MinFrameRate", value)?;
                }
                if let Some(value) = animation.one_time_anim_loop {
                    self.patch_existing_bool(export_offset, &locations, "OneTimeAnimLoop", value)?;
                }
                if let Some(value) = animation.prime_count {
                    self.patch_existing_integer(export_offset, &locations, "PrimeCount", value)?;
                }
                if let Some(value) = animation.total_frame_num {
                    self.patch_existing_integer(export_offset, &locations, "TotalFrameNum", value)?;
                }
            }
        }
        Ok(())
    }

    fn existing_texture_property_locations(
        &self,
        raw: &[u8],
    ) -> TextureEngineResult<Vec<ExistingPropertyLocation>> {
        let mut reader = Reader::new(raw);
        let mut properties = Vec::new();
        loop {
            let name_index = reader.read_compact()?;
            let name = self.names.name(name_index)?.to_owned();
            if name.eq_ignore_ascii_case("None") {
                return Ok(properties);
            }

            let info_offset = reader.position();
            let info = reader.read_u8()?;
            let property_type = info & 0x0f;
            let size_type = (info >> 4) & 0x07;
            let is_array = info & 0x80 != 0;
            if property_type == 10 {
                reader.read_compact()?;
            }
            let size = property_size(&mut reader, size_type)?;
            if is_array && property_type != TYPE_BOOL {
                reader.read_compact()?;
            }
            let value_offset = reader.position();
            reader.skip(size)?;
            properties.push(ExistingPropertyLocation {
                name,
                property_type,
                info_offset,
                value_offset,
                size,
            });
        }
    }

    fn existing_property<'a>(
        locations: &'a [ExistingPropertyLocation],
        name: &str,
    ) -> TextureEngineResult<&'a ExistingPropertyLocation> {
        locations
            .iter()
            .find(|property| property.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                format!(
                    "A textura não possui a propriedade {name}; esta alteração exige uma reserialização que não é segura para este pacote."
                )
            })
    }

    fn patch_existing_bool(
        &mut self,
        export_offset: usize,
        locations: &[ExistingPropertyLocation],
        name: &str,
        value: bool,
    ) -> TextureEngineResult<()> {
        let property = Self::existing_property(locations, name)?;
        if property.property_type != TYPE_BOOL {
            return Err(format!("A propriedade {name} não é um bool UE2."));
        }
        let position = export_offset
            .checked_add(property.info_offset)
            .ok_or("Offset de propriedade inválido.")?;
        let info = self
            .data
            .get_mut(position)
            .ok_or("Dados de propriedade truncados.")?;
        *info = (*info & !0x80) | if value { 0x80 } else { 0 };
        Ok(())
    }

    fn patch_existing_integer(
        &mut self,
        export_offset: usize,
        locations: &[ExistingPropertyLocation],
        name: &str,
        value: i32,
    ) -> TextureEngineResult<()> {
        let property = Self::existing_property(locations, name)?;
        let position = export_offset
            .checked_add(property.value_offset)
            .ok_or("Offset de propriedade inválido.")?;
        let target = self
            .data
            .get_mut(
                position
                    ..position
                        .checked_add(property.size)
                        .ok_or("Offset de propriedade inválido.")?,
            )
            .ok_or("Dados de propriedade truncados.")?;
        match (property.property_type, property.size) {
            (TYPE_BYTE, 1) => {
                target[0] =
                    u8::try_from(value).map_err(|_| format!("{name} deve estar entre 0 e 255."))?;
            }
            (TYPE_INT, 4) => target.copy_from_slice(&value.to_le_bytes()),
            _ => {
                return Err(format!(
                    "A propriedade {name} não possui um formato inteiro UE2 suportado."
                ))
            }
        }
        Ok(())
    }

    fn patch_existing_float(
        &mut self,
        export_offset: usize,
        locations: &[ExistingPropertyLocation],
        name: &str,
        value: f32,
    ) -> TextureEngineResult<()> {
        let property = Self::existing_property(locations, name)?;
        if property.property_type != TYPE_FLOAT || property.size != 4 {
            return Err(format!("A propriedade {name} não é um decimal UE2."));
        }
        let position = export_offset
            .checked_add(property.value_offset)
            .ok_or("Offset de propriedade inválido.")?;
        self.data
            .get_mut(position..position + 4)
            .ok_or("Dados de propriedade truncados.")?
            .copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn patch_existing_object(
        &mut self,
        export_offset: usize,
        locations: &[ExistingPropertyLocation],
        name: &str,
        value: i32,
    ) -> TextureEngineResult<()> {
        let property = Self::existing_property(locations, name)?;
        if property.property_type != TYPE_OBJECT {
            return Err(format!("A propriedade {name} não é uma referência UE2."));
        }
        let mut encoded = Vec::new();
        write_compact(&mut encoded, value);
        if encoded.len() != property.size {
            return Err(format!(
                "A referência {name} exige alterar o tamanho serializado; esta alteração não é segura para este pacote."
            ));
        }
        let position = export_offset
            .checked_add(property.value_offset)
            .ok_or("Offset de propriedade inválido.")?;
        self.data
            .get_mut(position..position + encoded.len())
            .ok_or("Dados de propriedade truncados.")?
            .copy_from_slice(&encoded);
        Ok(())
    }

    fn import_edit(
        &self,
        texture: &TextureImportRequest,
        export_index: usize,
    ) -> TextureEngineResult<Option<TextureEditorEdit>> {
        let animation = match texture.animation.as_ref() {
            Some(animation) => Some(TextureAnimationEdit {
                anim_next: animation
                    .anim_next
                    .as_deref()
                    .map(|path| self.resolve_texture_reference(export_index, path))
                    .transpose()?,
                max_frame_rate: animation.max_frame_rate,
                min_frame_rate: animation.min_frame_rate,
                one_time_anim_loop: animation.one_time_anim_loop,
                prime_count: animation.prime_count,
                total_frame_num: animation.total_frame_num,
            }),
            None => None,
        };
        let edit = TextureEditorEdit {
            alpha: texture.alpha,
            masked: texture.masked,
            clamp: texture.clamp,
            split9: texture.split9,
            animation,
        };
        Ok(edit.has_changes().then_some(edit))
    }

    fn apply_texture_edits(
        &mut self,
        edits: Vec<(usize, TextureEditorEdit)>,
    ) -> TextureEngineResult<()> {
        if edits.is_empty() {
            return Ok(());
        }
        let mut exports = self.exports.clone();
        let mut rewritten = Vec::with_capacity(edits.len());
        for (export_index, edit) in edits {
            let export = self.export(export_index)?;
            let raw = self.export_data(export)?.to_vec();
            let layout = texture_layout(&raw, self)?;
            let mut properties = TexturePropertyStream::parse(&raw, &self.names)?;
            properties.apply_editor_edit(&mut self.names, &edit)?;
            let mut bytes = properties.serialize(&self.names)?;
            bytes.extend_from_slice(
                raw.get(properties.tail_offset()..)
                    .ok_or("Dados de textura truncados.")?,
            );
            let mip_count_offset = bytes
                .len()
                .checked_sub(
                    raw.len()
                        .checked_sub(layout.mip_count_offset)
                        .ok_or("Offset de mip inválido.")?,
                )
                .ok_or("Offset de mip inválido.")?;
            let mip = self.mip0_location(export_index)?;
            rewritten.push((export_index, bytes, mip_count_offset, mip));
        }
        let mut output = self.data.clone();
        for (export_index, mut bytes, mip_count_offset, mip) in rewritten {
            let pointer_offset = mip_count_offset
                .checked_add(
                    mip.width_offset_position
                        .checked_sub(mip.mip_count_offset)
                        .ok_or("Offset de mip inválido.")?,
                )
                .ok_or("Offset de mip inválido.")?;
            let pixel_offset = mip_count_offset
                .checked_add(
                    mip.pixel_offset
                        .checked_sub(mip.mip_count_offset)
                        .ok_or("Offset de mip inválido.")?,
                )
                .ok_or("Offset de mip inválido.")?;
            let width_offset = output
                .len()
                .checked_add(pixel_offset)
                .and_then(|offset| offset.checked_add(mip.size))
                .ok_or("Offset de mip inválido.")?;
            write_i32_at(
                &mut bytes,
                pointer_offset,
                checked_i32(width_offset, "O pacote excede 2 GB.")?,
            )?;
            let export = exports
                .get_mut(export_index)
                .ok_or("Exportação de textura inválida.")?;
            export.offset = checked_i32(output.len(), "O pacote excede 2 GB.")?;
            export.size = checked_i32(bytes.len(), "A textura é grande demais.")?;
            output.extend_from_slice(&bytes);
        }
        self.rewrite_tables_from_data(output, self.names.clone(), self.imports.clone(), exports)?;
        Ok(())
    }

    fn rewrite_tables(
        &mut self,
        names: NameTable,
        imports: Vec<ImportEntry>,
        exports: Vec<ExportEntry>,
        appended: Vec<u8>,
    ) -> TextureEngineResult<()> {
        let mut data = self.data.clone();
        data.extend_from_slice(&appended);
        self.rewrite_tables_from_data(data, names, imports, exports)
    }

    fn rewrite_tables_from_data(
        &mut self,
        mut data: Vec<u8>,
        names: NameTable,
        imports: Vec<ImportEntry>,
        exports: Vec<ExportEntry>,
    ) -> TextureEngineResult<()> {
        let name_table = names.serialize()?;
        let import_table = serialize_import_table(&imports);
        let export_table = serialize_export_table(&exports);
        let name_offset = data.len();
        let import_offset = name_offset
            .checked_add(name_table.len())
            .ok_or("Tamanho de pacote inválido.")?;
        let export_offset = import_offset
            .checked_add(import_table.len())
            .ok_or("Tamanho de pacote inválido.")?;
        data.extend_from_slice(&name_table);
        data.extend_from_slice(&import_table);
        data.extend_from_slice(&export_table);
        write_i32_at(
            &mut data,
            12,
            checked_i32(names.entries.len(), "Muitos nomes no pacote.")?,
        )?;
        write_i32_at(
            &mut data,
            16,
            checked_i32(name_offset, "Pacote grande demais.")?,
        )?;
        write_i32_at(
            &mut data,
            20,
            checked_i32(exports.len(), "Muitas exportações no pacote.")?,
        )?;
        write_i32_at(
            &mut data,
            24,
            checked_i32(export_offset, "Pacote grande demais.")?,
        )?;
        write_i32_at(
            &mut data,
            28,
            checked_i32(imports.len(), "Muitas importações no pacote.")?,
        )?;
        write_i32_at(
            &mut data,
            32,
            checked_i32(import_offset, "Pacote grande demais.")?,
        )?;
        self.data = data;
        self.names = names;
        self.imports = imports;
        self.exports = exports;
        Ok(())
    }

    fn group_outer_for_name(&self, group_name: &str) -> TextureEngineResult<Option<i32>> {
        if group_name.eq_ignore_ascii_case("Pacote principal") {
            return Ok(Some(0));
        }
        for (index, export) in self.exports.iter().enumerate() {
            if self.is_texture(export)
                && self
                    .inner_name(export)
                    .is_ok_and(|name| texture_in_group(&name, group_name))
            {
                return Ok(Some(self.export(index)?.package));
            }
        }
        for (index, export) in self.exports.iter().enumerate() {
            if self
                .class_name(export)
                .is_ok_and(|name| name.eq_ignore_ascii_case("Core.Package"))
                && self
                    .inner_name(export)
                    .is_ok_and(|name| name.eq_ignore_ascii_case(group_name))
            {
                return export_reference(index).map(Some);
            }
        }
        Ok(None)
    }

    fn create_group(
        &self,
        group_name: &str,
        names: &mut NameTable,
        imports: &mut Vec<ImportEntry>,
        exports: &mut Vec<ExportEntry>,
    ) -> TextureEngineResult<(i32, Vec<u8>)> {
        let name_index = names.intern(group_name)?;
        let template = self.exports.iter().find(|export| {
            self.class_name(export)
                .is_ok_and(|name| name.eq_ignore_ascii_case("Core.Package"))
        });
        let (class, super_class, flags, bytes) = match template {
            Some(template) => (
                template.class,
                template.super_class,
                template.flags,
                self.export_data(template)?.to_vec(),
            ),
            None => (
                ensure_core_package_import(names, imports)?,
                0,
                0x0007_0004,
                vec![1],
            ),
        };
        let reference = export_reference(exports.len())?;
        exports.push(ExportEntry {
            class,
            super_class,
            package: 0,
            name_index,
            flags,
            size: checked_i32(bytes.len(), "O grupo é grande demais.")?,
            offset: 0,
        });
        Ok((reference, bytes))
    }

    fn template_texture_for_import(
        &self,
        package_name: &str,
        requires_split9: bool,
        requires_animation: bool,
    ) -> TextureEngineResult<usize> {
        let mut fallback = None;
        for (index, export) in self.exports.iter().enumerate() {
            if !self.is_texture(export) {
                continue;
            }
            let raw = self.export_data(export)?;
            let properties = TexturePropertyStream::parse(raw, &self.names)?;
            let has_split9 = properties.editor_state().split9.unwrap_or(false);
            let has_animation = texture_layout(raw, self)?
                .anim_next
                .is_some_and(|reference| reference != 0);
            // Split9 + animation no longer needs a fourth, combined seed.
            // An animation seed already provides the delicate UE2 animation
            // layout; the property engine adds the Split9 fields afterwards.
            let matches = if requires_split9 && requires_animation {
                has_animation
            } else {
                has_split9 == requires_split9 && has_animation == requires_animation
            };
            if !matches {
                continue;
            }
            let name = self.inner_name(export)?;
            if texture_in_group(&name, package_name) {
                return Ok(index);
            }
            fallback.get_or_insert(index);
        }
        if let Some(index) = fallback {
            return Ok(index);
        }
        let kind = match (requires_split9, requires_animation) {
            (true, true) => "uma textura Split9 animada",
            (true, false) => "uma textura Split9",
            (false, true) => "uma textura animada",
            (false, false) => "uma textura comum",
        };
        Err(format!(
            "O UTX não possui {kind} para usar como modelo estrutural."
        ))
    }

    fn is_compatible_template(&self, template: &Self) -> bool {
        if self.version != template.version || self.licensee != template.licensee {
            return false;
        }
        if self.names.entries.len() < template.names.entries.len()
            || self.imports.len() < template.imports.len()
        {
            return false;
        }
        let names_match = template
            .names
            .entries
            .iter()
            .enumerate()
            .all(|(index, expected)| {
                expected.name.eq_ignore_ascii_case("UnrealTlp")
                    || self.names.entries.get(index).is_some_and(|actual| {
                        actual.name.eq_ignore_ascii_case(&expected.name)
                            && actual.flags == expected.flags
                    })
            });
        let imports_match = template
            .imports
            .iter()
            .zip(&self.imports)
            .all(|(expected, actual)| {
                expected.class_package == actual.class_package
                    && expected.class_name == actual.class_name
                    && expected.package == actual.package
                    && expected.name_index == actual.name_index
            });
        names_match && imports_match
    }

    fn resolve_texture_reference(
        &self,
        current_export: usize,
        requested_path: &str,
    ) -> TextureEngineResult<i32> {
        let requested = requested_path
            .trim()
            .strip_prefix("Texture'")
            .unwrap_or(requested_path.trim())
            .trim_end_matches('\'')
            .trim();
        if requested.is_empty() {
            return Ok(0);
        }
        let current = self.inner_name(self.export(current_export)?)?;
        let requested_leaf = texture_leaf(requested);
        let mut same_group = None;
        let mut leaf_matches = Vec::new();
        for (index, export) in self.exports.iter().enumerate() {
            if !self.is_texture(export) {
                continue;
            }
            let candidate = self.inner_name(export)?;
            if candidate.eq_ignore_ascii_case(requested)
                || requested.strip_suffix(&format!(".{candidate}")).is_some()
            {
                return export_reference(index);
            }
            if texture_leaf(&candidate).eq_ignore_ascii_case(requested_leaf) {
                if texture_group(&candidate) == texture_group(&current) {
                    same_group = Some(index);
                }
                leaf_matches.push(index);
            }
        }
        if let Some(index) = same_group {
            return export_reference(index);
        }
        if leaf_matches.len() == 1 {
            return export_reference(leaf_matches[0]);
        }
        Err(format!(
            "A textura indicada em AnimNext não foi encontrada no pacote: {requested}."
        ))
    }

    fn is_texture(&self, export: &ExportEntry) -> bool {
        self.class_name(export)
            .is_ok_and(|name| name.eq_ignore_ascii_case("Engine.Texture"))
    }

    fn export(&self, index: usize) -> TextureEngineResult<&ExportEntry> {
        self.exports
            .get(index)
            .ok_or_else(|| "A textura selecionada não existe mais no pacote.".into())
    }

    fn export_data(&self, export: &ExportEntry) -> TextureEngineResult<&[u8]> {
        let start = read_offset(export.offset, "dados de exportação")?;
        let size = read_count(export.size, "tamanho de exportação")?;
        self.data
            .get(
                start
                    ..start
                        .checked_add(size)
                        .ok_or("Offset de exportação inválido.")?,
            )
            .ok_or_else(|| "Os dados de exportação estão fora do pacote.".into())
    }

    fn inner_name(&self, export: &ExportEntry) -> TextureEngineResult<String> {
        self.inner_name_depth(export, 0)
    }

    fn inner_name_depth(&self, export: &ExportEntry, depth: usize) -> TextureEngineResult<String> {
        if depth > 128 {
            return Err("A hierarquia de objetos contém uma referência cíclica.".into());
        }
        let name = self.names.name(export.name_index)?.to_owned();
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

    fn class_name(&self, export: &ExportEntry) -> TextureEngineResult<String> {
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

    fn import_path(&self, index: usize, depth: usize) -> TextureEngineResult<String> {
        if depth > 128 {
            return Err("A hierarquia de imports contém uma referência cíclica.".into());
        }
        let import = self
            .imports
            .get(index)
            .ok_or("Referência de import inválida.")?;
        let name = self.names.name(import.name_index)?.to_owned();
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

    fn mip0_location(&self, export_index: usize) -> TextureEngineResult<MipLocation> {
        let raw = self.export_data(self.export(export_index)?)?;
        let layout = texture_layout(raw, self)?;
        let mut reader = Reader::new(raw);
        reader.seek(layout.mip_count_offset)?;
        if reader.read_u8()? == 0 {
            return Err("A textura não contém mip maps.".into());
        }
        let width_offset_position = reader.position();
        reader.skip(4)?;
        let size = read_count(reader.read_compact()?, "mip")?;
        let pixel_offset = reader.position();
        reader.skip(size)?;
        let width = reader.read_i32()?;
        let height = reader.read_i32()?;
        Ok(MipLocation {
            mip_count_offset: layout.mip_count_offset,
            pixel_offset,
            width_offset_position,
            size,
            width,
            height,
        })
    }
}

fn build_texture_export(
    template_raw: &[u8],
    package: &TexturePackage,
    texture: &TextureImportRequest,
) -> TextureEngineResult<SerializedTexture> {
    let layout = texture_layout(template_raw, package)?;
    if !texture.width.is_positive() || !texture.height.is_positive() {
        return Err("Dimensões de textura inválidas.".into());
    }
    let mut output = template_raw
        .get(..layout.mip_count_offset)
        .ok_or("Modelo de textura truncado.")?
        .to_vec();
    patch_property(&mut output, layout.format, i32::from(texture.format))?;
    if let Some(patch) = layout.width {
        patch_property(&mut output, patch, texture.width)?;
    }
    if let Some(patch) = layout.height {
        patch_property(&mut output, patch, texture.height)?;
    }
    if let Some(patch) = layout.u_bits {
        patch_property(&mut output, patch, dimension_bits(texture.width)?)?;
    }
    if let Some(patch) = layout.v_bits {
        patch_property(&mut output, patch, dimension_bits(texture.height)?)?;
    }
    output.push(1);
    output.extend_from_slice(
        template_raw
            .get(layout.mip_count_offset + 1..layout.mip_payload_offset)
            .ok_or("Modelo de textura truncado.")?,
    );
    write_compact(
        &mut output,
        checked_i32(texture.pixels.len(), "A textura é grande demais.")?,
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

fn texture_layout(raw: &[u8], package: &TexturePackage) -> TextureEngineResult<TextureLayout> {
    let mut reader = Reader::new(raw);
    let mut format = None;
    let mut width = None;
    let mut height = None;
    let mut u_bits = None;
    let mut v_bits = None;
    let mut anim_next = None;
    loop {
        let name = package
            .names
            .name(reader.read_compact()?)?
            .to_ascii_lowercase();
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
        if is_array && property_type != TYPE_BOOL {
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
            "animnext" if size > 0 => {
                let bytes = raw
                    .get(patch.offset..patch.offset + patch.size)
                    .ok_or("Dados de animação truncados.")?;
                anim_next = Some(Reader::new(bytes).read_compact()?);
            }
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
        format: format.ok_or("O modelo não possui a propriedade Format.")?,
        width,
        height,
        u_bits,
        v_bits,
        anim_next,
        mip_count_offset,
        mip_width_offset,
        mip_payload_offset: reader.position(),
    })
}

fn patch_property(output: &mut [u8], patch: PropertyPatch, value: i32) -> TextureEngineResult<()> {
    let target = output
        .get_mut(patch.offset..patch.offset + patch.size)
        .ok_or("Modelo de textura truncado.")?;
    match patch.size {
        1 => target[0] = u8::try_from(value).map_err(|_| "Valor de propriedade inválido.")?,
        2 => target.copy_from_slice(
            &u16::try_from(value)
                .map_err(|_| "Valor de propriedade inválido.")?
                .to_le_bytes(),
        ),
        4 => target.copy_from_slice(&value.to_le_bytes()),
        _ => return Err("Tamanho de propriedade não suportado pelo motor.".into()),
    }
    Ok(())
}

fn dimension_bits(value: i32) -> TextureEngineResult<i32> {
    let value = u32::try_from(value).map_err(|_| "Dimensão inválida.")?;
    if !value.is_power_of_two() {
        return Err("A textura deve ter dimensões em potências de dois.".into());
    }
    Ok(value.ilog2() as i32)
}

fn skip_unreal_extra(
    reader: &mut Reader<'_>,
    version: i32,
    licensee: i32,
) -> TextureEngineResult<()> {
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
    // Later Lineage II licensee builds append a variable-length metadata block
    // after the fixed texture header. The engine reader must consume it before
    // locating the mip data; otherwise it interprets metadata as mip fields
    // and can write invalid pixel offsets back into the package.
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

fn validate_group_name(group_name: &str) -> TextureEngineResult<()> {
    if group_name.eq_ignore_ascii_case("Pacote principal") {
        return Ok(());
    }
    if !group_name.is_ascii() || group_name.contains('.') || group_name.is_empty() {
        return Err("O grupo deve usar ASCII, não pode conter ponto e não pode ser vazio.".into());
    }
    Ok(())
}

fn validate_texture_name(texture_name: &str) -> TextureEngineResult<()> {
    if !texture_name.is_ascii() || texture_name.contains('.') || texture_name.is_empty() {
        return Err(
            "O nome da textura deve usar ASCII, não pode conter ponto e não pode ser vazio.".into(),
        );
    }
    Ok(())
}

fn texture_group(name: &str) -> Option<&str> {
    name.split_once('.').map(|(group, _)| group)
}

fn texture_lookup_key(group_name: &str, texture_name: &str) -> String {
    format!(
        "{}\0{}",
        group_name.to_ascii_lowercase(),
        texture_name.to_ascii_lowercase()
    )
}

fn texture_seed_kind(texture: &TextureImportRequest) -> TextureSeedKind {
    if texture.animation.is_some() {
        TextureSeedKind::Animation
    } else if texture.split9.is_some() {
        TextureSeedKind::Split9
    } else {
        TextureSeedKind::Common
    }
}

fn texture_leaf(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn texture_in_group(name: &str, group_name: &str) -> bool {
    if group_name.eq_ignore_ascii_case("Pacote principal") {
        !name.contains('.')
    } else {
        texture_group(name).is_some_and(|group| group.eq_ignore_ascii_case(group_name))
    }
}

fn ensure_core_package_import(
    names: &mut NameTable,
    imports: &mut Vec<ImportEntry>,
) -> TextureEngineResult<i32> {
    let core = names.intern("Core")?;
    let package = names.intern("Package")?;
    let class = names.intern("Class")?;
    let core_reference = if let Some(index) = imports
        .iter()
        .position(|entry| entry.package == 0 && entry.name_index == core)
    {
        import_reference(index)?
    } else {
        imports.push(ImportEntry {
            class_package: core,
            class_name: package,
            package: 0,
            name_index: core,
        });
        import_reference(imports.len() - 1)?
    };
    if let Some(index) = imports
        .iter()
        .position(|entry| entry.package == core_reference && entry.name_index == package)
    {
        return import_reference(index);
    }
    imports.push(ImportEntry {
        class_package: core,
        class_name: class,
        package: core_reference,
        name_index: package,
    });
    import_reference(imports.len() - 1)
}

fn serialize_import_table(entries: &[ImportEntry]) -> Vec<u8> {
    let mut output = Vec::new();
    for entry in entries {
        write_compact(&mut output, entry.class_package);
        write_compact(&mut output, entry.class_name);
        output.extend_from_slice(&entry.package.to_le_bytes());
        write_compact(&mut output, entry.name_index);
    }
    output
}

fn serialize_export_table(entries: &[ExportEntry]) -> Vec<u8> {
    let mut output = Vec::new();
    for entry in entries {
        write_compact(&mut output, entry.class);
        write_compact(&mut output, entry.super_class);
        output.extend_from_slice(&entry.package.to_le_bytes());
        write_compact(&mut output, entry.name_index);
        output.extend_from_slice(&entry.flags.to_le_bytes());
        write_compact(&mut output, entry.size);
        if entry.size > 0 {
            write_compact(&mut output, entry.offset);
        }
    }
    output
}

fn read_export_table(
    data: &[u8],
    offset: usize,
    count: usize,
    name_count: usize,
) -> TextureEngineResult<(Vec<ExportEntry>, usize)> {
    let mut reader = Reader::new(data);
    reader.seek(offset)?;
    let mut exports = Vec::with_capacity(count);
    for _ in 0..count {
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
        let offset = if size > 0 { reader.read_compact()? } else { 0 };
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

fn read_count(value: i32, label: &str) -> TextureEngineResult<usize> {
    usize::try_from(value).map_err(|_| format!("Contagem de {label} inválida."))
}

fn read_offset(value: i32, label: &str) -> TextureEngineResult<usize> {
    usize::try_from(value).map_err(|_| format!("Offset de {label} inválido."))
}

fn checked_i32(value: usize, message: &str) -> TextureEngineResult<i32> {
    i32::try_from(value).map_err(|_| message.to_string())
}

fn object_index(reference: i32) -> TextureEngineResult<usize> {
    if reference == 0 {
        return Err("Referência de objeto nula inválida.".into());
    }
    usize::try_from(reference.unsigned_abs() - 1).map_err(|_| "Referência inválida.".into())
}

fn export_reference(index: usize) -> TextureEngineResult<i32> {
    checked_i32(
        index
            .checked_add(1)
            .ok_or("Referência de export inválida.")?,
        "Muitos exports.",
    )
}

fn import_reference(index: usize) -> TextureEngineResult<i32> {
    export_reference(index).map(|reference| -reference)
}

fn write_i32_at(data: &mut [u8], offset: usize, value: i32) -> TextureEngineResult<()> {
    data.get_mut(offset..offset.checked_add(4).ok_or("Offset inválido.")?)
        .ok_or("Cabeçalho do pacote truncado.")?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> NameTable {
        NameTable::new(
            [
                "None",
                "bAlphaTexture",
                "bMasked",
                "bSplit9Texture",
                "Split9X1",
                "Split9X2",
                "Split9X3",
                "Split9Y1",
                "Split9Y2",
                "Split9Y3",
                "AnimNext",
                "MaxFrameRate",
                "PrimeCount",
            ]
            .into_iter()
            .map(|name| NameEntry {
                name: name.into(),
                flags: 0x0007_0000,
            })
            .collect(),
        )
    }

    #[test]
    fn preserves_unknown_properties_when_editing_alpha() {
        let mut names = names();
        let mut raw = Vec::new();
        write_compact(&mut raw, 1);
        raw.extend_from_slice(&[0x53, 0]);
        write_compact(&mut raw, 12);
        raw.extend_from_slice(&[TYPE_BYTE, 77]);
        write_compact(&mut raw, 0);
        raw.extend_from_slice(&[99, 100]);

        let mut stream = TexturePropertyStream::parse(&raw, &names).unwrap();
        stream
            .apply_editor_edit(
                &mut names,
                &TextureEditorEdit {
                    alpha: Some(true),
                    masked: None,
                    clamp: None,
                    split9: None,
                    animation: None,
                },
            )
            .unwrap();
        let serialized = stream.serialize(&names).unwrap();
        assert_eq!(&serialized[..3], &[1, 0xd3, 0]);
        assert!(serialized
            .windows(3)
            .any(|bytes| bytes == [12, TYPE_BYTE, 77]));
        assert_eq!(stream.trailing_offset(), 6);
    }

    #[test]
    fn editor_edit_creates_split9_and_animation_properties() {
        let mut names = names();
        let mut raw = Vec::new();
        write_compact(&mut raw, 0);
        let mut stream = TexturePropertyStream::parse(&raw, &names).unwrap();
        stream
            .apply_editor_edit(
                &mut names,
                &TextureEditorEdit {
                    alpha: Some(false),
                    masked: Some(true),
                    clamp: Some(TextureClampEdit {
                        u_clamp: Some(10),
                        v_clamp: Some(11),
                        u_clamp_mode: Some(2),
                        v_clamp_mode: Some(3),
                    }),
                    split9: Some(Split9Edit {
                        enabled: true,
                        x1: 1,
                        x2: 2,
                        x3: 3,
                        y1: 4,
                        y2: 5,
                        y3: 6,
                    }),
                    animation: Some(TextureAnimationEdit {
                        anim_next: Some(129),
                        max_frame_rate: Some(24.0),
                        min_frame_rate: None,
                        one_time_anim_loop: None,
                        prime_count: Some(2),
                        total_frame_num: None,
                    }),
                },
            )
            .unwrap();
        let serialized = stream.serialize(&names).unwrap();
        let parsed = TexturePropertyStream::parse(&serialized, &names).unwrap();
        let state = parsed.editor_state();
        assert_eq!(state.alpha, Some(false));
        assert_eq!(state.masked, Some(true));
        assert_eq!(state.u_clamp, Some(10));
        assert_eq!(state.v_clamp, Some(11));
        assert_eq!(state.u_clamp_mode, Some(2));
        assert_eq!(state.v_clamp_mode, Some(3));
        assert_eq!(state.split9, Some(true));
        assert_eq!((state.split9_x1, state.split9_y3), (1, 6));
        assert_eq!(state.animation.anim_next, Some(129));
        assert_eq!(state.animation.max_frame_rate, Some(24.0));
        assert_eq!(state.animation.prime_count, Some(2));
    }

    #[test]
    fn compact_indices_round_trip_at_all_supported_lengths() {
        for value in [0, 63, 64, 8_191, 8_192, 1_048_576, -1, -8_192] {
            let mut encoded = Vec::new();
            write_compact(&mut encoded, value);
            assert_eq!(Reader::new(&encoded).read_compact().unwrap(), value);
        }
    }

    #[test]
    fn skips_the_variable_l2_metadata_before_texture_mips() {
        let mut data = vec![0; 92];
        write_compact(&mut data, 1);
        write_unreal_string(&mut data, "TextureMeta").unwrap();
        data.push(2);
        write_unreal_string(&mut data, "First").unwrap();
        write_unreal_string(&mut data, "Second").unwrap();
        write_unreal_string(&mut data, "TailMeta").unwrap();
        data.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
        data.push(0x5a);

        let mut reader = Reader::new(&data);
        skip_unreal_extra(&mut reader, 118, 40).unwrap();

        assert_eq!(reader.read_u8().unwrap(), 0x5a);
    }

    #[test]
    fn creates_and_populates_a_v118_package_with_its_own_writer() {
        const TEMPLATE: &[u8] = include_bytes!("../assets/UnrealTlp.utx");
        let empty = create_empty_package(TEMPLATE, "UnrealTlp", "EngineWriter").unwrap();
        let package = TexturePackage::parse(empty.clone()).unwrap();
        assert_eq!(package.version, UE2_PACKAGE_VERSION);
        assert_eq!(package.licensee, UE2_LICENSEE_VERSION);
        assert!(package.exports.is_empty());
        assert!(package.names.index_of("EngineWriter").is_some());

        let textures = [
            TextureImportRequest {
                name: "common".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![10, 20, 30, 255],
                alpha: None,
                masked: None,
                clamp: Some(TextureClampEdit {
                    u_clamp: Some(64),
                    v_clamp: Some(32),
                    u_clamp_mode: Some(2),
                    v_clamp_mode: Some(3),
                }),
                split9: None,
                animation: None,
            },
            TextureImportRequest {
                name: "split".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![40, 50, 60, 255],
                alpha: Some(true),
                masked: None,
                clamp: None,
                split9: Some(Split9Edit {
                    enabled: true,
                    x1: 1,
                    x2: 2,
                    x3: 3,
                    y1: 4,
                    y2: 5,
                    y3: 6,
                }),
                animation: None,
            },
            TextureImportRequest {
                name: "animated_first".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![70, 80, 90, 255],
                alpha: None,
                masked: None,
                clamp: None,
                split9: None,
                animation: Some(TextureAnimationImport {
                    anim_next: Some("animated_second".into()),
                    max_frame_rate: Some(24.0),
                    min_frame_rate: Some(12.0),
                    one_time_anim_loop: Some(false),
                    prime_count: Some(0),
                    total_frame_num: Some(2),
                }),
            },
            TextureImportRequest {
                name: "animated_second".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![100, 110, 120, 255],
                alpha: None,
                masked: None,
                clamp: None,
                split9: None,
                animation: Some(TextureAnimationImport {
                    anim_next: Some("animated_first".into()),
                    max_frame_rate: Some(24.0),
                    min_frame_rate: Some(12.0),
                    one_time_anim_loop: Some(true),
                    prime_count: Some(1),
                    total_frame_num: Some(2),
                }),
            },
            TextureImportRequest {
                name: "split_animated".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![130, 140, 150, 255],
                alpha: None,
                masked: None,
                clamp: None,
                split9: Some(Split9Edit {
                    enabled: true,
                    x1: 7,
                    x2: 8,
                    x3: 9,
                    y1: 10,
                    y2: 11,
                    y3: 12,
                }),
                animation: Some(TextureAnimationImport {
                    anim_next: Some("animated_first".into()),
                    max_frame_rate: Some(30.0),
                    min_frame_rate: Some(15.0),
                    one_time_anim_loop: Some(true),
                    prime_count: Some(2),
                    total_frame_num: Some(3),
                }),
            },
        ];
        let (written, outcomes) =
            import_new_textures(empty, TEMPLATE, "CandidateWnd", &textures).unwrap();
        assert_eq!(outcomes.len(), textures.len());
        let package = TexturePackage::parse(written).unwrap();
        assert_eq!(package.exports.len(), textures.len() + 1);
        let common = package.export(outcomes[0].export_index).unwrap();
        let common_state =
            TexturePropertyStream::parse(package.export_data(common).unwrap(), &package.names)
                .unwrap()
                .editor_state();
        assert_eq!(common_state.u_clamp, Some(64));
        assert_eq!(common_state.v_clamp, Some(32));
        assert_eq!(common_state.u_clamp_mode, Some(2));
        assert_eq!(common_state.v_clamp_mode, Some(3));
        let split = package.export(outcomes[1].export_index).unwrap();
        let split_state =
            TexturePropertyStream::parse(package.export_data(split).unwrap(), &package.names)
                .unwrap()
                .editor_state();
        assert_eq!(split_state.split9, Some(true));
        assert_eq!((split_state.split9_x1, split_state.split9_y3), (1, 6));
        let animated = package.export(outcomes[2].export_index).unwrap();
        let animated_state =
            TexturePropertyStream::parse(package.export_data(animated).unwrap(), &package.names)
                .unwrap()
                .editor_state();
        assert_eq!(animated_state.animation.max_frame_rate, Some(24.0));
        assert_eq!(animated_state.animation.one_time_anim_loop, Some(false));
        assert_eq!(animated_state.animation.prime_count, Some(0));
        assert_eq!(animated_state.animation.total_frame_num, Some(2));
        assert_eq!(
            animated_state.animation.anim_next,
            Some(outcomes[3].export_index as i32 + 1)
        );
        let split_animated = package.export(outcomes[4].export_index).unwrap();
        let split_animated_state = TexturePropertyStream::parse(
            package.export_data(split_animated).unwrap(),
            &package.names,
        )
        .unwrap()
        .editor_state();
        assert_eq!(split_animated_state.split9, Some(true));
        assert_eq!(
            (
                split_animated_state.split9_x1,
                split_animated_state.split9_y3
            ),
            (7, 12)
        );
        assert_eq!(split_animated_state.animation.max_frame_rate, Some(30.0));
        assert_eq!(
            split_animated_state.animation.anim_next,
            Some(outcomes[2].export_index as i32 + 1)
        );
    }

    #[test]
    fn imports_a_mixed_batch_with_native_replace_and_add_paths() {
        const TEMPLATE: &[u8] = include_bytes!("../assets/UnrealTlp.utx");
        let empty = create_empty_package(TEMPLATE, "UnrealTlp", "EngineWriter").unwrap();
        let original = TextureImportRequest {
            name: "existing".into(),
            format: 6,
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 255],
            alpha: None,
            masked: None,
            clamp: None,
            split9: None,
            animation: None,
        };
        let (initial, _) =
            import_new_textures(empty, TEMPLATE, "CandidateWnd", &[original]).unwrap();

        let changes = [
            TextureImportRequest {
                name: "existing".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![20, 21, 22, 255],
                alpha: Some(true),
                masked: None,
                clamp: None,
                split9: None,
                animation: None,
            },
            TextureImportRequest {
                name: "added".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![30, 31, 32, 255],
                alpha: None,
                masked: None,
                clamp: None,
                split9: None,
                animation: None,
            },
        ];
        let (written, outcomes) =
            import_textures(initial, TEMPLATE, "CandidateWnd", &changes).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(!outcomes[0].added);
        assert!(outcomes[1].added);

        let package = TexturePackage::parse(written).unwrap();
        assert_eq!(package.exports.len(), 3);
        let existing = package.export(outcomes[0].export_index).unwrap();
        let mip = package.mip0_location(outcomes[0].export_index).unwrap();
        let raw = package.export_data(existing).unwrap();
        assert_eq!(
            &raw[mip.pixel_offset..mip.pixel_offset + 4],
            &[20, 21, 22, 255]
        );
        let state = TexturePropertyStream::parse(raw, &package.names)
            .unwrap()
            .editor_state();
        assert_eq!(state.alpha, Some(true));
    }

    #[test]
    fn replacement_metadata_keeps_the_existing_export_layout() {
        const TEMPLATE: &[u8] = include_bytes!("../assets/UnrealTlp.utx");
        let empty = create_empty_package(TEMPLATE, "UnrealTlp", "LayoutWriter").unwrap();
        let source = TextureImportRequest {
            name: "existing".into(),
            format: 6,
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 255],
            alpha: Some(true),
            masked: Some(false),
            clamp: None,
            split9: None,
            animation: None,
        };
        let (initial, outcomes) =
            import_new_textures(empty, TEMPLATE, "CandidateWnd", &[source]).unwrap();
        let export_index = outcomes[0].export_index;
        let before = TexturePackage::parse(initial.clone()).unwrap();
        let before_export = before.export(export_index).unwrap().clone();

        let replacement = TextureImportRequest {
            name: "existing".into(),
            format: 6,
            width: 1,
            height: 1,
            pixels: vec![10, 20, 30, 255],
            alpha: Some(false),
            masked: Some(true),
            clamp: None,
            split9: None,
            animation: None,
        };
        let replaced = replace_texture(initial, export_index, &replacement).unwrap();
        let after = TexturePackage::parse(replaced.clone()).unwrap();
        let after_export = after.export(export_index).unwrap();

        assert_eq!(replaced.len(), before.data.len());
        assert_eq!(after_export.offset, before_export.offset);
        assert_eq!(after_export.size, before_export.size);
        let state =
            TexturePropertyStream::parse(after.export_data(after_export).unwrap(), &after.names)
                .unwrap()
                .editor_state();
        assert_eq!(state.alpha, Some(false));
        assert_eq!(state.masked, Some(true));
    }

    #[test]
    fn edits_properties_without_replacing_texture_pixels() {
        const TEMPLATE: &[u8] = include_bytes!("../assets/UnrealTlp.utx");
        let empty = create_empty_package(TEMPLATE, "UnrealTlp", "PropertyWriter").unwrap();
        let source = TextureImportRequest {
            name: "editable".into(),
            format: 6,
            width: 1,
            height: 1,
            pixels: vec![31, 63, 95, 255],
            alpha: None,
            masked: None,
            clamp: None,
            split9: None,
            animation: None,
        };
        let (written, outcomes) =
            import_new_textures(empty, TEMPLATE, "PropertyGroup", &[source]).unwrap();
        let export_index = outcomes[0].export_index;
        let before_package = TexturePackage::parse(written.clone()).unwrap();
        let before_export = before_package.export(export_index).unwrap();
        let before_mip = before_package.mip0_location(export_index).unwrap();
        let before_pixels = before_package.export_data(before_export).unwrap()
            [before_mip.pixel_offset..before_mip.pixel_offset + 4]
            .to_vec();

        let edited = edit_texture_properties(
            written,
            export_index,
            TextureEditorEdit {
                alpha: Some(true),
                masked: Some(true),
                clamp: Some(TextureClampEdit {
                    u_clamp: Some(32),
                    v_clamp: Some(48),
                    u_clamp_mode: Some(2),
                    v_clamp_mode: Some(3),
                }),
                split9: Some(Split9Edit {
                    enabled: true,
                    x1: 1,
                    x2: 2,
                    x3: 3,
                    y1: 4,
                    y2: 5,
                    y3: 6,
                }),
                animation: Some(TextureAnimationEdit {
                    anim_next: Some(export_index as i32 + 1),
                    max_frame_rate: Some(20.0),
                    min_frame_rate: Some(10.0),
                    one_time_anim_loop: Some(true),
                    prime_count: Some(1),
                    total_frame_num: Some(1),
                }),
            },
        )
        .unwrap();
        let state = texture_editor_state(edited.clone(), export_index).unwrap();
        assert_eq!(state.alpha, Some(true));
        assert_eq!(state.masked, Some(true));
        assert_eq!((state.u_clamp, state.v_clamp), (Some(32), Some(48)));
        assert_eq!(state.split9, Some(true));
        assert_eq!((state.split9_x1, state.split9_y3), (1, 6));
        assert_eq!(state.animation.anim_next, Some(export_index as i32 + 1));
        assert_eq!(state.animation.max_frame_rate, Some(20.0));
        assert_eq!(state.animation.one_time_anim_loop, Some(true));

        let after_package = TexturePackage::parse(edited).unwrap();
        let after_export = after_package.export(export_index).unwrap();
        let after_mip = after_package.mip0_location(export_index).unwrap();
        let after_pixels = after_package.export_data(after_export).unwrap()
            [after_mip.pixel_offset..after_mip.pixel_offset + 4]
            .to_vec();
        assert_eq!(after_pixels, before_pixels);
    }

    #[test]
    fn batch_property_edits_apply_the_same_values_to_every_texture() {
        const TEMPLATE: &[u8] = include_bytes!("../assets/UnrealTlp.utx");
        let empty = create_empty_package(TEMPLATE, "UnrealTlp", "BatchPropertyWriter").unwrap();
        let textures = [
            TextureImportRequest {
                name: "first".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![10, 20, 30, 255],
                alpha: None,
                masked: None,
                clamp: None,
                split9: None,
                animation: None,
            },
            TextureImportRequest {
                name: "second".into(),
                format: 6,
                width: 1,
                height: 1,
                pixels: vec![40, 50, 60, 255],
                alpha: None,
                masked: None,
                clamp: None,
                split9: None,
                animation: None,
            },
        ];
        let (written, outcomes) =
            import_new_textures(empty, TEMPLATE, "BatchGroup", &textures).unwrap();
        let edit = TextureEditorEdit {
            alpha: Some(true),
            masked: Some(false),
            clamp: None,
            split9: Some(Split9Edit {
                enabled: true,
                x1: 1,
                x2: 2,
                x3: 3,
                y1: 4,
                y2: 5,
                y3: 6,
            }),
            animation: None,
        };
        let edited = edit_texture_properties_batch(
            written,
            &outcomes
                .iter()
                .map(|outcome| (outcome.export_index, edit.clone()))
                .collect::<Vec<_>>(),
        )
        .unwrap();

        for outcome in outcomes {
            let state = texture_editor_state(edited.clone(), outcome.export_index).unwrap();
            assert_eq!(state.alpha, Some(true));
            assert_eq!(state.masked, Some(false));
            assert_eq!(state.split9, Some(true));
            assert_eq!((state.split9_x1, state.split9_y3), (1, 6));
        }
    }

    #[test]
    fn duplicates_a_texture_into_a_new_group_without_changing_its_payload() {
        const TEMPLATE: &[u8] = include_bytes!("../assets/UnrealTlp.utx");
        let empty = create_empty_package(TEMPLATE, "UnrealTlp", "DuplicateWriter").unwrap();
        let source = TextureImportRequest {
            name: "source".into(),
            format: 6,
            width: 1,
            height: 1,
            pixels: vec![25, 50, 75, 255],
            alpha: Some(true),
            masked: Some(true),
            clamp: None,
            split9: Some(Split9Edit {
                enabled: true,
                x1: 1,
                x2: 2,
                x3: 3,
                y1: 4,
                y2: 5,
                y3: 6,
            }),
            animation: None,
        };
        let (written, outcomes) =
            import_new_textures(empty, TEMPLATE, "SourceGroup", &[source]).unwrap();
        let source_export_index = outcomes[0].export_index;
        let (duplicated, duplicate_export_index) =
            duplicate_texture(written, source_export_index, "NewGroup", "copy").unwrap();
        let package = TexturePackage::parse(duplicated.clone()).unwrap();
        assert_eq!(
            package
                .inner_name(package.export(duplicate_export_index).unwrap())
                .unwrap(),
            "NewGroup.copy"
        );
        let state = texture_editor_state(duplicated.clone(), duplicate_export_index).unwrap();
        assert_eq!(state.alpha, Some(true));
        assert_eq!(state.masked, Some(true));
        assert_eq!(state.split9, Some(true));
        assert_eq!((state.split9_x1, state.split9_y3), (1, 6));

        let source_export = package.export(source_export_index).unwrap();
        let duplicate_export = package.export(duplicate_export_index).unwrap();
        let source_mip = package.mip0_location(source_export_index).unwrap();
        let duplicate_mip = package.mip0_location(duplicate_export_index).unwrap();
        let source_pixels = package.export_data(source_export).unwrap()
            [source_mip.pixel_offset..source_mip.pixel_offset + source_mip.size]
            .to_vec();
        let duplicate_pixels = package.export_data(duplicate_export).unwrap()
            [duplicate_mip.pixel_offset..duplicate_mip.pixel_offset + duplicate_mip.size]
            .to_vec();
        assert_eq!(duplicate_pixels, source_pixels);

        let renamed = rename_texture(duplicated, duplicate_export_index, "renamed").unwrap();
        let renamed_package = TexturePackage::parse(renamed.clone()).unwrap();
        assert_eq!(
            renamed_package
                .inner_name(renamed_package.export(duplicate_export_index).unwrap())
                .unwrap(),
            "NewGroup.renamed"
        );
        let renamed_state = texture_editor_state(renamed.clone(), duplicate_export_index).unwrap();
        assert_eq!(renamed_state.alpha, Some(true));
        assert_eq!(renamed_state.split9, Some(true));
        assert!(duplicate_texture(renamed, source_export_index, "NewGroup", "renamed").is_err());
    }
}
