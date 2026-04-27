//! On-disk signature library at `$XDG_DATA_HOME/previewer/signatures/`.

use std::path::{Path, PathBuf};

use crate::{Signature, SignatureId};

const SIG_EXT: &str = "sig.json";

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// A directory of signature files.
pub struct Library {
    dir: PathBuf,
}

impl Library {
    /// Library backed by an explicit directory (used by tests).
    pub fn at(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    /// Default user library: `$XDG_DATA_HOME/previewer/signatures/` (or
    /// `~/.local/share/previewer/signatures/` if XDG_DATA_HOME is unset).
    pub fn default_user_library() -> Self {
        Self::at(default_library_dir())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save(&self, sig: &Signature) -> Result<(), LibraryError> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{}.{SIG_EXT}", sig.id.0));
        let json = serde_json::to_vec_pretty(sig)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    pub fn load_all(&self) -> Result<Vec<Signature>, LibraryError> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(SIG_EXT))
                .unwrap_or(false)
            {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            match serde_json::from_slice::<Signature>(&bytes) {
                Ok(sig) => out.push(sig),
                Err(e) => tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    "ignoring malformed signature file",
                ),
            }
        }
        // Sort by id so iteration order is stable across runs.
        out.sort_by_key(|s| s.id.0);
        Ok(out)
    }

    pub fn delete(&self, id: SignatureId) -> Result<(), LibraryError> {
        let path = self.dir.join(format!("{}.{SIG_EXT}", id.0));
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}

fn default_library_dir() -> PathBuf {
    let xdg = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    let base = xdg.unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".local/share")
    });
    base.join("previewer/signatures")
}
