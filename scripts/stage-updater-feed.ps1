param(
    [string]$BundleDir = "H:\agentlink-desktop-target\release\bundle",
    [string]$FeedDir,
    [string]$BaseUrl
)

$ErrorActionPreference = "Stop"

if (-not $FeedDir) {
    throw "FeedDir is required."
}

if (-not $BaseUrl) {
    throw "BaseUrl is required."
}

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
& powershell -ExecutionPolicy Bypass -File (Join-Path $scriptRoot "validate-updater-artifacts.ps1") -BundleDir $BundleDir
if ($LASTEXITCODE -ne 0) {
    throw "validate-updater-artifacts.ps1 failed with exit code $LASTEXITCODE"
}

$bundleFiles = Get-ChildItem -LiteralPath $BundleDir -Recurse -File
$latest = $bundleFiles | Where-Object { $_.Name -eq "latest.json" } | Select-Object -First 1
$artifacts = $bundleFiles | Where-Object {
    $_.Extension -match '^\.(exe|msi|zip|AppImage|dmg)$' -and $_.Name -notmatch '\.sig$'
}
$signatures = $bundleFiles | Where-Object { $_.Extension -eq ".sig" }

if (-not $latest) {
    throw "latest.json was not found under $BundleDir"
}

if (-not $artifacts) {
    throw "No updater-compatible artifacts were found under $BundleDir"
}

if (Test-Path -LiteralPath $FeedDir) {
    Remove-Item -LiteralPath $FeedDir -Recurse -Force
}
$null = New-Item -ItemType Directory -Force -Path $FeedDir

$copiedNames = @{}
Copy-Item -LiteralPath $latest.FullName -Destination (Join-Path $FeedDir $latest.Name) -Force
foreach ($artifact in $artifacts) {
    Copy-Item -LiteralPath $artifact.FullName -Destination (Join-Path $FeedDir $artifact.Name) -Force
    $copiedNames[$artifact.Name] = $true
}
foreach ($signature in $signatures) {
    Copy-Item -LiteralPath $signature.FullName -Destination (Join-Path $FeedDir $signature.Name) -Force
}

$metadata = Get-Content -LiteralPath $latest.FullName -Raw | ConvertFrom-Json

function Update-Urls {
    param([Parameter(Mandatory = $true)] $Node)

    if ($null -eq $Node) {
        return
    }

    $properties = $Node.PSObject.Properties
    if ($properties.Count -gt 0) {
        foreach ($property in $properties) {
            if ($property.Name -eq "url" -and $property.Value -is [string]) {
                try {
                    $fileName = [System.IO.Path]::GetFileName(([Uri]$property.Value).AbsolutePath)
                }
                catch {
                    $fileName = [System.IO.Path]::GetFileName($property.Value)
                }

                if ($copiedNames.ContainsKey($fileName)) {
                    $property.Value = "{0}/{1}" -f $BaseUrl.TrimEnd('/'), $fileName
                }
            }
            elseif ($property.Value -isnot [string]) {
                Update-Urls -Node $property.Value
            }
        }
        return
    }

    if ($Node -is [System.Collections.IEnumerable] -and $Node -isnot [string]) {
        foreach ($item in $Node) {
            Update-Urls -Node $item
        }
    }
}

function Collect-Urls {
    param(
        [Parameter(Mandatory = $true)] $Node,
        [Parameter(Mandatory = $true)] [System.Collections.Generic.List[string]] $Urls
    )

    if ($null -eq $Node) {
        return
    }

    $properties = $Node.PSObject.Properties
    if ($properties.Count -gt 0) {
        foreach ($property in $properties) {
            if ($property.Name -eq "url" -and $property.Value -is [string]) {
                $Urls.Add($property.Value)
            }
            elseif ($property.Value -isnot [string]) {
                Collect-Urls -Node $property.Value -Urls $Urls
            }
        }
        return
    }

    if ($Node -is [System.Collections.IEnumerable] -and $Node -isnot [string]) {
        foreach ($item in $Node) {
            Collect-Urls -Node $item -Urls $Urls
        }
    }
}

Update-Urls -Node $metadata

$urls = [System.Collections.Generic.List[string]]::new()
Collect-Urls -Node $metadata -Urls $urls
foreach ($url in $urls) {
    try {
        $fileName = [System.IO.Path]::GetFileName(([Uri]$url).AbsolutePath)
    }
    catch {
        $fileName = [System.IO.Path]::GetFileName($url)
    }

    if (-not $copiedNames.ContainsKey($fileName)) {
        throw "Updater metadata references '$fileName', but it was not copied into $FeedDir"
    }
}

$outputLatest = Join-Path $FeedDir "latest.json"
$metadata | ConvertTo-Json -Depth 100 | Set-Content -LiteralPath $outputLatest -Encoding UTF8

Write-Host "Updater feed staged:"
Write-Host "  feed: $FeedDir"
Write-Host "  latest: $outputLatest"
Write-Host "  baseUrl: $BaseUrl"
