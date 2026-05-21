param(
    [Parameter(Mandatory = $true)]
    [string]$ArtifactsRoot,
    [Parameter(Mandatory = $true)]
    [string]$FeedRoot,
    [string]$TagName = $env:GITHUB_REF_NAME
)

$ErrorActionPreference = "Stop"

if (-not $TagName) {
    throw "TagName is required (or set GITHUB_REF_NAME)."
}

$artifactsRoot = Resolve-Path -LiteralPath $ArtifactsRoot
$feedRoot = Join-Path (Get-Location) $FeedRoot

if (Test-Path -LiteralPath $feedRoot) {
    Remove-Item -LiteralPath $feedRoot -Recurse -Force
}
$null = New-Item -ItemType Directory -Force -Path $feedRoot

Get-ChildItem -LiteralPath $artifactsRoot -Directory | ForEach-Object {
    if ($_.Name -notmatch '^updater-feed-(.+)-(.+)$') {
        throw "Unexpected updater artifact directory name: $($_.Name)"
    }

    $target = $Matches[1]
    $arch = $Matches[2]
    $destinationDir = Join-Path $feedRoot $target
    $destinationDir = Join-Path $destinationDir $arch

    New-Item -ItemType Directory -Force -Path $destinationDir | Out-Null
    $artifactItems = Get-ChildItem -LiteralPath $_.FullName -Force
    if (-not $artifactItems) {
        throw "Updater artifact directory is empty: $($_.FullName)"
    }

    $artifactItems | Copy-Item -Destination $destinationDir -Recurse -Force
}

Write-Host "Updater feed staged under $feedRoot for $TagName"
