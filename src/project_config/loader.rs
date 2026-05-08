use super::{
    discover::{find_file_upward_with_options, DiscoverOptions, ProjectRoot},
    ProjectConfigError,
};

/// Read and deserialize a TOML file into `T`.
pub fn load_toml_file<T: serde::de::DeserializeOwned>(
    path: &std::path::Path,
) -> Result<T, ProjectConfigError> {
    let content = std::fs::read_to_string(path).map_err(|e| ProjectConfigError::IoError {
        path: path.to_path_buf(),
        source: e,
    })?;
    load_toml_str(&content, path)
}

/// Deserialize a TOML string into `T` (no I/O; for testing).
pub fn load_toml_str<T: serde::de::DeserializeOwned>(
    content: &str,
    path_hint: &std::path::Path,
) -> Result<T, ProjectConfigError> {
    toml::from_str::<T>(content).map_err(|e| ProjectConfigError::ParseError {
        path: path_hint.to_path_buf(),
        source: e,
    })
}

/// Convenience: discover config file upward then load it.
///
/// Returns the deserialized config and the [`ProjectRoot`] so callers
/// know which directory was chosen as the project root.
pub fn find_and_load<T: serde::de::DeserializeOwned>(
    start: &std::path::Path,
    file_name: &str,
) -> Result<(T, ProjectRoot), ProjectConfigError> {
    find_and_load_with_options(start, file_name, DiscoverOptions::default())
}

/// Same as [`find_and_load`] but accepts [`DiscoverOptions`].
pub fn find_and_load_with_options<T: serde::de::DeserializeOwned>(
    start: &std::path::Path,
    file_name: &str,
    options: DiscoverOptions,
) -> Result<(T, ProjectRoot), ProjectConfigError> {
    let root = find_file_upward_with_options(start, file_name, options)?;
    let config = load_toml_file::<T>(&root.config_file)?;
    Ok((config, root))
}
