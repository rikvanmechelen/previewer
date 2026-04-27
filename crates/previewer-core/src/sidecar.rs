use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::AnnotationLayer;

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Build the sidecar JSON path for an image path: appends `.previewer.json`
/// to the *full* image path (so `foo.png` → `foo.png.previewer.json`,
/// keeping the original extension distinguishable).
pub fn sidecar_path(image_path: &Path) -> PathBuf {
    let mut s = OsString::from(image_path.as_os_str());
    s.push(".previewer.json");
    PathBuf::from(s)
}

pub fn save_layer(layer: &AnnotationLayer, path: impl AsRef<Path>) -> Result<(), SidecarError> {
    let json = serde_json::to_vec_pretty(layer)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load_layer(path: impl AsRef<Path>) -> Result<AnnotationLayer, SidecarError> {
    let bytes = std::fs::read(path)?;
    let layer = serde_json::from_slice(&bytes)?;
    Ok(layer)
}
