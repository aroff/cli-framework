use super::ProjectConfigError;

/// Result of a successful upward file search.
#[derive(Debug, Clone)]
pub struct ProjectRoot {
    /// The directory that contains the config file.
    pub root_dir: std::path::PathBuf,
    /// Absolute path to the config file: always equal to `root_dir.join(file_name)`.
    pub config_file: std::path::PathBuf,
}

/// Options for [`find_file_upward_with_options`].
#[derive(Debug, Clone, Default)]
pub struct DiscoverOptions {
    /// Maximum number of parent steps to take from `start`. `None` means unlimited.
    pub max_depth: Option<usize>,
    /// Stop searching when a directory entry with this name is found in the current dir.
    /// When the marker is encountered and the config file is not in that directory,
    /// returns `Err(ProjectConfigError::NotFound)`.
    pub stop_at_marker: Option<String>,
}

/// Walk parent directories from `start`, returning the first directory that
/// contains a file named exactly `file_name`.
///
/// `file_name` MUST be a bare filename with no path separators.
pub fn find_file_upward(
    start: &std::path::Path,
    file_name: &str,
) -> Result<ProjectRoot, ProjectConfigError> {
    find_file_upward_with_options(start, file_name, DiscoverOptions::default())
}

/// Walk parent directories with additional `options`.
pub fn find_file_upward_with_options(
    start: &std::path::Path,
    file_name: &str,
    options: DiscoverOptions,
) -> Result<ProjectRoot, ProjectConfigError> {
    if file_name.contains('/') || file_name.contains('\\') {
        return Err(ProjectConfigError::InvalidFileName {
            file_name: file_name.to_string(),
        });
    }

    let mut current = start.to_path_buf();
    let mut depth: usize = 0;

    loop {
        let candidate = current.join(file_name);
        if candidate.is_file() {
            return Ok(ProjectRoot {
                root_dir: current,
                config_file: candidate,
            });
        }

        if let Some(marker) = &options.stop_at_marker {
            if current.join(marker).exists() {
                return Err(ProjectConfigError::NotFound {
                    file_name: file_name.to_string(),
                    start: start.to_path_buf(),
                });
            }
        }

        match current.parent() {
            None => {
                return Err(ProjectConfigError::NotFound {
                    file_name: file_name.to_string(),
                    start: start.to_path_buf(),
                });
            }
            Some(parent) => {
                depth += 1;
                if let Some(max) = options.max_depth {
                    if depth > max {
                        return Err(ProjectConfigError::DepthExceeded {
                            max_depth: max,
                            start: start.to_path_buf(),
                        });
                    }
                }
                current = parent.to_path_buf();
            }
        }
    }
}
