use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::config::MAX_LINE_BYTES;

pub(super) const LOG_FILE: &str = "messenger.log";

/// Size-bounded rotating log file: at most `max_files` files of roughly
/// `max_file_bytes` each. One `write` call is one formatted event.
pub(super) struct RollingFile {
    dir: PathBuf,
    max_file_bytes: u64,
    max_files: usize,
    file: Option<File>,
    written: u64,
}

impl RollingFile {
    pub(super) fn open(dir: &Path, max_file_bytes: u64, max_files: usize) -> io::Result<Self> {
        ensure_private_dir(dir)?;
        let (file, written) = open_log(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            max_file_bytes,
            max_files: max_files.max(1),
            file: Some(file),
            written,
        })
    }

    fn rotated(&self, index: usize) -> PathBuf {
        self.dir.join(format!("{LOG_FILE}.{index}"))
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Close first: some platforms refuse to rename an open file.
        self.file = None;
        let active = self.dir.join(LOG_FILE);
        if self.max_files == 1 {
            fs::remove_file(&active)?;
        } else {
            let _ = fs::remove_file(self.rotated(self.max_files - 1));
            for i in (1..self.max_files - 1).rev() {
                let from = self.rotated(i);
                if from.exists() {
                    fs::rename(from, self.rotated(i + 1))?;
                }
            }
            fs::rename(&active, self.rotated(1))?;
        }
        self.reopen()
    }

    fn reopen(&mut self) -> io::Result<()> {
        let (file, written) = open_log(&self.dir)?;
        self.file = Some(file);
        self.written = written;
        Ok(())
    }
}

impl Write for RollingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = sanitize(buf);
        let len = line.len() as u64;
        if self.file.is_none() {
            self.reopen()?;
        }
        if self.written > 0 && self.written + len > self.max_file_bytes {
            self.rotate()?;
        }
        if let Some(file) = self.file.as_mut() {
            file.write_all(&line)?;
            self.written += len;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), Write::flush)
    }
}

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// One line: control and bidi characters neutralised, capped at
/// `MAX_LINE_BYTES`, ending in `\n`.
fn sanitize(buf: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(buf);
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let mut out = String::with_capacity(body.len() + 1);
    for c in body.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push('\t'),
            c if c.is_control() || is_bidi(c) => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    let mut end = out.len().min(MAX_LINE_BYTES - 1);
    while !out.is_char_boundary(end) {
        end -= 1;
    }
    out.truncate(end);
    out.push('\n');
    out.into_bytes()
}

fn refused(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what)
}

fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)?;
    // Checked after creation so a swap to a link is caught as well.
    if !fs::symlink_metadata(dir)?.file_type().is_dir() {
        return Err(refused("log directory is a link or not a directory"));
    }
    #[cfg(unix)]
    fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    Ok(())
}

fn open_log(dir: &Path) -> io::Result<(File, u64)> {
    let path = dir.join(LOG_FILE);
    match fs::symlink_metadata(&path) {
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(refused("log file is a link or not a regular file"));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let file = options.open(&path)?;
    let meta = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let now = fs::symlink_metadata(&path)?;
        if (meta.dev(), meta.ino()) != (now.dev(), now.ino()) {
            return Err(refused("log file changed while opening"));
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok((file, meta.len()))
}
