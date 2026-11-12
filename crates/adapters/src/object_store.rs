use anyhow::{Result, ensure};
use bank_controls::evidence::BlobStore;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub struct FilesystemStore {
    root: PathBuf,
}
impl FilesystemStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        let root = fs::canonicalize(root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { root })
    }
    fn open(&self, path: &Path, write: bool) -> Result<File> {
        let mut options = OpenOptions::new();
        if write {
            options.write(true).create_new(true);
        } else {
            options.read(true);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
        }
        Ok(options.open(path)?)
    }
}
impl BlobStore for FilesystemStore {
    fn write_new(&self, id: Uuid, data: &[u8]) -> Result<()> {
        let temporary = self.root.join(format!(".tmp-{id}"));
        let final_path = self.root.join(format!("{id}.bin"));
        let result = (|| {
            let mut file = self.open(&temporary, true)?;
            file.write_all(data)?;
            file.sync_all()?;
            fs::hard_link(&temporary, &final_path)?; // Refuse replacement of an existing version.
            fs::remove_file(&temporary)?;
            File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    fn read_bounded(&self, id: Uuid, max: usize) -> Result<Vec<u8>> {
        ensure!(max <= 8_388_608, "Object read bound exceeded");
        let file = self.open(&self.root.join(format!("{id}.bin")), false)?;
        ensure!(file.metadata()?.is_file(), "Object is not a regular file");
        let mut data = Vec::new();
        file.take(max as u64 + 1).read_to_end(&mut data)?;
        ensure!(data.len() <= max, "Object exceeds recorded size");
        Ok(data)
    }
    fn ids(&self) -> Result<Vec<Uuid>> {
        Ok(fs::read_dir(&self.root)?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.strip_suffix(".bin"))
                    .and_then(|id| Uuid::parse_str(id).ok())
            })
            .collect())
    }
}
