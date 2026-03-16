// Platform-specific service integration — launchd on macOS, systemd on Linux.

/// Generate and install the OS-native service configuration.
pub fn install_service() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        install_launchd()?;
    }
    #[cfg(target_os = "linux")]
    {
        install_systemd()?;
    }
    Ok(())
}

/// Remove the OS-native service configuration.
pub fn uninstall_service() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        uninstall_launchd()?;
    }
    #[cfg(target_os = "linux")]
    {
        uninstall_systemd()?;
    }
    Ok(())
}

// ── macOS: launchd ────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn launchd_plist_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Library/LaunchAgents/com.cronymax.service.plist")
}

#[cfg(target_os = "macos")]
fn install_launchd() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.cronymax.service</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>--service</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/cronymax-service.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/cronymax-service.err</string>
</dict>
</plist>"#,
        exe.display()
    );

    let plist_path = launchd_plist_path();
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plist_path, plist_content)?;

    // Load the agent.
    let status = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()?;
    if !status.success() {
        anyhow::bail!("launchctl load failed with status {}", status);
    }

    log::info!("Installed launchd agent at {}", plist_path.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall_launchd() -> anyhow::Result<()> {
    let plist_path = launchd_plist_path();
    if plist_path.exists() {
        let _ = std::process::Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(&plist_path)
            .status();
        std::fs::remove_file(&plist_path)?;
        log::info!("Removed launchd agent");
    }
    Ok(())
}

// ── Linux: systemd ────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn systemd_unit_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".config/systemd/user/cronymax.service")
}

#[cfg(target_os = "linux")]
fn install_systemd() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let unit_content = format!(
        r#"[Unit]
Description=cronymax Background Service
After=default.target

[Service]
Type=simple
ExecStart={} --service
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#,
        exe.display()
    );

    let unit_path = systemd_unit_path();
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&unit_path, unit_content)?;

    // Enable and start the service.
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "cronymax.service"])
        .status()?;
    if !status.success() {
        anyhow::bail!("systemctl enable failed with status {}", status);
    }

    log::info!("Installed systemd user service at {}", unit_path.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall_systemd() -> anyhow::Result<()> {
    let unit_path = systemd_unit_path();
    if unit_path.exists() {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "--now", "cronymax.service"])
            .status();
        std::fs::remove_file(&unit_path)?;
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
        log::info!("Removed systemd user service");
    }
    Ok(())
}
