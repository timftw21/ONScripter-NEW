//! Streaming SAR, NSA and NS2 indexes. Archive payloads are never scanned.

use std::{
    collections::HashMap,
    io::{BufReader, Read, Seek, SeekFrom},
    mem,
};

use crate::{Error, Limits, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveKind {
    Sar,
    Nsa,
    Ns2,
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub offset: u64,
    pub length: u64,
    pub original_length: u64,
    pub compression: u8,
}

#[derive(Debug)]
pub struct ArchiveIndex {
    pub kind: ArchiveKind,
    pub base_offset: u64,
    pub archive_bytes: u64,
    pub index_bytes: usize,
    entries: HashMap<Vec<u8>, Entry>,
}

impl ArchiveIndex {
    pub fn read(
        reader: &mut (impl Read + Seek),
        kind: ArchiveKind,
        offset: u64,
        limits: Limits,
    ) -> Result<Self> {
        let archive_bytes = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(offset))?;
        let mut header = Header {
            reader: BufReader::new(reader),
            position: offset,
            end: archive_bytes,
        };
        let count = if kind == ArchiveKind::Ns2 {
            None
        } else {
            Some(u16::from_be_bytes(header.bytes()?) as usize)
        };
        let base = if kind == ArchiveKind::Ns2 {
            u32::from_le_bytes(header.bytes()?)
        } else {
            u32::from_be_bytes(header.bytes()?)
        };
        let base_offset = offset
            .checked_add(u64::from(base))
            .ok_or_else(|| Error::invalid("archive offset overflow"))?;
        if base_offset < header.position || base_offset > archive_bytes {
            return Err(Error::invalid(
                "archive data offset is outside its header/file",
            ));
        }
        header.end = base_offset;
        let mut index = Self {
            kind,
            base_offset,
            archive_bytes,
            index_bytes: 0,
            entries: HashMap::new(),
        };
        if let Some(count) = count {
            let minimum = if kind == ArchiveKind::Nsa { 14 } else { 9 };
            if count as u64 > (base_offset - header.position) / minimum {
                return Err(Error::invalid("archive file count exceeds header size"));
            }
            for _ in 0..count {
                let name = header.name(0, limits.filename_bytes)?;
                let compression = if kind == ArchiveKind::Nsa {
                    header.byte()?
                } else {
                    0
                };
                let relative = u32::from_be_bytes(header.bytes()?);
                let length = u64::from(u32::from_be_bytes(header.bytes()?));
                let original_length = if kind == ArchiveKind::Nsa {
                    u64::from(u32::from_be_bytes(header.bytes()?))
                } else {
                    length
                };
                let entry_offset = base_offset
                    .checked_add(u64::from(relative))
                    .ok_or_else(|| Error::invalid("archive entry offset overflow"))?;
                index.insert(
                    name,
                    Entry {
                        offset: entry_offset,
                        length,
                        original_length,
                        compression,
                    },
                    limits,
                )?;
            }
        } else {
            let mut data_offset = base_offset;
            while header.position < base_offset {
                let delimiter = header.byte()?;
                if delimiter != b'"' {
                    // Legacy NS2 writers may use one trailing header byte.
                    if header.position == base_offset {
                        break;
                    }
                    return Err(Error::invalid("invalid NS2 filename delimiter"));
                }
                let name = header.name(b'"', limits.filename_bytes)?;
                let length = u64::from(u32::from_le_bytes(header.bytes()?));
                index.insert(
                    name,
                    Entry {
                        offset: data_offset,
                        length,
                        original_length: length,
                        compression: 0,
                    },
                    limits,
                )?;
                data_offset = data_offset
                    .checked_add(length)
                    .ok_or_else(|| Error::invalid("NS2 data offset overflow"))?;
            }
        }
        Ok(index)
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(&normalize_name(name.as_bytes()))
    }

    pub fn entries(&self) -> impl Iterator<Item = (&[u8], &Entry)> {
        self.entries
            .iter()
            .map(|(name, entry)| (name.as_slice(), entry))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn insert(&mut self, name: Vec<u8>, entry: Entry, limits: Limits) -> Result<()> {
        if entry.offset < self.base_offset
            || entry
                .offset
                .checked_add(entry.length)
                .is_none_or(|end| end > self.archive_bytes)
        {
            return Err(Error::invalid(
                "archive entry extends outside the data region",
            ));
        }
        let name = normalize_name(&name);
        // Keep the first duplicate, matching the existing engine's index.
        if self.entries.contains_key(&name) {
            return Ok(());
        }
        let cost = name
            .len()
            .checked_add(2 * (mem::size_of::<Vec<u8>>() + mem::size_of::<Entry>()) + 64)
            .ok_or(Error::Limit("archive index"))?;
        self.index_bytes = self
            .index_bytes
            .checked_add(cost)
            .filter(|&n| n <= limits.index_bytes)
            .ok_or(Error::Limit("archive index"))?;
        self.entries
            .try_reserve(1)
            .map_err(|_| Error::Limit("archive index allocation"))?;
        self.entries.insert(name, entry);
        Ok(())
    }
}

pub fn normalize_name(name: &[u8]) -> Vec<u8> {
    name.iter()
        .map(|&byte| {
            if byte == b'/' {
                b'\\'
            } else {
                byte.to_ascii_uppercase()
            }
        })
        .collect()
}

struct Header<R> {
    reader: R,
    position: u64,
    end: u64,
}

impl<R: Read> Header<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        if self.position > self.end || N as u64 > self.end - self.position {
            return Err(Error::invalid("truncated archive header"));
        }
        let mut bytes = [0; N];
        self.reader.read_exact(&mut bytes)?;
        self.position += N as u64;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.bytes::<1>()?[0])
    }

    fn name(&mut self, delimiter: u8, maximum: usize) -> Result<Vec<u8>> {
        let mut name = Vec::new();
        loop {
            let byte = self.byte()?;
            if byte == delimiter {
                break;
            }
            if byte == 0 || name.len() >= maximum {
                return Err(Error::invalid("invalid/oversized archive filename"));
            }
            name.try_reserve(1)
                .map_err(|_| Error::Limit("archive filename allocation"))?;
            name.push(byte);
        }
        if name.is_empty() {
            return Err(Error::invalid("empty archive filename"));
        }
        Ok(name)
    }
}
