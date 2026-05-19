param(
    [string]$BundleDir = "H:\agentlink-desktop-target\release\bundle"
)

$ErrorActionPreference = "Stop"

function Test-UpdaterArtifact {
    param(
        [Parameter(Mandatory = $true)]
        [System.IO.FileInfo]$File
    )

    $name = $File.Name
    return (
        $name -match '\.(exe|msi|zip|AppImage|dmg)$' -or
        $name -match '\.(tar\.gz)$'
    ) -and $name -notmatch '\.sig$'
}

if (-not (Test-Path -LiteralPath $BundleDir)) {
    throw "Bundle directory not found: $BundleDir"
}

$latest = Get-ChildItem -LiteralPath $BundleDir -Recurse -File -Filter "latest.json" | Select-Object -First 1
if (-not $latest) {
    throw "latest.json was not found under $BundleDir"
}

$metadata = Get-Content -LiteralPath $latest.FullName -Raw | ConvertFrom-Json
$metadataText = Get-Content -LiteralPath $latest.FullName -Raw
if ($metadataText -notmatch '"signature"\s*:') {
    throw "latest.json does not include updater signature information: $($latest.FullName)"
}

$bundleFiles = Get-ChildItem -LiteralPath $BundleDir -Recurse -File
$installer = $bundleFiles | Where-Object { Test-UpdaterArtifact -File $_ } | Select-Object -First 1
if (-not $installer) {
    throw "No updater-compatible installer or bundle was found under $BundleDir"
}

$signature = $bundleFiles | Where-Object { $_.Name -like "$($installer.Name).sig" -or $_.Extension -eq ".sig" } | Select-Object -First 1
if (-not $signature) {
    throw "No updater signature file was found for bundle artifacts under $BundleDir"
}

Write-Host "Updater metadata validated:"
Write-Host "  latest: $($latest.FullName)"
Write-Host "  bundle: $($installer.FullName)"
Write-Host "  signature: $($signature.FullName)"
