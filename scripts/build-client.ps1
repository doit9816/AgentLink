param(
    [string]$NodeDir = "F:\nvm\v18.20.8",
    [string]$CargoTargetDir = "H:\agentlink-desktop-target",
    [string]$BridgeCargoTargetDir = "H:\agentlink-target",
    [switch]$SkipInstall,
    [switch]$SkipTests,
    [switch]$SkipPackage,
    [switch]$NoStop,
    [switch]$Launch,
    [switch]$UseGlobalNode
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$ClientRoot = Join-Path $Root "client\agentlink-desktop"
$TauriRoot = Join-Path $ClientRoot "src-tauri"
$TauriManifest = Join-Path $TauriRoot "Cargo.toml"
$ReleaseExe = Join-Path $CargoTargetDir "release\agentlink-desktop.exe"

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

function Get-NodeMajor {
    $version = (& node -v).Trim()
    if ($version -notmatch '^v(\d+)\.') {
        throw "Cannot parse node version: $version"
    }
    return [int]$Matches[1]
}

if (-not $UseGlobalNode) {
    $nodeExe = Join-Path $NodeDir "node.exe"
    if (-not (Test-Path $nodeExe)) {
        throw "Node.js not found at $nodeExe. Pass -UseGlobalNode or -NodeDir <node18+ path>."
    }
    $env:Path = "$NodeDir;$env:Path"
}

$nodeVersion = (& node -v).Trim()
$npmVersion = (& npm -v).Trim()
if ((Get-NodeMajor) -lt 18) {
    throw "Node.js $nodeVersion is too old. Use Node.js 18+."
}
Write-Host "Using node $nodeVersion, npm $npmVersion"

if (-not $NoStop) {
    Get-Process -Name agentlink-desktop -ErrorAction SilentlyContinue | Stop-Process -Force
}

Push-Location $ClientRoot
try {
    if (-not $SkipInstall) {
        if (-not (Test-Path (Join-Path $ClientRoot "node_modules"))) {
            Invoke-Checked npm ci
        }
    }

    Invoke-Checked npm run build:ui

    if (-not $SkipTests) {
        Push-Location $TauriRoot
        try {
            Invoke-Checked cargo check
        }
        finally {
            Pop-Location
        }
    }

    $env:CARGO_TARGET_DIR = $CargoTargetDir
    Invoke-Checked cargo build --release --features tauri/custom-protocol --manifest-path $TauriManifest

    Write-Host "AgentLink Desktop built: $ReleaseExe"
}
finally {
    Pop-Location
}

if (-not $SkipPackage) {
    $oldCargoTargetDir = $env:CARGO_TARGET_DIR
    $env:CARGO_TARGET_DIR = $BridgeCargoTargetDir
    try {
        & powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "package.ps1") -SkipTests
        if ($LASTEXITCODE -ne 0) {
            throw "package.ps1 failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        $env:CARGO_TARGET_DIR = $oldCargoTargetDir
    }
}

if ($Launch) {
    Start-Process $ReleaseExe
    Write-Host "AgentLink Desktop started: $ReleaseExe"
}
