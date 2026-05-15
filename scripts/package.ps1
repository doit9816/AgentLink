param(
    [string]$Version = "0.1.0",
    [string]$TargetName = "windows-amd64",
    [string]$DesktopTargetDir = "",
    [switch]$SkipDesktopArtifacts,
    [switch]$SkipTests
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Dist = Join-Path $Root "dist"
$Stage = Join-Path $Dist "agentlink-v$Version-$TargetName"
$Zip = "$Stage.zip"
$CargoTargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root "target" }
$ReleaseExe = Join-Path $CargoTargetDir "release\agentlink.exe"
$ResolvedDesktopTargetDir = if ($DesktopTargetDir) {
    $DesktopTargetDir
} elseif ($env:AGENTLINK_DESKTOP_TARGET_DIR) {
    $env:AGENTLINK_DESKTOP_TARGET_DIR
} else {
    Join-Path $Root "client\agentlink-desktop\src-tauri\target"
}

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

Push-Location $Root
try {
    if (-not $SkipTests) {
        Invoke-Checked cargo test
    }

    Invoke-Checked cargo build --release

    if (Test-Path $Stage) {
        Remove-Item -LiteralPath $Stage -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $Stage | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Stage "examples") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Stage "docs") | Out-Null
    New-Item -ItemType Directory -Force -Path (Join-Path $Stage "client") | Out-Null

    Copy-Item -LiteralPath $ReleaseExe -Destination $Stage
    Copy-Item -LiteralPath (Join-Path $Root "README.zh-CN.md") -Destination $Stage
    Get-ChildItem -LiteralPath (Join-Path $Root "examples") | Copy-Item -Destination (Join-Path $Stage "examples") -Recurse -Force
    Copy-Item -LiteralPath (Join-Path $Root "docs\agentlink-protocol.zh-CN.md") -Destination (Join-Path $Stage "docs")

    if (-not $SkipDesktopArtifacts) {
        $DesktopExe = Join-Path $ResolvedDesktopTargetDir "release\agentlink-desktop.exe"
        $DesktopBundle = Join-Path $ResolvedDesktopTargetDir "release\bundle"
        $DesktopStage = Join-Path $Stage "desktop"
        if ((Test-Path $DesktopExe) -or (Test-Path $DesktopBundle)) {
            New-Item -ItemType Directory -Force -Path $DesktopStage | Out-Null
            if (Test-Path $DesktopExe) {
                Copy-Item -LiteralPath $DesktopExe -Destination $DesktopStage
            }
            if (Test-Path $DesktopBundle) {
                Copy-Item -LiteralPath $DesktopBundle -Destination (Join-Path $DesktopStage "bundle") -Recurse -Force
            }
        }
    }

    $ClientSrc = Join-Path $Root "client\agentlink-desktop"
    $ClientDst = Join-Path $Stage "client\agentlink-desktop"
    New-Item -ItemType Directory -Force -Path $ClientDst | Out-Null
    & robocopy $ClientSrc $ClientDst /E /XD node_modules target dist data /XF *.log *.sqlite3 *.sqlite3-shm *.sqlite3-wal /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) {
        throw "robocopy client files failed with exit code $LASTEXITCODE"
    }

    if (Test-Path $Zip) {
        Remove-Item -LiteralPath $Zip -Force
    }
    Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $Zip
    Write-Host "Package written: $Zip"
}
finally {
    Pop-Location
}
