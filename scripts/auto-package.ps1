param(
    [string]$Version = "0.1.0",
    [string]$TargetName = "windows-amd64",
    [string]$NodeDir = "",
    [string]$CoreTargetDir = "",
    [string]$DesktopTargetDir = "",
    [switch]$UseGlobalNode,
    [switch]$SkipInstall,
    [switch]$SkipTests,
    [switch]$SkipDesktop,
    [switch]$SkipArchive,
    [switch]$NoStop
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$ClientRoot = Join-Path $Root "client\agentlink-desktop"

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

function Resolve-TargetDir {
    param([string]$Value, [string]$Name)
    if ($Value) {
        return $Value
    }
    $envName = "AGENTLINK_$($Name.ToUpper())_TARGET_DIR"
    if ([Environment]::GetEnvironmentVariable($envName)) {
        return [Environment]::GetEnvironmentVariable($envName)
    }
    if (Test-Path "H:\") {
        return "H:\agentlink-$Name-target"
    }
    return Join-Path $Root "target\$Name"
}

function Get-NodeMajor {
    $version = (& node -v).Trim()
    if ($version -notmatch '^v(\d+)\.') {
        throw "Cannot parse node version: $version"
    }
    return [int]$Matches[1]
}

$CoreTargetDir = Resolve-TargetDir $CoreTargetDir "core"
$DesktopTargetDir = Resolve-TargetDir $DesktopTargetDir "desktop"
$Archive = Join-Path $Root "dist\agentlink-v$Version-$TargetName.zip"

if (-not $SkipDesktop) {
    if ($NodeDir -and -not $UseGlobalNode) {
        $nodeExe = Join-Path $NodeDir "node.exe"
        if (-not (Test-Path $nodeExe)) {
            throw "Node.js not found at $nodeExe. Pass -UseGlobalNode or use a valid -NodeDir."
        }
        $env:Path = "$NodeDir;$env:Path"
    }

    $nodeVersion = (& node -v).Trim()
    $npmVersion = (& npm -v).Trim()
    if ((Get-NodeMajor) -lt 18) {
        throw "Node.js $nodeVersion is too old. Use Node.js 18+."
    }
} else {
    $nodeVersion = "skipped"
    $npmVersion = "skipped"
}

Write-Host "AgentLink auto package"
Write-Host "  node:    $nodeVersion, npm $npmVersion"
Write-Host "  core:    $CoreTargetDir"
Write-Host "  desktop: $DesktopTargetDir"

if (-not $NoStop) {
    Get-Process -Name agentlink -ErrorAction SilentlyContinue | Stop-Process -Force
    Get-Process -Name agentlink-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
}

Push-Location $Root
try {
    $oldCargoTargetDir = $env:CARGO_TARGET_DIR
    $env:CARGO_TARGET_DIR = $CoreTargetDir
    try {
        if (-not $SkipTests) {
            Invoke-Checked cargo test
        }
        Invoke-Checked cargo build --release
    }
    finally {
        $env:CARGO_TARGET_DIR = $oldCargoTargetDir
    }
}
finally {
    Pop-Location
}

if (-not $SkipDesktop) {
    Push-Location $ClientRoot
    try {
        if (-not $SkipInstall) {
            Invoke-Checked npm ci
        }
        $oldCargoTargetDir = $env:CARGO_TARGET_DIR
        $env:CARGO_TARGET_DIR = $DesktopTargetDir
        try {
            if (-not $SkipTests) {
                Push-Location (Join-Path $ClientRoot "src-tauri")
                try {
                    Invoke-Checked cargo check
                }
                finally {
                    Pop-Location
                }
            }
            Invoke-Checked npm run build
        }
        finally {
            $env:CARGO_TARGET_DIR = $oldCargoTargetDir
        }
    }
    finally {
        Pop-Location
    }
}

if (-not $SkipArchive) {
    $oldCargoTargetDir = $env:CARGO_TARGET_DIR
    $oldDesktopTargetDir = $env:AGENTLINK_DESKTOP_TARGET_DIR
    $env:CARGO_TARGET_DIR = $CoreTargetDir
    $env:AGENTLINK_DESKTOP_TARGET_DIR = $DesktopTargetDir
    try {
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "package.ps1") -Version $Version -TargetName $TargetName -DesktopTargetDir $DesktopTargetDir -SkipTests
        if ($LASTEXITCODE -ne 0) {
            throw "package.ps1 failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        $env:CARGO_TARGET_DIR = $oldCargoTargetDir
        $env:AGENTLINK_DESKTOP_TARGET_DIR = $oldDesktopTargetDir
    }
}

Write-Host "Done."
if (Test-Path $Archive) {
    Write-Host "  archive: $Archive"
}
if (Test-Path (Join-Path $DesktopTargetDir "release\bundle")) {
    Write-Host "  desktop bundles: $(Join-Path $DesktopTargetDir "release\bundle")"
}
