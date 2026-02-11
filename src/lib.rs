use std::fs;

use zed_extension_api::{self as zed, settings::LspSettings, Result};

struct MarksmanExtension {
    cached_binary_path: Option<String>,
    current_version: Option<String>,
    previous_version_path: Option<String>,
}

impl MarksmanExtension {
    fn download_and_install_binary(
        &self,
        language_server_id: &zed::LanguageServerId,
    ) -> Result<(String, String)> {
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

        let binary_path = format!("{version_dir}/marksman");

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

        Ok((binary_path, release.version))
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

        // Save current binary as fallback before attempting update
        if let Some(current_path) = &self.cached_binary_path {
            if fs::metadata(current_path).is_ok_and(|stat| stat.is_file()) {
                self.previous_version_path = Some(current_path.clone());
            }
        }

        match self.download_and_install_binary(language_server_id) {
            Ok((binary_path, version)) => {
                self.cached_binary_path = Some(binary_path.clone());
                self.current_version = Some(version);
                Ok(binary_path)
            }
            Err(e) => {
                if let Some(previous_path) = &self.previous_version_path {
                    if fs::metadata(previous_path).is_ok_and(|stat| stat.is_file()) {
                        let path = previous_path.clone();
                        self.cached_binary_path = Some(path.clone());
                        return Ok(path);
                    }
                }
                Err(e)
            }
        }
    }
}

impl zed::Extension for MarksmanExtension {
    fn new() -> Self {
        Self {
            cached_binary_path: None,
            current_version: None,
            previous_version_path: None,
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
        assert!(
            ext.current_version.is_none(),
            "A new extension instance should have no current version by default."
        );
        assert!(
            ext.previous_version_path.is_none(),
            "A new extension instance should have no previous version path by default."
        );
    }
}
