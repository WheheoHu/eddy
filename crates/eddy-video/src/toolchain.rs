use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use url::Url;

const VIDEO_FILE_EXTENSIONS: &[&str] = &["mp4", "mov", "mkv", "webm", "m4v", "avi"];

const VIDEO_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "youtube-nocookie.com",
    "bilibili.com",
    "b23.tv",
    "vimeo.com",
    "twitch.tv",
    "ted.com",
    "loom.com",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainStatus {
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
    pub yt_dlp_path: Option<String>,
    pub ffmpeg_version: Option<String>,
    pub yt_dlp_version: Option<String>,
}

impl ToolchainStatus {
    pub fn can_process_local_files(&self) -> bool {
        self.ffmpeg_path.is_some() && self.ffprobe_path.is_some()
    }

    pub fn can_process_remote_urls(&self) -> bool {
        self.can_process_local_files() && self.yt_dlp_path.is_some()
    }
}

/// Checks whether a local file path has a supported video extension.
pub fn is_video_file_path(path: &str) -> bool {
    Path::new(path.trim())
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| VIDEO_FILE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Checks whether an HTTP(S) URL points to a known video hosting platform or direct video file.
pub fn is_video_url(raw: &str) -> bool {
    let Ok(parsed) = Url::parse(raw.trim()) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if VIDEO_HOSTS
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
    {
        return true;
    }
    is_video_file_path(parsed.path())
}

/// Resolves an executable by checking an explicit override first, then standard
/// macOS Homebrew / Linux / user binary directories, and finally `PATH`.
pub fn resolve_binary(name: &str, custom_override: Option<&str>) -> Option<PathBuf> {
    if let Some(custom) = custom_override.map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }

    let mut search_dirs: Vec<PathBuf> = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];
    if let Ok(home) = std::env::var("HOME") {
        search_dirs.insert(0, PathBuf::from(&home).join(".local/bin"));
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for entry in std::env::split_paths(&path_var) {
            if !search_dirs.contains(&entry) {
                search_dirs.push(entry);
            }
        }
    }

    for dir in search_dirs {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn read_version_line(bin: &Path, arg: &str) -> Option<String> {
    let output = Command::new(bin).arg(arg).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .next()
        .map(|l| l.trim().chars().take(80).collect::<String>())
        .filter(|s| !s.is_empty())
}

/// Builds an augmented `PATH` for child media processes (`yt-dlp`, `ffmpeg`) so GUI apps
/// launched by macOS `launchd` (whose default `PATH` is only `/usr/bin:/bin`) still find
/// Homebrew `deno`, `ffmpeg`, and `node` runtimes needed for YouTube JS challenge solving.
pub fn augmented_child_path(extra_bin_dirs: &[&Path]) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for extra in extra_bin_dirs {
        if !extra.as_os_str().is_empty() && !dirs.iter().any(|d| d == extra) {
            dirs.push(extra.to_path_buf());
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let home_p = PathBuf::from(&home);
        for rel in [".deno/bin", ".local/bin", ".cargo/bin"] {
            let p = home_p.join(rel);
            if p.is_dir() && !dirs.contains(&p) {
                dirs.push(p);
            }
        }
        // Include latest nvm node bin if installed
        let nvm_versions = home_p.join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(nvm_versions) {
            let mut versions: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path().join("bin"))
                .filter(|p| p.is_dir())
                .collect();
            versions.sort();
            if let Some(latest) = versions.pop()
                && !dirs.contains(&latest)
            {
                dirs.push(latest);
            }
        }
    }
    for std_dir in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        let p = PathBuf::from(std_dir);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    if let Some(existing) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&existing) {
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}
/// Probes local availability and versions of `ffmpeg`, `ffprobe`, and `yt-dlp`.
pub fn detect_toolchain(
    ffmpeg_override: Option<&str>,
    yt_dlp_override: Option<&str>,
) -> ToolchainStatus {
    let ffmpeg = resolve_binary("ffmpeg", ffmpeg_override);
    let ffprobe_hint = ffmpeg
        .as_ref()
        .and_then(|p| p.parent().map(|dir| dir.join("ffprobe")))
        .filter(|p| p.is_file());
    let ffprobe = ffprobe_hint.or_else(|| resolve_binary("ffprobe", None));
    let yt_dlp = resolve_binary("yt-dlp", yt_dlp_override);

    let ffmpeg_version = ffmpeg
        .as_deref()
        .and_then(|p| read_version_line(p, "-version"));
    let yt_dlp_version = yt_dlp
        .as_deref()
        .and_then(|p| read_version_line(p, "--version"));

    ToolchainStatus {
        ffmpeg_path: ffmpeg.map(|p| p.to_string_lossy().into_owned()),
        ffprobe_path: ffprobe.map(|p| p.to_string_lossy().into_owned()),
        yt_dlp_path: yt_dlp.map(|p| p.to_string_lossy().into_owned()),
        ffmpeg_version,
        yt_dlp_version,
    }
}
