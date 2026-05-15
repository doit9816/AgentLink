param(
    [string]$NodeDir = "F:\nvm\v18.20.8",
    [string]$ClientCargoTargetDir = "H:\agentlink-desktop-target",
    [string]$BridgeCargoTargetDir = "H:\agentlink-target",
    [switch]$UseGlobalNode,
    [switch]$SkipInstall,
    [switch]$SkipBuild,
    [switch]$CheckOnly,
    [switch]$NoStop,
    [switch]$CheckBridge,
    [switch]$BuildBridge,
    [switch]$SmokeWeixinQr,
    [bool]$LaunchClient = $true
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$ClientRoot = Join-Path $Root "client\agentlink-desktop"
$TauriRoot = Join-Path $ClientRoot "src-tauri"
$TauriManifest = Join-Path $TauriRoot "Cargo.toml"
$ClientExe = Join-Path $ClientCargoTargetDir "release\agentlink-desktop.exe"
$BridgeExe = Join-Path $BridgeCargoTargetDir "release\agentlink.exe"

function Write-Step {
    param([string]$Message)
    Write-Host ""
    Write-Host "==> $Message" -ForegroundColor Cyan
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

function Get-NodeMajor {
    $version = (& node -v).Trim()
    if ($version -notmatch '^v(\d+)\.') {
        throw "Cannot parse node version: $version"
    }
    return [int]$Matches[1]
}

Write-Step "Prepare Node.js"
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
Write-Host "Node: $nodeVersion"
Write-Host "npm : $npmVersion"

if (-not $NoStop) {
    Write-Step "Stop running local processes"
    Get-Process -Name agentlink-desktop,agentlink -ErrorAction SilentlyContinue | Stop-Process -Force
}

Write-Step "Build Vue UI"
Push-Location $ClientRoot
try {
    if (-not $SkipInstall -and -not (Test-Path (Join-Path $ClientRoot "node_modules"))) {
        Invoke-Checked npm ci
    }
    Invoke-Checked npm run build:ui
}
finally {
    Pop-Location
}

Write-Step "Check Tauri client"
$oldCargoTargetDir = $env:CARGO_TARGET_DIR
$env:CARGO_TARGET_DIR = $ClientCargoTargetDir
try {
    Push-Location $TauriRoot
    try {
        Invoke-Checked cargo check
    }
    finally {
        Pop-Location
    }

    if (-not $CheckOnly -and -not $SkipBuild) {
        Write-Step "Build AgentLink Desktop release"
        Invoke-Checked cargo build --release --features tauri/custom-protocol --manifest-path $TauriManifest
        Write-Host "AgentLink Desktop exe: $ClientExe" -ForegroundColor Green
    }
}
finally {
    $env:CARGO_TARGET_DIR = $oldCargoTargetDir
}

if ($CheckBridge -or $BuildBridge) {
    $oldCargoTargetDir = $env:CARGO_TARGET_DIR
    $env:CARGO_TARGET_DIR = $BridgeCargoTargetDir
    try {
        Push-Location $Root
        try {
            Write-Step "Check AgentLink core"
            Invoke-Checked cargo check

            if ($BuildBridge -and -not $CheckOnly -and -not $SkipBuild) {
                Write-Step "Build AgentLink core release"
                Invoke-Checked cargo build --release
                Write-Host "Bridge exe: $BridgeExe" -ForegroundColor Green
            }
        }
        finally {
            Pop-Location
        }
    }
    finally {
        $env:CARGO_TARGET_DIR = $oldCargoTargetDir
    }
}

if ($SmokeWeixinQr) {
    Write-Step "Smoke test Weixin ilink QR endpoint"
    $url = "https://ilinkai.weixin.qq.com/ilink/bot/get_bot_qrcode?bot_type=3"
    $result = Invoke-RestMethod -Uri $url -Method Get -TimeoutSec 30
    if (-not $result.qrcode -or -not $result.qrcode_img_content) {
        throw "Weixin QR endpoint returned incomplete response: $($result | ConvertTo-Json -Depth 5)"
    }
    Write-Host "qrcode: $($result.qrcode)"
    Write-Host "qr url : $($result.qrcode_img_content)"
}

if ($LaunchClient -and -not $CheckOnly) {
    if (-not (Test-Path $ClientExe)) {
        throw "Client exe not found: $ClientExe. Re-run without -SkipBuild."
    }
    Write-Step "Launch AgentLink Desktop"
    Start-Process $ClientExe
    Write-Host "Started: $ClientExe" -ForegroundColor Green
}

Write-Step "Done"
Write-Host "Manual checks:"
Write-Host "1. Channel -> Weixin -> Scan bind, QR URL should be liteapp.weixin.qq.com."
Write-Host "2. Switch to another Channel, old QR panel should disappear."
Write-Host "3. After Weixin scan succeeds, send one WeChat message, then click Refresh discovered targets."
