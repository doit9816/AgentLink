param(
    [string]$BundleDir = "H:\agentlink-desktop-target\release\bundle",
    [string]$PlatformKey
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
            return $BundleFiles | Where-Object {
                $_.Name -match '\.exe$' -or $_.Name -match '\.msi$' -or $_.Name -match '\.(nsis\.zip|msi\.zip)$'
            }
        }
        '^linux-' {
            return $BundleFiles | Where-Object {
                $_.Name -match '\.AppImage$' -or $_.Name -match '\.AppImage\.tar\.gz$'
            }
        }
        '^darwin-' {
            return $BundleFiles | Where-Object {
                $_.Name -match '\.app\.tar\.gz$' -or $_.Name -match '\.dmg$'
            }
        }
        default {
            return $BundleFiles | Where-Object {
                $_.Name -match '\.(exe|msi|zip|AppImage|dmg)$' -or $_.Name -match '\.(tar\.gz)$'
            }
        }
    }
}

if (-not $PlatformKey) {
    throw "PlatformKey is required."
}

if (-not (Test-Path -LiteralPath $BundleDir)) {
    throw "Bundle directory not found: $BundleDir"
}

$bundleFiles = Get-ChildItem -LiteralPath $BundleDir -Recurse -File
$artifacts = Get-UpdaterArtifacts -BundleFiles $bundleFiles -PlatformKey $PlatformKey
if (-not $artifacts) {
    throw "No updater-compatible artifacts were found under $BundleDir for $PlatformKey"
}

$installer = $artifacts | Sort-Object Name | Select-Object -First 1
if (-not $installer) {
    throw "No updater-compatible installer or bundle was found under $BundleDir"
}

$signature = $bundleFiles | Where-Object { $_.Name -eq "$($installer.Name).sig" } | Select-Object -First 1
if (-not $signature) {
    throw "No updater signature file was found for artifact '$($installer.Name)' under $BundleDir"
}

Write-Host "Updater metadata validated:"
Write-Host "  platform: $PlatformKey"
Write-Host "  bundle: $($installer.FullName)"
Write-Host "  signature: $($signature.FullName)"
