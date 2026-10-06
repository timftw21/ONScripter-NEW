//! Native services behind the core's platform-independent interfaces.

use cap_std::{ambient_authority, fs::Dir};
use onscripter_core::assets::{ReadSeek, Storage};
use std::{io, path::Path};

pub struct FileStorage {
    roots: Vec<Dir>,
}

impl FileStorage {
    /// Earlier roots override later roots. Callers select these roots explicitly.
    pub fn new(roots: impl IntoIterator<Item = impl AsRef<Path>>) -> io::Result<Self> {
        let roots = roots
            .into_iter()
            .map(|root| Dir::open_ambient_dir(root, ambient_authority()))
            .collect::<io::Result<Vec<_>>>()?;
        if roots.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no game directories selected",
            ));
        }
        Ok(Self { roots })
    }
}

impl Storage for FileStorage {
    fn open(&self, name: &str) -> io::Result<Option<Box<dyn ReadSeek>>> {
        let name = name.replace('\\', "/");
        if name.is_empty()
            || name.starts_with('/')
            || name.contains(['\0', ':'])
            || name.split('/').any(|part| part == "..")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "game filename must be relative to a selected root",
            ));
        }
        for root in &self.roots {
            match root.open(&name) {
                Ok(file) => {
                    if !file.metadata()?.is_file() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "asset is not a regular file",
                        ));
                    }
                    return Ok(Some(Box::new(file.into_std())));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }
}
