param(
    [string]$BundleDir = "H:\agentlink-desktop-target\release\bundle",
    [string]$FeedDir,
    [string]$BaseUrl,
    [string]$PlatformKey,
    [string]$Version
)

$ErrorActionPreference = "Stop"

function Get-UpdaterArtifacts {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.FileInfo[]]$BundleFiles,
        [string]$PlatformKey
    )

    switch -Regex ($PlatformKey) {
        '^windows-' {
            $candidates = $BundleFiles | Where-Object { $_.Name -match '\.exe$' -or $_.Name -match '\.msi$' -or $_.Name -match '\.(nsis\.zip|msi\.zip)$' }
            return $candidates | Sort-Object @{ Expression = { if ($_.Name -match '\.exe$') { 0 } elseif ($_.Name -match '\.msi$') { 1 } else { 2 } } }, Name
        }
        '^linux-' {
            $candidates = $BundleFiles | Where-Object { $_.Name -match '\.AppImage$' -or $_.Name -match '\.AppImage\.tar\.gz$' }
            return $candidates | Sort-Object @{ Expression = { if ($_.Name -match '\.AppImage$') { 0 } else { 1 } } }, Name
        }
        '^darwin-' {
            $candidates = $BundleFiles | Where-Object { $_.Name -match '\.app\.tar\.gz$' -or $_.Name -match '\.dmg$' }
            return $candidates | Sort-Object @{ Expression = { if ($_.Name -match '\.app\.tar\.gz$') { 0 } else { 1 } } }, Name
        }
        default {
            return $BundleFiles | Where-Object {
                $_.Name -match '\.(exe|msi|zip|AppImage|dmg)$' -or $_.Name -match '\.(tar\.gz)$'
            } | Sort-Object Name
        }
    }
}

if (-not $FeedDir) {
    throw "FeedDir is required."
}

if (-not $BaseUrl) {
    throw "BaseUrl is required."
}

if (-not $PlatformKey) {
    throw "PlatformKey is required."
}

if (-not $Version) {
    if ($env:GITHUB_REF_NAME) {
        $Version = $env:GITHUB_REF_NAME
    }
    else {
        throw "Version is required."
    }
}

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$validateScript = Join-Path $scriptRoot "validate-updater-artifacts.ps1"
& $validateScript -BundleDir $BundleDir -PlatformKey $PlatformKey

$bundleFiles = Get-ChildItem -LiteralPath $BundleDir -Recurse -File
$artifacts = Get-UpdaterArtifacts -BundleFiles $bundleFiles -PlatformKey $PlatformKey
$artifact = $artifacts | Select-Object -First 1
$signature = $bundleFiles | Where-Object { $_.Name -eq "$($artifact.Name).sig" } | Select-Object -First 1

if (-not $artifact) {
    throw "No updater-compatible artifacts were found under $BundleDir for $PlatformKey"
}

if (-not $signature) {
    throw "No signature was found for updater artifact '$($artifact.Name)'"
}

if (Test-Path -LiteralPath $FeedDir) {
    Remove-Item -LiteralPath $FeedDir -Recurse -Force
}
$null = New-Item -ItemType Directory -Force -Path $FeedDir

Copy-Item -LiteralPath $artifact.FullName -Destination (Join-Path $FeedDir $artifact.Name) -Force
Copy-Item -LiteralPath $signature.FullName -Destination (Join-Path $FeedDir $signature.Name) -Force

$signatureText = (Get-Content -LiteralPath $signature.FullName -Raw).Trim()
$artifactUrl = "{0}/{1}" -f $BaseUrl.TrimEnd('/'), $artifact.Name

$metadata = [ordered]@{
    version = $Version
    pub_date = [DateTime]::UtcNow.ToString("o")
    platforms = [ordered]@{
        $PlatformKey = [ordered]@{
            signature = $signatureText
            url = $artifactUrl
        }
    }
}

$outputLatest = Join-Path $FeedDir "latest.json"
$metadata | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $outputLatest -Encoding UTF8

Write-Host "Updater feed staged:"
Write-Host "  feed: $FeedDir"
Write-Host "  latest: $outputLatest"
Write-Host "  platform: $PlatformKey"
Write-Host "  version: $Version"
Write-Host "  artifact: $($artifact.Name)"
Write-Host "  baseUrl: $BaseUrl"
