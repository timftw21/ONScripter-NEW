use std::io::{self, Read, Seek, SeekFrom};

use crate::{
    Error, Limits, Result,
    archive::{ArchiveIndex, ArchiveKind},
    read_limited,
};

pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// Platform storage supplies seekable handles, including Android providers.
pub trait Storage {
    fn open(&self, name: &str) -> io::Result<Option<Box<dyn ReadSeek>>>;
}

pub struct Asset {
    pub source: String,
    pub length: u64,
    pub reader: Box<dyn ReadSeek>,
}

impl Asset {
    pub fn read(mut self, maximum: usize) -> Result<Vec<u8>> {
        if self.length > maximum as u64 {
            return Err(Error::Limit("asset size"));
        }
        let bytes = read_limited(&mut self.reader, maximum)?;
        if bytes.len() as u64 != self.length {
            return Err(Error::invalid("asset changed size while reading"));
        }
        Ok(bytes)
    }
}

pub struct AssetStore<S> {
    pub storage: S,
    pub limits: Limits,
    archives: Vec<(String, ArchiveIndex)>,
}

impl<S: Storage> AssetStore<S> {
    pub fn new(storage: S, limits: Limits) -> Self {
        Self {
            storage,
            limits,
            archives: Vec::new(),
        }
    }

    pub fn archives(&self) -> impl Iterator<Item = (&str, &ArchiveIndex)> {
        self.archives
            .iter()
            .map(|(name, index)| (name.as_str(), index))
    }

    pub fn mount_archives(&mut self, directory: &str, nsa_offset: u64) -> Result<()> {
        let directory = if directory.is_empty() {
            String::new()
        } else {
            format!("{}/", directory.trim_end_matches(['/', '\\']))
        };
        let mut archives = Vec::new();
        let mut remaining = self.limits.index_bytes;
        let mut mount = |name: String, kind, offset| -> Result<bool> {
            let Some(mut reader) = self.storage.open(&name)? else {
                return Ok(false);
            };
            let mut limits = self.limits;
            limits.index_bytes = remaining;
            let index = ArchiveIndex::read(&mut reader, kind, offset, limits)?;
            remaining -= index.index_bytes;
            archives.push((name, index));
            Ok(true)
        };
        // Preserve RU precedence: loose files, descending NS2, ascending NSA, SAR.
        if self.storage.open(&format!("{directory}00.ns2"))?.is_some() {
            for number in (0..100).rev() {
                mount(format!("{directory}{number:02}.ns2"), ArchiveKind::Ns2, 0)?;
            }
        }
        if mount(format!("{directory}arc.nsa"), ArchiveKind::Nsa, nsa_offset)? {
            for number in 1..=9 {
                if !mount(
                    format!("{directory}arc{number}.nsa"),
                    ArchiveKind::Nsa,
                    nsa_offset,
                )? {
                    break;
                }
            }
        }
        mount(format!("{directory}arc.sar"), ArchiveKind::Sar, 0)?;
        self.archives = archives;
        Ok(())
    }

    pub fn open(&self, name: &str) -> Result<Option<Asset>> {
        if let Some(mut reader) = self.storage.open(name)? {
            let length = reader.seek(SeekFrom::End(0))?;
            reader.seek(SeekFrom::Start(0))?;
            return Ok(Some(Asset {
                source: "loose file".into(),
                length,
                reader,
            }));
        }
        for (archive_name, index) in &self.archives {
            let Some(entry) = index.get(name) else {
                continue;
            };
            if entry.compression != 0 {
                return Err(Error::invalid(format!(
                    "{name}: NSA compression {} is not implemented",
                    entry.compression
                )));
            }
            let reader = self
                .storage
                .open(archive_name)?
                .ok_or_else(|| Error::invalid("mounted archive disappeared"))?;
            let reader = AssetRange::new(reader, entry.offset, entry.length)?;
            return Ok(Some(Asset {
                source: archive_name.clone(),
                length: entry.length,
                reader: Box::new(reader),
            }));
        }
        Ok(None)
    }
}

/// Seeking an asset never exposes the adjacent archive entries.
pub struct AssetRange<R> {
    reader: R,
    start: u64,
    length: u64,
    position: u64,
}

impl<R: Read + Seek> AssetRange<R> {
    pub fn new(mut reader: R, start: u64, length: u64) -> io::Result<Self> {
        let size = reader.seek(SeekFrom::End(0))?;
        if start.checked_add(length).is_none_or(|end| end > size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "asset range exceeds its file",
            ));
        }
        reader.seek(SeekFrom::Start(start))?;
        Ok(Self {
            reader,
            start,
            length,
            position: 0,
        })
    }
}

impl<R: Read> Read for AssetRange<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = (self.length - self.position).min(buffer.len() as u64) as usize;
        let count = self.reader.read(&mut buffer[..count])?;
        self.position += count as u64;
        Ok(count)
    }
}

impl<R: Seek> Seek for AssetRange<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => i128::from(self.length) + i128::from(n),
        };
        if position < 0 || position > i128::from(self.length) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside asset",
            ));
        }
        self.reader
            .seek(SeekFrom::Start(self.start + position as u64))?;
        self.position = position as u64;
        Ok(self.position)
    }
}
