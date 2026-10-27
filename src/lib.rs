use std::{fs, path::Path};

use zed_extension_api::{self as zed, settings::LspSettings, Result};

struct MarksmanExtension {
    cached_binary_path: Option<String>,
}

impl MarksmanExtension {
    fn binary_name(os: zed::Os) -> &'static str {
        if os == zed::Os::Windows {
            "marksman.exe"
        } else {
            "marksman"
        }
    }

    fn downloaded_binary_or_error(
        error: String,
        directory: &Path,
        binary_name: &str,
    ) -> Result<String> {
        fs::read_dir(directory)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("marksman-"))
            .filter_map(|entry| {
                let binary = entry.path().join(binary_name);
                binary.is_file().then_some(binary)
            })
            .max()
            .map(|path| path.to_string_lossy().into_owned())
            .ok_or_else(|| {
                format!(
                    "{error}: no existing Marksman binary found in {}",
                    directory.display()
                )
            })
    }

    fn download_and_install_binary(
        &self,
        language_server_id: &zed::LanguageServerId,
    ) -> Result<String> {
        zed::set_language_server_installation_status(
            language_server_id,
            &zed::LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let release = zed::latest_github_release(
            "artempyanykh/marksman",
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        let (platform, arch) = zed::current_platform();
        let asset_name = match (platform, arch) {
            (zed::Os::Linux, zed::Architecture::Aarch64) => "marksman-linux-arm64",
            (zed::Os::Linux, zed::Architecture::X8664) => "marksman-linux-x64",
            (zed::Os::Mac, _) => "marksman-macos",
            (zed::Os::Windows, _) => "marksman.exe",
            (unsupported_os, unsupported_arch) => {
                return Err(format!(
                    "Unsupported OS {:?} and architecture {:?} combination",
                    unsupported_os, unsupported_arch
                ));
            }
        };

        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("no asset found matching {:?}", asset_name))?;

        let version_dir = format!("marksman-{}", release.version);
        fs::create_dir_all(&version_dir).map_err(|e| format!("failed to create directory: {e}"))?;

        let binary_path = format!("{version_dir}/{}", Self::binary_name(platform));

        if !fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            zed::set_language_server_installation_status(
                language_server_id,
                &zed::LanguageServerInstallationStatus::Downloading,
            );

            zed::download_file(
                &asset.download_url,
                &binary_path,
                zed::DownloadedFileType::Uncompressed,
            )
            .map_err(|e| format!("failed to download file: {e}"))?;

            zed::make_file_executable(&binary_path)?;

            let version_dir_name = std::ffi::OsStr::new(&version_dir);
            fs::read_dir(".")
                .map_err(|e| format!("failed to list working directory {e}"))?
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .path()
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("marksman-"))
                })
                .filter(|entry| entry.path().file_name() != Some(version_dir_name))
                .for_each(|entry| {
                    fs::remove_dir_all(entry.path()).ok();
                });
        }

        Ok(binary_path)
    }

    fn language_server_binary_path(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        if let Some(path) = &self.cached_binary_path {
            if fs::metadata(path).is_ok_and(|stat| stat.is_file()) {
                return Ok(path.clone());
            }
        }

        let lsp_settings = LspSettings::for_worktree(language_server_id.as_ref(), worktree)?;

        if let Some(binary_settings) = lsp_settings.binary {
            let path = binary_settings.path.unwrap_or_default();
            self.cached_binary_path = Some(path.clone());
            return Ok(path);
        }

        if let Some(path) = worktree.which("marksman") {
            self.cached_binary_path = Some(path.clone());
            return Ok(path);
        }

        match self.download_and_install_binary(language_server_id) {
            Ok(binary_path) => {
                self.cached_binary_path = Some(binary_path.clone());
                Ok(binary_path)
            }
            Err(e) => {
                let (platform, _) = zed::current_platform();
                let path = Self::downloaded_binary_or_error(
                    e,
                    Path::new("."),
                    Self::binary_name(platform),
                )?;
                self.cached_binary_path = Some(path.clone());
                Ok(path)
            }
        }
    }
}

impl zed::Extension for MarksmanExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        Ok(zed::Command {
            command: self.language_server_binary_path(language_server_id, worktree)?,
            args: vec!["server".to_string()],
            env: Default::default(),
        })
    }
}

zed::register_extension!(MarksmanExtension);

#[cfg(test)]
mod tests {
    use super::*;
    use zed_extension_api::Extension;

    #[test]
    fn test_new_extension_initial_state() {
        let ext = MarksmanExtension::new();
        assert!(
            ext.cached_binary_path.is_none(),
            "A new extension instance should have no cached binary path by default."
        );
    }

    #[test]
    fn downloaded_binary_fallback_preserves_lookup_error_when_missing() {
        let directory = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "zed-marksman-fallback-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let version_dir = directory.join("marksman-1.0.0");
        fs::create_dir_all(&version_dir).unwrap();
        let unrelated_dir = directory.join("unrelated");
        fs::create_dir_all(&unrelated_dir).unwrap();
        fs::write(unrelated_dir.join("marksman"), b"not a downloaded binary").unwrap();

        let result = MarksmanExtension::downloaded_binary_or_error(
            "release lookup failed".into(),
            &directory,
            "marksman",
        );
        assert!(result.is_err(), "Expected error when no binary exists");
        assert!(
            result.unwrap_err().contains("release lookup failed"),
            "Error should contain original message"
        );

        let binary = version_dir.join("marksman");
        fs::write(&binary, b"marksman").unwrap();
        assert_eq!(
            MarksmanExtension::downloaded_binary_or_error(
                "release lookup failed".into(),
                &directory,
                "marksman",
            ),
            Ok(binary.to_string_lossy().into_owned())
        );

        let windows_binary = version_dir.join("marksman.exe");
        fs::write(&windows_binary, b"marksman").unwrap();
        assert_eq!(
            MarksmanExtension::downloaded_binary_or_error(
                "release lookup failed".into(),
                &directory,
                "marksman.exe",
            ),
            Ok(windows_binary.to_string_lossy().into_owned())
        );

        let newer = directory.join("marksman-2.0.0");
        fs::create_dir_all(&newer).unwrap();
        let newer_binary = newer.join("marksman");
        fs::write(&newer_binary, b"marksman").unwrap();
        assert_eq!(
            MarksmanExtension::downloaded_binary_or_error(
                "release lookup failed".into(),
                &directory,
                "marksman",
            ),
            Ok(newer_binary.to_string_lossy().into_owned())
        );

        fs::remove_dir_all(directory).unwrap();
    }
}
