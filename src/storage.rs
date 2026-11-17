use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Durable byte-stream abstraction for the write-ahead log.
pub trait Storage: Send {
    /// Reads the entire log from start to end.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the underlying medium cannot be read.
    fn read_all(&self) -> io::Result<Vec<u8>>;

    /// Appends `bytes` to the end of the log.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the write fails.
    fn append(&mut self, bytes: &[u8]) -> io::Result<()>;

    /// Flushes buffered writes to durable storage.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if syncing fails.
    fn sync(&mut self) -> io::Result<()>;

    /// Atomically replaces the entire log content with `bytes` and syncs it.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if replacing or syncing fails.
    fn replace(&mut self, bytes: &[u8]) -> io::Result<()>;
}

/// In-memory [`Storage`] implementation backed by an `Arc<Mutex<Vec<u8>>>` for testing and
/// crash-recovery simulation.
#[derive(Debug, Clone, Default)]
pub struct MemStorage {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl MemStorage {
    /// Creates an empty in-memory storage handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a snapshot copy of the current byte buffer.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        self.buf
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Truncates the buffer to `len` bytes (if currently longer), simulating a crash mid-write.
    pub fn crash_at(&self, len: usize) {
        let mut guard = self
            .buf
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if len < guard.len() {
            guard.truncate(len);
        }
    }

    /// Overwrites the byte at `index` with `val` (used in tests to inject bit flips).
    pub fn corrupt_byte(&self, index: usize, val: u8) {
        let mut guard = self
            .buf
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if index < guard.len() {
            guard[index] = val;
        }
    }
}

impl Storage for MemStorage {
    fn read_all(&self) -> io::Result<Vec<u8>> {
        Ok(self.bytes())
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.buf
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(())
    }

    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn replace(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut guard = self
            .buf
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.clear();
        guard.extend_from_slice(bytes);
        Ok(())
    }
}

/// File-backed [`Storage`] implementation that writes to disk and uses atomic temp-file rename on
/// [`Storage::replace`].
#[derive(Debug)]
pub struct FileStorage {
    path: PathBuf,
    file: File,
}

impl FileStorage {
    /// Opens or creates a log file at `path`.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the file or parent directory cannot be accessed.
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)?;
        Ok(Self { path, file })
    }

    /// Returns the path of the underlying log file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Storage for FileStorage {
    fn read_all(&self) -> io::Result<Vec<u8>> {
        let mut reader = File::open(&self.path)?;
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)
    }

    fn sync(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }

    fn replace(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut tmp_name = self
            .path
            .file_name()
            .map_or_else(|| std::ffi::OsString::from("wal"), ToOwned::to_owned);
        tmp_name.push(".tmp");
        let tmp_path = self.path.with_file_name(tmp_name);

        {
            let mut tmp_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)?;
            tmp_file.write_all(bytes)?;
            tmp_file.sync_all()?;
        }

        fs::rename(&tmp_path, &self.path)?;
        self.file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&self.path)?;
        Ok(())
    }
}
