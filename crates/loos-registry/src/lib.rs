//! A bounded reader and canonical writer for the classic Windows registry hive.
//!
//! The public model is deliberately small: keys form a tree and values contain
//! a type plus bytes. Loading validates cell bounds, allocated-cell markers,
//! list signatures, names, and recursion/cycle limits before constructing that
//! model. Saving emits a conventional `regf` base block and `hbin` cells, so a
//! hive produced here can be consumed by standard registry tooling.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const BASE_BLOCK: usize = 0x1000;
const HBIN_HEADER: usize = 0x20;
const MAX_HIVE: usize = 64 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_CHILDREN: usize = 1_000_000;
const MAX_VALUES: usize = 1_000_000;
const NO_CELL: u32 = 0xffff_ffff;
const KEY_COMP_NAME: u16 = 0x0020;
const VALUE_COMP_NAME: u16 = 0x0001;
const VALUE_INLINE: u32 = 0x8000_0000;

/// Registry value types. Unknown numeric types are retained as [`ValueType::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    /// NUL-terminated UTF-16 string.
    Sz,
    /// Expandable UTF-16 string containing environment references.
    ExpandSz,
    /// Raw bytes.
    Binary,
    /// Little-endian 32-bit integer.
    Dword,
    /// Little-endian 64-bit integer.
    Qword,
    /// Multiple NUL-terminated UTF-16 strings.
    MultiSz,
    /// An extension type preserved by its numeric identifier.
    Other(u32),
}

impl ValueType {
    fn raw(self) -> u32 {
        match self {
            Self::Sz => 1,
            Self::ExpandSz => 2,
            Self::Binary => 3,
            Self::Dword => 4,
            Self::MultiSz => 7,
            Self::Qword => 11,
            Self::Other(value) => value,
        }
    }

    fn from_raw(value: u32) -> Self {
        match value {
            1 => Self::Sz,
            2 => Self::ExpandSz,
            3 => Self::Binary,
            4 => Self::Dword,
            7 => Self::MultiSz,
            11 => Self::Qword,
            value => Self::Other(value),
        }
    }
}

/// A registry value, including its name and raw encoded data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    /// Value name. The empty name is the key's default value.
    pub name: String,
    /// Registry type.
    pub kind: ValueType,
    /// Type-specific bytes, normally including the UTF-16 terminator for strings.
    pub data: Vec<u8>,
}

/// A registry key and its descendants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// Key's component name. The root name is informational.
    pub name: String,
    /// Values directly attached to this key.
    pub values: Vec<Value>,
    /// Child keys.
    pub children: Vec<Key>,
}

impl Key {
    fn new(name: String) -> Self {
        Self {
            name,
            values: Vec::new(),
            children: Vec::new(),
        }
    }

    fn child(&self, name: &str) -> Option<&Self> {
        self.children
            .iter()
            .find(|key| key.name.eq_ignore_ascii_case(name))
    }

    fn child_mut(&mut self, name: &str) -> Option<&mut Self> {
        self.children
            .iter_mut()
            .find(|key| key.name.eq_ignore_ascii_case(name))
    }
}

/// Hive parser/writer failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input is not a registry hive or has inconsistent bounds.
    Corrupt,
    /// The requested key or value does not exist.
    NotFound,
    /// A path/name is empty or contains a separator.
    InvalidName,
    /// A key or value already exists.
    Exists,
    /// The input uses a structure outside this implementation's bounded subset.
    Unsupported,
    /// The requested output exceeds the supported hive size.
    TooLarge,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Corrupt => "corrupt registry hive",
            Self::NotFound => "registry key or value not found",
            Self::InvalidName => "invalid registry name",
            Self::Exists => "registry key or value already exists",
            Self::Unsupported => "unsupported registry hive structure",
            Self::TooLarge => "registry hive too large",
        })
    }
}

/// A parsed registry hive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hive {
    root: Key,
}

impl Default for Hive {
    fn default() -> Self {
        Self::new()
    }
}

impl Hive {
    /// Creates an empty hive with a conventional root key.
    pub fn new() -> Self {
        Self {
            root: Key::new(String::from("ROOT")),
        }
    }

    /// Returns the root key.
    pub fn root(&self) -> &Key {
        &self.root
    }

    /// Returns a key using `\`-separated, case-insensitive components.
    pub fn key(&self, path: &str) -> Result<&Key, Error> {
        let parts = components(path)?;
        let mut key = &self.root;
        for part in parts {
            key = key.child(part).ok_or(Error::NotFound)?;
        }
        Ok(key)
    }

    /// Returns a mutable key using `\`-separated components.
    pub fn key_mut(&mut self, path: &str) -> Result<&mut Key, Error> {
        let parts = components(path)?;
        let mut key = &mut self.root;
        for part in parts {
            key = key.child_mut(part).ok_or(Error::NotFound)?;
        }
        Ok(key)
    }

    /// Creates all missing keys in a path and returns the final key.
    pub fn create_key(&mut self, path: &str) -> Result<&mut Key, Error> {
        let parts = components(path)?;
        let mut key = &mut self.root;
        for part in parts {
            if key.child(part).is_none() {
                key.children.push(Key::new(String::from(part)));
            }
            key = key.child_mut(part).ok_or(Error::Corrupt)?;
        }
        Ok(key)
    }

    /// Gets a value by key path and value name.
    pub fn value(&self, path: &str, name: &str) -> Result<&Value, Error> {
        if name.contains('\\') || name.contains('/') {
            return Err(Error::InvalidName);
        }
        self.key(path)?
            .values
            .iter()
            .find(|value| value.name.eq_ignore_ascii_case(name))
            .ok_or(Error::NotFound)
    }

    /// Inserts or replaces a value on a key.
    pub fn set_value(
        &mut self,
        path: &str,
        name: &str,
        kind: ValueType,
        data: &[u8],
    ) -> Result<(), Error> {
        if name.contains('\\') || name.contains('/') {
            return Err(Error::InvalidName);
        }
        let key = self.create_key(path)?;
        if let Some(value) = key
            .values
            .iter_mut()
            .find(|value| value.name.eq_ignore_ascii_case(name))
        {
            value.kind = kind;
            value.data.clear();
            value.data.extend_from_slice(data);
        } else {
            key.values.push(Value {
                name: String::from(name),
                kind,
                data: data.into(),
            });
        }
        Ok(())
    }

    /// Deletes a value from a key.
    pub fn delete_value(&mut self, path: &str, name: &str) -> Result<(), Error> {
        let key = self.key_mut(path)?;
        let index = key
            .values
            .iter()
            .position(|value| value.name.eq_ignore_ascii_case(name))
            .ok_or(Error::NotFound)?;
        key.values.remove(index);
        Ok(())
    }

    /// Parses a classic registry hive image.
    pub fn load(data: &[u8]) -> Result<Self, Error> {
        if data.len() < BASE_BLOCK + HBIN_HEADER || data.len() > MAX_HIVE || &data[0..4] != b"regf"
        {
            return Err(Error::Corrupt);
        }
        let bins_size = read_u32(data, 40)? as usize;
        if bins_size == 0
            || bins_size % 0x1000 != 0
            || BASE_BLOCK.checked_add(bins_size).ok_or(Error::Corrupt)? > data.len()
        {
            return Err(Error::Corrupt);
        }
        validate_bins(data, bins_size)?;
        let hive_end = BASE_BLOCK.checked_add(bins_size).ok_or(Error::Corrupt)?;
        let image = &data[..hive_end];
        let stored_checksum = read_u32(image, 0x1fc)?;
        if stored_checksum != 0 && stored_checksum != checksum(&image[..0x1fc]) {
            return Err(Error::Corrupt);
        }
        let root_offset = read_u32(image, 36)?;
        let mut stack = Vec::new();
        let root = parse_key(image, root_offset, &mut stack, 0)?;
        Ok(Self { root })
    }

    /// Serializes the hive to a canonical classic registry image.
    pub fn save(&self) -> Result<Vec<u8>, Error> {
        let mut bins = vec![0u8; HBIN_HEADER];
        let root_offset = write_key(&mut bins, &self.root, 0)?;
        let target = align_up(bins.len(), 0x1000).ok_or(Error::TooLarge)?;
        if target > MAX_HIVE - BASE_BLOCK {
            return Err(Error::TooLarge);
        }
        if target > bins.len() {
            let free = target - bins.len();
            if free < 4 {
                return Err(Error::TooLarge);
            }
            let old = bins.len();
            bins.resize(target, 0);
            write_i32(&mut bins, old, free as i32)?;
        }
        write_bytes(&mut bins, 0, b"hbin")?;
        write_u32(&mut bins, 4, 0)?;
        let bins_len = bins.len() as u32;
        write_u32(&mut bins, 8, bins_len)?;
        let mut output = vec![0u8; BASE_BLOCK];
        output.extend_from_slice(&bins);
        write_bytes(&mut output, 0, b"regf")?;
        write_u32(&mut output, 4, 1)?;
        write_u32(&mut output, 8, 1)?;
        write_u32(&mut output, 20, 1)?;
        write_u32(&mut output, 24, 5)?;
        write_u32(&mut output, 28, 0)?;
        write_u32(&mut output, 32, 1)?;
        write_u32(&mut output, 36, root_offset)?;
        write_u32(&mut output, 40, bins.len() as u32)?;
        write_u32(&mut output, 44, 1)?;
        let checksum = checksum(&output[0..0x1fc]);
        write_u32(&mut output, 0x1fc, checksum)?;
        Ok(output)
    }
}

fn components(path: &str) -> Result<Vec<&str>, Error> {
    let mut result = Vec::new();
    for component in path.split(|c| c == '\\' || c == '/') {
        if component.is_empty() {
            continue;
        }
        if component == "." || component == ".." || component.chars().any(|c| c == '\0') {
            return Err(Error::InvalidName);
        }
        result.push(component);
    }
    Ok(result)
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, Error> {
    if offset.checked_add(2).ok_or(Error::Corrupt)? > data.len() {
        return Err(Error::Corrupt);
    }
    Ok(u16::from_le_bytes([data[offset], data[offset + 1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, Error> {
    if offset.checked_add(4).ok_or(Error::Corrupt)? > data.len() {
        return Err(Error::Corrupt);
    }
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

fn write_bytes(data: &mut [u8], offset: usize, value: &[u8]) -> Result<(), Error> {
    let end = offset.checked_add(value.len()).ok_or(Error::TooLarge)?;
    if end > data.len() {
        return Err(Error::TooLarge);
    }
    data[offset..end].copy_from_slice(value);
    Ok(())
}

fn write_u16(data: &mut [u8], offset: usize, value: u16) -> Result<(), Error> {
    write_bytes(data, offset, &value.to_le_bytes())
}
fn write_u32(data: &mut [u8], offset: usize, value: u32) -> Result<(), Error> {
    write_bytes(data, offset, &value.to_le_bytes())
}
fn write_i32(data: &mut [u8], offset: usize, value: i32) -> Result<(), Error> {
    write_bytes(data, offset, &value.to_le_bytes())
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
}

fn checksum(data: &[u8]) -> u32 {
    let mut value = 0;
    for chunk in data.chunks_exact(4) {
        value ^= u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    value
}

fn validate_bins(data: &[u8], bins_size: usize) -> Result<(), Error> {
    let end = BASE_BLOCK.checked_add(bins_size).ok_or(Error::Corrupt)?;
    let mut offset = BASE_BLOCK;
    while offset < end {
        if offset + HBIN_HEADER > end || &data[offset..offset + 4] != b"hbin" {
            return Err(Error::Corrupt);
        }
        let bin_offset = read_u32(data, offset + 4)? as usize;
        let size = read_u32(data, offset + 8)? as usize;
        if bin_offset != offset - BASE_BLOCK
            || size < 0x1000
            || size % 0x1000 != 0
            || offset.checked_add(size).ok_or(Error::Corrupt)? > end
        {
            return Err(Error::Corrupt);
        }
        offset += size;
    }
    if offset != end {
        return Err(Error::Corrupt);
    }
    Ok(())
}

fn cell<'a>(data: &'a [u8], offset: u32) -> Result<&'a [u8], Error> {
    if offset == NO_CELL {
        return Err(Error::Corrupt);
    }
    let absolute = BASE_BLOCK
        .checked_add(offset as usize)
        .ok_or(Error::Corrupt)?;
    let size_raw = read_u32(data, absolute)? as i32;
    if size_raw >= 0 || size_raw == i32::MIN {
        return Err(Error::Corrupt);
    }
    let size = (-size_raw) as usize;
    if size < 4 || absolute.checked_add(size).ok_or(Error::Corrupt)? > data.len() {
        return Err(Error::Corrupt);
    }
    Ok(&data[absolute + 4..absolute + size])
}

fn decode_name(raw: &[u8], compressed: bool) -> Result<String, Error> {
    if compressed {
        if raw.iter().any(|byte| *byte >= 0x80) {
            return Err(Error::Unsupported);
        }
        let mut result = String::new();
        for byte in raw {
            result.push(*byte as char);
        }
        Ok(result)
    } else {
        if raw.len() % 2 != 0 {
            return Err(Error::Corrupt);
        }
        let mut result = String::new();
        let words: Vec<u16> = raw
            .chunks_exact(2)
            .map(|word| u16::from_le_bytes([word[0], word[1]]))
            .collect();
        let mut index = 0;
        while index < words.len() {
            let value = words[index];
            if value == 0 {
                return Err(Error::Corrupt);
            }
            let codepoint = if (0xd800..=0xdbff).contains(&value) {
                index += 1;
                if index == words.len() || !(0xdc00..=0xdfff).contains(&words[index]) {
                    return Err(Error::Unsupported);
                }
                0x1_0000 + (((value as u32) - 0xd800) << 10) + (words[index] as u32 - 0xdc00)
            } else if (0xdc00..=0xdfff).contains(&value) {
                return Err(Error::Unsupported);
            } else {
                value as u32
            };
            result.push(core::char::from_u32(codepoint).ok_or(Error::Unsupported)?);
            index += 1;
        }
        Ok(result)
    }
}

fn encode_name(name: &str) -> (u16, Vec<u8>) {
    if name.chars().all(|c| c as u32 <= 0x7f) {
        (KEY_COMP_NAME, name.as_bytes().into())
    } else {
        let mut bytes = Vec::new();
        for value in name.encode_utf16() {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        (0, bytes)
    }
}

fn collect_subkeys(
    data: &[u8],
    offset: u32,
    output: &mut Vec<u32>,
    depth: usize,
) -> Result<(), Error> {
    if offset == NO_CELL {
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err(Error::Corrupt);
    }
    let payload = cell(data, offset)?;
    if payload.len() < 4 {
        return Err(Error::Corrupt);
    }
    let signature = [payload[0], payload[1]];
    let count = read_u16(payload, 2)? as usize;
    if count > MAX_CHILDREN || output.len().checked_add(count).ok_or(Error::Corrupt)? > MAX_CHILDREN
    {
        return Err(Error::Corrupt);
    }
    match &signature {
        b"li" => {
            let end = 4usize
                .checked_add(count.checked_mul(4).ok_or(Error::Corrupt)?)
                .ok_or(Error::Corrupt)?;
            if end > payload.len() {
                return Err(Error::Corrupt);
            }
            for i in 0..count {
                output.push(read_u32(payload, 4 + i * 4)?);
            }
        }
        b"lf" | b"lh" => {
            let end = 4usize
                .checked_add(count.checked_mul(8).ok_or(Error::Corrupt)?)
                .ok_or(Error::Corrupt)?;
            if end > payload.len() {
                return Err(Error::Corrupt);
            }
            for i in 0..count {
                output.push(read_u32(payload, 4 + i * 8)?);
            }
        }
        b"ri" => {
            let end = 4usize
                .checked_add(count.checked_mul(4).ok_or(Error::Corrupt)?)
                .ok_or(Error::Corrupt)?;
            if end > payload.len() {
                return Err(Error::Corrupt);
            }
            for i in 0..count {
                collect_subkeys(data, read_u32(payload, 4 + i * 4)?, output, depth + 1)?;
            }
        }
        _ => return Err(Error::Corrupt),
    }
    Ok(())
}

fn parse_value(data: &[u8], offset: u32) -> Result<Value, Error> {
    let payload = cell(data, offset)?;
    if payload.len() < 20 || &payload[0..2] != b"vk" {
        return Err(Error::Corrupt);
    }
    let name_len = read_u16(payload, 2)? as usize;
    let data_len_raw = read_u32(payload, 4)?;
    let data_len = (data_len_raw & !VALUE_INLINE) as usize;
    let data_offset = read_u32(payload, 8)?;
    let kind = ValueType::from_raw(read_u32(payload, 12)?);
    let flags = read_u16(payload, 16)?;
    let name_end = 20usize.checked_add(name_len).ok_or(Error::Corrupt)?;
    if name_end > payload.len() {
        return Err(Error::Corrupt);
    }
    let name = decode_name(&payload[20..name_end], flags & VALUE_COMP_NAME != 0)?;
    let bytes = if data_len_raw & VALUE_INLINE != 0 {
        if data_len > 4 {
            return Err(Error::Corrupt);
        }
        data_offset.to_le_bytes()[..data_len].into()
    } else if data_len == 0 {
        Vec::new()
    } else {
        let source = cell(data, data_offset)?;
        if data_len > source.len() {
            return Err(Error::Corrupt);
        }
        source[..data_len].into()
    };
    Ok(Value {
        name,
        kind,
        data: bytes,
    })
}

fn parse_key(data: &[u8], offset: u32, stack: &mut Vec<u32>, depth: usize) -> Result<Key, Error> {
    if depth > MAX_DEPTH || stack.contains(&offset) {
        return Err(Error::Corrupt);
    }
    stack.push(offset);
    let result = (|| {
        let payload = cell(data, offset)?;
        if payload.len() < 76 || &payload[0..2] != b"nk" {
            return Err(Error::Corrupt);
        }
        let flags = read_u16(payload, 2)?;
        let name_len = read_u16(payload, 72)? as usize;
        let class_len = read_u16(payload, 74)? as usize;
        let name_end = 76usize.checked_add(name_len).ok_or(Error::Corrupt)?;
        if name_end > payload.len()
            || name_end.checked_add(class_len).ok_or(Error::Corrupt)? > payload.len()
        {
            return Err(Error::Corrupt);
        }
        let mut key = Key::new(decode_name(
            &payload[76..name_end],
            flags & KEY_COMP_NAME != 0,
        )?);
        let value_count = read_u32(payload, 36)? as usize;
        if value_count > MAX_VALUES {
            return Err(Error::Corrupt);
        }
        let value_list = read_u32(payload, 40)?;
        if value_count != 0 {
            let list = cell(data, value_list)?;
            if list.len() < value_count.checked_mul(4).ok_or(Error::Corrupt)? {
                return Err(Error::Corrupt);
            }
            for i in 0..value_count {
                key.values.push(parse_value(data, read_u32(list, i * 4)?)?);
            }
        }
        let mut child_offsets = Vec::new();
        collect_subkeys(data, read_u32(payload, 28)?, &mut child_offsets, 0)?;
        for child in child_offsets {
            key.children.push(parse_key(data, child, stack, depth + 1)?);
        }
        Ok(key)
    })();
    stack.pop();
    result
}

fn alloc_cell(bins: &mut Vec<u8>, payload_len: usize) -> Result<u32, Error> {
    let total =
        align_up(payload_len.checked_add(4).ok_or(Error::TooLarge)?, 8).ok_or(Error::TooLarge)?;
    if total > i32::MAX as usize
        || bins.len().checked_add(total).ok_or(Error::TooLarge)? > MAX_HIVE - BASE_BLOCK
    {
        return Err(Error::TooLarge);
    }
    // Cell offsets are relative to the beginning of the hbin, including its
    // 0x20-byte header. The first cell is therefore offset 0x20.
    let offset = bins.len() as u32;
    let old = bins.len();
    bins.resize(old + total, 0);
    write_i32(bins, old, -(total as i32))?;
    Ok(offset)
}

fn write_value(bins: &mut Vec<u8>, value: &Value) -> Result<u32, Error> {
    let (name_flags, name) = encode_name(&value.name);
    let flags = if name_flags & KEY_COMP_NAME != 0 {
        VALUE_COMP_NAME
    } else {
        0
    };
    let inline = value.data.len() <= 4;
    let data_offset = if inline {
        let mut bytes = [0u8; 4];
        bytes[..value.data.len()].copy_from_slice(&value.data);
        u32::from_le_bytes(bytes)
    } else {
        let offset = alloc_cell(bins, value.data.len())?;
        let absolute = offset as usize + 4;
        write_bytes(bins, absolute, &value.data)?;
        offset
    };
    let payload_len = 20usize.checked_add(name.len()).ok_or(Error::TooLarge)?;
    let offset = alloc_cell(bins, payload_len)?;
    let absolute = offset as usize + 4;
    write_bytes(bins, absolute, b"vk")?;
    write_u16(bins, absolute + 2, name.len() as u16)?;
    write_u32(
        bins,
        absolute + 4,
        (value.data.len() as u32) | if inline { VALUE_INLINE } else { 0 },
    )?;
    write_u32(bins, absolute + 8, data_offset)?;
    write_u32(bins, absolute + 12, value.kind.raw())?;
    write_u16(bins, absolute + 16, flags & VALUE_COMP_NAME)?;
    write_u16(bins, absolute + 18, 0)?;
    write_bytes(bins, absolute + 20, &name)?;
    Ok(offset)
}

fn write_key(bins: &mut Vec<u8>, key: &Key, depth: usize) -> Result<u32, Error> {
    if depth > MAX_DEPTH
        || key.name.len() > u16::MAX as usize
        || key.values.len() > MAX_VALUES
        || key.children.len() > u16::MAX as usize
    {
        return Err(Error::TooLarge);
    }
    let mut value_offsets = Vec::new();
    for value in &key.values {
        value_offsets.push(write_value(bins, value)?);
    }
    let value_list = if value_offsets.is_empty() {
        NO_CELL
    } else {
        let offset = alloc_cell(bins, value_offsets.len() * 4)?;
        let absolute = offset as usize + 4;
        for (i, value) in value_offsets.iter().enumerate() {
            write_u32(bins, absolute + i * 4, *value)?;
        }
        offset
    };
    let mut child_offsets = Vec::new();
    for child in &key.children {
        child_offsets.push(write_key(bins, child, depth + 1)?);
    }
    let subkey_list = if child_offsets.is_empty() {
        NO_CELL
    } else {
        let offset = alloc_cell(bins, 4 + child_offsets.len() * 4)?;
        let absolute = offset as usize + 4;
        write_bytes(bins, absolute, b"li")?;
        write_u16(bins, absolute + 2, child_offsets.len() as u16)?;
        for (i, child) in child_offsets.iter().enumerate() {
            write_u32(bins, absolute + 4 + i * 4, *child)?;
        }
        offset
    };
    let (flags, name) = encode_name(&key.name);
    let payload_len = 76usize.checked_add(name.len()).ok_or(Error::TooLarge)?;
    let offset = alloc_cell(bins, payload_len)?;
    let absolute = offset as usize + 4;
    write_bytes(bins, absolute, b"nk")?;
    write_u16(bins, absolute + 2, flags)?;
    write_u32(bins, absolute + 12, 0)?;
    write_u32(bins, absolute + 16, NO_CELL)?;
    write_u32(bins, absolute + 20, child_offsets.len() as u32)?;
    write_u32(bins, absolute + 24, 0)?;
    write_u32(bins, absolute + 28, subkey_list)?;
    write_u32(bins, absolute + 32, NO_CELL)?;
    write_u32(bins, absolute + 36, value_offsets.len() as u32)?;
    write_u32(bins, absolute + 40, value_list)?;
    write_u32(bins, absolute + 44, NO_CELL)?;
    write_u32(bins, absolute + 48, NO_CELL)?;
    write_u32(bins, absolute + 52, 0)?;
    write_u32(bins, absolute + 56, 0)?;
    write_u32(bins, absolute + 60, 0)?;
    write_u32(bins, absolute + 64, 0)?;
    write_u32(bins, absolute + 68, 0)?;
    write_u16(bins, absolute + 72, name.len() as u16)?;
    write_u16(bins, absolute + 74, 0)?;
    write_bytes(bins, absolute + 76, &name)?;
    Ok(offset)
}
