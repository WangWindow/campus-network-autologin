use std::{
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use directories::BaseDirs;

#[cfg(target_os = "linux")]
const LINUX_SERVICE_NAME: &str = "campus-network-autologin.service";
#[cfg(target_os = "macos")]
const MACOS_AGENT_ID: &str = "com.campus-network-autologin";
#[cfg(windows)]
const WINDOWS_TASK_NAME: &str = "campus-network-autologin";

pub fn show_autostart_path() -> Result<PathBuf> {
    platform_autostart_path()
}

pub fn autostart_enabled() -> Result<bool> {
    #[cfg(windows)]
    {
        return windows_task_exists();
    }
    #[cfg(not(windows))]
    {
        Ok(platform_autostart_path()?.exists())
    }
}

pub fn install_autostart() -> Result<PathBuf> {
    let exe_path = executable_path()?;

    #[cfg(windows)]
    {
        install_windows_task(&exe_path)?;
        return platform_autostart_path();
    }

    #[cfg(not(windows))]
    {
        let target_path = platform_autostart_path()?;
        let content = platform_autostart_content(&exe_path);

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create startup directory {}", parent.display())
            })?;
        }
        fs::write(&target_path, content)
            .with_context(|| format!("failed to write startup file {}", target_path.display()))?;
        Ok(target_path)
    }
}

pub fn remove_autostart() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        remove_windows_task()?;
        return platform_autostart_path();
    }

    #[cfg(not(windows))]
    {
        let target_path = platform_autostart_path()?;
        if !target_path.exists() {
            bail!("autostart file does not exist: {}", target_path.display());
        }
        fs::remove_file(&target_path)
            .with_context(|| format!("failed to remove startup file {}", target_path.display()))?;
        Ok(target_path)
    }
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
fn install_windows_task(exe_path: &Path) -> Result<()> {
    let run_action = format!("\"{}\" run", exe_path.display());
    let status = std::process::Command::new("schtasks")
        .args([
            "/Create",
            "/TN",
            WINDOWS_TASK_NAME,
            "/SC",
            "ONLOGON",
            "/DELAY",
            "0000:20",
            "/TR",
            &run_action,
            "/RL",
            "LIMITED",
            "/F",
        ])
        .status()
        .context("failed to run schtasks /Create")?;

    if status.success() {
        Ok(())
    } else {
        bail!("failed to create scheduled task for autostart")
    }
}

#[cfg(windows)]
fn remove_windows_task() -> Result<()> {
    if !windows_task_exists()? {
        bail!("autostart task does not exist: {WINDOWS_TASK_NAME}");
    }

    let status = std::process::Command::new("schtasks")
        .args(["/Delete", "/TN", WINDOWS_TASK_NAME, "/F"])
        .status()
        .context("failed to run schtasks /Delete")?;
    if status.success() {
        Ok(())
    } else {
        bail!("failed to remove scheduled task autostart entry")
    }
}

#[cfg(windows)]
fn windows_task_exists() -> Result<bool> {
    let status = std::process::Command::new("schtasks")
        .args(["/Query", "/TN", WINDOWS_TASK_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("failed to run schtasks /Query")?;
    Ok(status.success())
}

#[cfg(windows)]
fn platform_autostart_path() -> Result<PathBuf> {
    Ok(PathBuf::from(format!(
        "Task Scheduler\\{}",
        WINDOWS_TASK_NAME
    )))
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

#[cfg(target_os = "macos")]
fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
