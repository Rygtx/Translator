use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;

#[derive(Debug, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub download_url: String,
    pub changelog: String,
}

#[tauri::command]
pub async fn download_and_install_update(
    app: AppHandle,
    version: String,
    download_url: String,
) -> Result<(), String> {
    let temp_dir = std::env::temp_dir();
    let zip_path = temp_dir.join(format!("MoonTranslator-{}.zip", version));

    let response = reqwest::get(&download_url)
        .await
        .map_err(|e| format!("Failed to download update: {}", e))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read update data: {}", e))?;

    std::fs::write(&zip_path, bytes)
        .map_err(|e| format!("Failed to save update file: {}", e))?;

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe path: {}", e))?;
    let app_dir = current_exe
        .parent()
        .ok_or("Failed to get app directory")?
        .to_path_buf();

    let script_path = temp_dir.join("update_moontranslator.ps1");
    let exe_name = current_exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("MoonTranslator.exe")
        .to_string();

    let script_content = format!(
        r#"
Clear-Host
$host.UI.RawUI.WindowTitle = "MoonTranslator Update"

$supportsRGB = $PSVersionTable.PSVersion.Major -ge 6

if ($supportsRGB) {{
    $InfoPrefix = "`e[38;2;200;162;200m"
    $SepPrefix = "`e[38;2;150;150;150m"
    $ErrorPrefix = "`e[38;2;255;120;120m"
    $WarnPrefix = "`e[38;2;255;200;100m"
    $ResetColor = "`e[0m"
}} else {{
    $InfoColor = [System.ConsoleColor]::Magenta
    $SepColor = [System.ConsoleColor]::DarkGray
    $ErrorColor = [System.ConsoleColor]::Red
    $WarnColor = [System.ConsoleColor]::Yellow
}}

function Write-Log {{
    param([string]$Level, [string]$Message)
    if ($supportsRGB) {{
        switch ($Level) {{
            "INFO"  {{ Write-Host "$($InfoPrefix)INFO$($ResetColor)$($SepPrefix) | $($ResetColor)$Message" }}
            "WARN"  {{ Write-Host "$($WarnPrefix)WARN$($ResetColor)$($SepPrefix) | $($ResetColor)$Message" }}
            default {{ Write-Host "$($ErrorPrefix)ERROR$($ResetColor)$($SepPrefix) | $($ResetColor)$Message" }}
        }}
    }} else {{
        switch ($Level) {{
            "INFO"  {{ Write-Host "INFO" -ForegroundColor $InfoColor -NoNewline }}
            "WARN"  {{ Write-Host "WARN" -ForegroundColor $WarnColor -NoNewline }}
            default {{ Write-Host "ERROR" -ForegroundColor $ErrorColor -NoNewline }}
        }}
        Write-Host " | " -ForegroundColor $SepColor -NoNewline
        Write-Host $Message
    }}
}}

$zipPath = "{zip}"
$appDir = "{app}"
$exeName = "{exe}"

try {{
    Write-Log "INFO" "Waiting for MoonTranslator to close..."

    $timeout = 30
    $elapsed = 0
    while ($elapsed -lt $timeout) {{
        $procs = Get-Process -Name ($exeName -replace '\.exe$','') -ErrorAction SilentlyContinue
        if (-not $procs) {{ break }}
        Start-Sleep -Milliseconds 500
        $elapsed += 0.5
        if ($elapsed % 5 -eq 0) {{
            Write-Log "WARN" "Still waiting for process to exit... ($elapsed s)"
        }}
    }}

    if ($elapsed -ge $timeout) {{
        Write-Log "WARN" "Process did not exit in $timeout seconds, attempting to kill..."
        Stop-Process -Name ($exeName -replace '\.exe$','') -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }} else {{
        Write-Log "INFO" "Process exited after $elapsed seconds"
        Start-Sleep -Seconds 1
    }}

    Write-Log "INFO" "Extracting update package to temp directory..."

    $extractFolder = Join-Path $env:TEMP "MoonTranslator_Update_$(Get-Date -Format 'yyyyMMddHHmmss')"
    Expand-Archive -Path $zipPath -DestinationPath $extractFolder -Force

    Write-Log "INFO" "Extraction completed successfully"

    # Handle nested directory: if the zip contains a single top-level folder, use its contents
    $sourceDir = $extractFolder
    $topItems = Get-ChildItem -Path $extractFolder
    if ($topItems.Count -eq 1 -and $topItems[0].PSIsContainer) {{
        $nested = $topItems[0].FullName
        $nestedExe = Join-Path $nested $exeName
        if (Test-Path $nestedExe) {{
            Write-Log "INFO" "Detected nested zip structure, using: $($topItems[0].Name)"
            $sourceDir = $nested
        }}
    }}

    Write-Log "INFO" "Verifying extracted files..."

    $exePath = Join-Path $sourceDir $exeName
    $altExePath = Join-Path $sourceDir "moon-translator.exe"
    $altExePath2 = Join-Path $sourceDir "MoonTranslator.exe"

    if (Test-Path $exePath) {{
        Write-Log "INFO" "Found $exeName in extracted files"
    }} elseif (($exeName -ne "MoonTranslator.exe") -and (Test-Path $altExePath2)) {{
        Write-Log "INFO" "Found MoonTranslator.exe in extracted files"
        $exeName = "MoonTranslator.exe"
    }} elseif (Test-Path $altExePath) {{
        Write-Log "INFO" "Found moon-translator.exe, renaming to MoonTranslator.exe"
        Rename-Item -Path $altExePath -NewName "MoonTranslator.exe"
        $exeName = "MoonTranslator.exe"
    }} else {{
        $extractedFiles = Get-ChildItem -Path $sourceDir -Recurse -File | Select-Object -First 10
        Write-Log "ERROR" "$exeName not found in extracted files"
        Write-Log "ERROR" "Files found: $($extractedFiles.Name -join ', ')"
        throw "$exeName not found in extracted files"
    }}

    Write-Log "INFO" "Installing new version to: $appDir"

    # Copy with retry for locked files
    $maxRetries = 3
    for ($attempt = 1; $attempt -le $maxRetries; $attempt++) {{
        try {{
            Copy-Item -Path "$sourceDir\*" -Destination $appDir -Recurse -Force -ErrorAction Stop
            Write-Log "INFO" "Installation completed successfully"
            break
        }} catch {{
            if ($attempt -lt $maxRetries) {{
                Write-Log "WARN" "Copy attempt $attempt failed (files may still be locked), retrying in 3 seconds..."
                Start-Sleep -Seconds 3
            }} else {{
                throw "Failed to copy files after $maxRetries attempts: $_"
            }}
        }}
    }}

    # Verify the exe was actually updated
    $installedExe = Join-Path $appDir $exeName
    if (-not (Test-Path $installedExe)) {{
        throw "Verification failed: $exeName not found in $appDir after copy"
    }}
    Write-Log "INFO" "Verified: $exeName exists in install directory"

    Write-Log "INFO" "Cleaning up temporary files..."
    Remove-Item -Path $extractFolder -Recurse -Force -ErrorAction SilentlyContinue

    Write-Log "INFO" "Removing downloaded zip file..."
    Remove-Item -Path $zipPath -Force -ErrorAction SilentlyContinue

    Write-Log "INFO" "Restarting MoonTranslator..."
    Start-Sleep -Seconds 1
    Start-Process -FilePath $installedExe

    Write-Log "INFO" "Update completed successfully!"
    Start-Sleep -Seconds 2
}} catch {{
    Write-Host ""
    Write-Log "ERROR" "Update failed: $_"
    Write-Host ""
    Write-Host "Press any key to close..." -ForegroundColor Gray
    pause
}}
"#,
        zip = zip_path.display().to_string().replace("\\", "\\\\"),
        app = app_dir.display().to_string().replace("\\", "\\\\"),
        exe = exe_name,
    );

    std::fs::write(&script_path, script_content)
        .map_err(|e| format!("Failed to create update script: {}", e))?;

    std::process::Command::new("powershell")
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script_path)
        .spawn()
        .map_err(|e| format!("Failed to start update process: {}", e))?;

    app.exit(0);

    Ok(())
}
