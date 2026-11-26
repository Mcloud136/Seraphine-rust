use std::path::PathBuf;

/// Resolves the directories the application reads and writes.
///
/// The legacy Python app stays usable, so the Rust app keeps its own mutable
/// state under `LOCALAPPDATA\Seraphine\app` while reusing read-only caches.
#[derive(Clone, Debug)]
pub struct AppPaths {
    pub legacy_data: PathBuf,
    pub local: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub catalog: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Self {
        let roaming = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let local = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let legacy_data = roaming.join("Seraphine");
        let local_root = local.join("Seraphine");
        Self {
            state: local_root.join("app"),
            cache: local_root.join("Cache"),
            catalog: local_root.clone(),
            local: local_root,
            legacy_data,
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.state.join("config.json")
    }

    pub fn legacy_config_file(&self) -> PathBuf {
        self.legacy_data.join("config.json")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.state.join("log")
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.state)?;
        std::fs::create_dir_all(self.log_dir())?;
        Ok(())
    }
}

impl Default for AppPaths {
    fn default() -> Self {
        Self::discover()
    }
}
