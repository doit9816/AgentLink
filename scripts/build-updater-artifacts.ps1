param(
    [string]$NodeDir = "F:\nvm\v18.20.8",
    [string]$CargoTargetDir = "H:\agentlink-desktop-target",
    [string]$FeedDir = "",
    [string]$BaseUrl = "",
    [switch]$SkipInstall,
    [switch]$UseGlobalNode
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$ClientRoot = Join-Path $Root "client\agentlink-desktop"
$BundleDir = Join-Path $CargoTargetDir "release\bundle"

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

if (-not $UseGlobalNode) {
    $nodeExe = Join-Path $NodeDir "node.exe"
    if (-not (Test-Path -LiteralPath $nodeExe)) {
        throw "Node.js not found at $nodeExe. Pass -UseGlobalNode or -NodeDir <node18+ path>."
    }
    $env:Path = "$NodeDir;$env:Path"
}

if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
    throw "TAURI_SIGNING_PRIVATE_KEY must be set to build updater artifacts."
}

$env:CARGO_TARGET_DIR = $CargoTargetDir

Push-Location $ClientRoot
try {
    if (-not $SkipInstall) {
        Invoke-Checked npm ci
    }
    Invoke-Checked npm run build:tauri
}
finally {
    Pop-Location
}

& powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "validate-updater-artifacts.ps1") -BundleDir $BundleDir
if ($LASTEXITCODE -ne 0) {
    throw "validate-updater-artifacts.ps1 failed with exit code $LASTEXITCODE"
}

if ($FeedDir -or $BaseUrl) {
    if (-not $FeedDir -or -not $BaseUrl) {
        throw "FeedDir and BaseUrl must be provided together when staging a public updater feed."
    }

    & powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "stage-updater-feed.ps1") -BundleDir $BundleDir -FeedDir $FeedDir -BaseUrl $BaseUrl
    if ($LASTEXITCODE -ne 0) {
        throw "stage-updater-feed.ps1 failed with exit code $LASTEXITCODE"
    }
}
