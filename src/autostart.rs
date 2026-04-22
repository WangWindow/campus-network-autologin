use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use directories::BaseDirs;

#[cfg(target_os = "linux")]
const LINUX_SERVICE_NAME: &str = "campus-network-autologin.service";
#[cfg(target_os = "macos")]
const MACOS_AGENT_ID: &str = "com.campus-network-autologin";

pub fn show_autostart_path() -> Result<PathBuf> {
    platform_autostart_path()
}

pub fn autostart_enabled() -> Result<bool> {
    Ok(platform_autostart_path()?.exists())
}

pub fn install_autostart() -> Result<PathBuf> {
    let target_path = platform_autostart_path()?;
    let exe_path = executable_path()?;
    let content = platform_autostart_content(&exe_path);

    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create startup directory {}", parent.display()))?;
    }
    fs::write(&target_path, content)
        .with_context(|| format!("failed to write startup file {}", target_path.display()))?;

    Ok(target_path)
}

pub fn remove_autostart() -> Result<PathBuf> {
    let target_path = platform_autostart_path()?;
    if !target_path.exists() {
        bail!("autostart file does not exist: {}", target_path.display());
    }
    fs::remove_file(&target_path)
        .with_context(|| format!("failed to remove startup file {}", target_path.display()))?;
    Ok(target_path)
}

fn executable_path() -> Result<PathBuf> {
    let current_exe =
        std::env::current_exe().context("failed to resolve current executable path")?;
    let resolved_exe = current_exe
        .canonicalize()
        .unwrap_or_else(|_| current_exe.clone());

    if resolved_exe.exists() {
        Ok(resolved_exe)
    } else if current_exe.exists() {
        Ok(current_exe)
    } else {
        bail!(
            "executable path is not available: {}",
            resolved_exe.display()
        )
    }
}

#[cfg(windows)]
fn platform_autostart_path() -> Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA")
        .ok_or_else(|| anyhow!("APPDATA is not available in this environment"))?;
    Ok(PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join("campus-network-autologin.vbs"))
}

#[cfg(target_os = "linux")]
fn platform_autostart_path() -> Result<PathBuf> {
    let base_dirs =
        BaseDirs::new().ok_or_else(|| anyhow!("could not resolve the current user home path"))?;
    Ok(base_dirs
        .home_dir()
        .join(".config")
        .join("systemd")
        .join("user")
        .join(LINUX_SERVICE_NAME))
}

#[cfg(target_os = "macos")]
fn platform_autostart_path() -> Result<PathBuf> {
    let base_dirs =
        BaseDirs::new().ok_or_else(|| anyhow!("could not resolve the current user home path"))?;
    Ok(base_dirs
        .home_dir()
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{MACOS_AGENT_ID}.plist")))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn platform_autostart_path() -> Result<PathBuf> {
    bail!("autostart is not implemented for this platform")
}

#[cfg(windows)]
fn platform_autostart_content(exe_path: &Path) -> String {
    let escaped = exe_path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\"\"");
    format!(
        "Set ws = CreateObject(\"WScript.Shell\")\r\nws.Run \"\"\"{escaped}\"\" run\", 0, False\r\n"
    )
}

#[cfg(target_os = "linux")]
fn platform_autostart_content(exe_path: &Path) -> String {
    let exec = exe_path.to_string_lossy().replace('"', "\\\"");
    format!(
        "[Unit]\nDescription=Campus network auto-login\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart=\"{exec}\" run\nRestart=always\nRestartSec=10\n\n[Install]\nWantedBy=default.target\n"
    )
}

#[cfg(target_os = "macos")]
fn platform_autostart_content(exe_path: &Path) -> String {
    let exec = xml_escape(&exe_path.to_string_lossy());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{MACOS_AGENT_ID}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{exec}</string>\n    <string>run</string>\n  </array>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <true/>\n</dict>\n</plist>\n"
    )
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn platform_autostart_content(_exe_path: &Path) -> String {
    String::new()
}

#[cfg(target_os = "macos")]
fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
