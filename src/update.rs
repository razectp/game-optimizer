//! Check GitHub Releases for a newer Game Optimizer build.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const RELEASES_LATEST: &str = "https://api.github.com/repos/razectp/game-optimizer/releases/latest";
const USER_AGENT: &str = concat!("GameOptimizer/", env!("CARGO_PKG_VERSION"));

/// A newer GitHub Release the user can install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Installed crate version (`0.5.3`).
    pub current: String,
    /// Latest release version without a leading `v`.
    pub latest: String,
    /// Release page on GitHub.
    pub html_url: String,
    /// Direct URL of `GameOptimizer-*-setup.exe` when present.
    pub setup_url: Option<String>,
}

/// Compare dotted versions (`1.2.3`); missing parts are treated as 0.
pub fn version_is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

fn parse_version(raw: &str) -> [u64; 3] {
    let trimmed = raw.trim().trim_start_matches('v').trim_start_matches('V');
    let mut parts = [0u64; 3];
    for (i, piece) in trimmed.split('.').take(3).enumerate() {
        let digits: String = piece.chars().take_while(|c| c.is_ascii_digit()).collect();
        parts[i] = digits.parse().unwrap_or(0);
    }
    parts
}

fn normalize_tag(tag: &str) -> String {
    tag.trim()
        .trim_start_matches('v')
        .trim_start_matches('V')
        .to_string()
}

fn curl_get(url: &str) -> anyhow::Result<String> {
    let output = Command::new(curl_bin())
        .args([
            "-sS",
            "-L",
            "--max-time",
            "20",
            "-A",
            USER_AGENT,
            "-H",
            "Accept: application/vnd.github+json",
            url,
        ])
        .output()
        .map_err(|err| anyhow::anyhow!("curl failed to start: {err}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("curl exited {}: {err}", output.status);
    }
    String::from_utf8(output.stdout)
        .map_err(|err| anyhow::anyhow!("curl output was not UTF-8: {err}"))
}

fn curl_bin() -> &'static str {
    if cfg!(windows) {
        "curl.exe"
    } else {
        "curl"
    }
}

fn json_string_after(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let idx = body.find(&needle)?;
    let after = body[idx + needle.len()..].trim_start();
    let after = after.strip_prefix(':')?.trim_start();
    parse_json_string(after)
}

fn parse_json_string(after_colon: &str) -> Option<String> {
    let rest = after_colon.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16).ok()?;
                    out.push(char::from_u32(code)?);
                }
                other => out.push(other),
            },
            other => out.push(other),
        }
    }
    None
}

fn is_setup_asset(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("-setup.exe") || lower.ends_with("/setup.exe")
}

fn find_setup_url(body: &str) -> Option<String> {
    let mut rest = body;
    let needle = "\"browser_download_url\"";
    while let Some(idx) = rest.find(needle) {
        let after = rest[idx + needle.len()..].trim_start();
        let after = after.strip_prefix(':')?.trim_start();
        if let Some(url) = parse_json_string(after) {
            if is_setup_asset(&url) {
                return Some(url);
            }
        }
        rest = &rest[idx + needle.len()..];
    }
    None
}

/// Query GitHub for a newer release. `Ok(None)` means this build is current.
pub fn check_latest() -> anyhow::Result<Option<UpdateInfo>> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let body = curl_get(RELEASES_LATEST)?;
    let tag = json_string_after(&body, "tag_name")
        .ok_or_else(|| anyhow::anyhow!("GitHub response missing tag_name"))?;
    let html_url = json_string_after(&body, "html_url")
        .ok_or_else(|| anyhow::anyhow!("GitHub response missing html_url"))?;
    let latest = normalize_tag(&tag);
    if !version_is_newer(&latest, &current) {
        return Ok(None);
    }
    Ok(Some(UpdateInfo {
        current,
        latest,
        html_url,
        setup_url: find_setup_url(&body),
    }))
}

/// Download the setup installer to a temp file.
pub fn download_setup(url: &str) -> anyhow::Result<PathBuf> {
    let path = env::temp_dir().join("GameOptimizer-update-setup.exe");
    let output = Command::new(curl_bin())
        .args([
            "-sS",
            "-L",
            "--max-time",
            "120",
            "-A",
            USER_AGENT,
            "-o",
            path.to_str()
                .ok_or_else(|| anyhow::anyhow!("temp path is not UTF-8"))?,
            url,
        ])
        .output()
        .map_err(|err| anyhow::anyhow!("curl failed to start: {err}"))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("download failed: {err}");
    }
    if !path.is_file() {
        anyhow::bail!("download did not create {}", path.display());
    }
    let header = std::fs::read(&path).unwrap_or_default();
    if header.len() < 2 || header[0] != b'M' || header[1] != b'Z' {
        let _ = std::fs::remove_file(&path);
        anyhow::bail!("downloaded file is not a Windows installer");
    }
    Ok(path)
}

/// Launch the downloaded Inno installer (closes this app via CloseApplications).
pub fn launch_setup(path: &Path) -> anyhow::Result<()> {
    Command::new(path)
        .spawn()
        .map_err(|err| anyhow::anyhow!("could not start installer: {err}"))?;
    Ok(())
}

/// Open the GitHub release page in the default browser (Windows).
pub fn open_release_page(url: &str) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|err| anyhow::anyhow!("could not open browser: {err}"))?;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let _ = url;
        anyhow::bail!("opening a browser is only supported on Windows");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_patch_is_detected() {
        assert!(version_is_newer("0.5.1", "0.5.0"));
        assert!(version_is_newer("v0.6.0", "0.5.1"));
        assert!(!version_is_newer("0.5.0", "0.5.0"));
        assert!(!version_is_newer("0.4.9", "0.5.0"));
    }

    #[test]
    fn strips_v_prefix() {
        assert_eq!(normalize_tag("v0.5.1"), "0.5.1");
        assert_eq!(parse_version("v1.2"), [1, 2, 0]);
    }

    #[test]
    fn parses_github_release_json() {
        let body = r#"{
            "tag_name": "v0.9.0",
            "html_url": "https://github.com/razectp/game-optimizer/releases/tag/v0.9.0",
            "assets": [
              {"name": "GameOptimizer-0.9.0.exe", "browser_download_url": "https://example.com/GameOptimizer-0.9.0.exe"},
              {"name": "GameOptimizer-0.9.0-setup.exe", "browser_download_url": "https://github.com/razectp/game-optimizer/releases/download/v0.9.0/GameOptimizer-0.9.0-setup.exe"}
            ]
        }"#;
        assert_eq!(
            json_string_after(body, "tag_name").as_deref(),
            Some("v0.9.0")
        );
        assert_eq!(
            find_setup_url(body).as_deref(),
            Some("https://github.com/razectp/game-optimizer/releases/download/v0.9.0/GameOptimizer-0.9.0-setup.exe")
        );
    }
}
