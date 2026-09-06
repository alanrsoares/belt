# belt installer for Windows PowerShell
#
#   irm https://raw.githubusercontent.com/alanrsoares/belt/main/install.ps1 | iex
#
# Install a subset:
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/alanrsoares/belt/main/install.ps1))) webdriver jwt
#
# Options (env vars):
#   GITHUB_TOKEN / GH_TOKEN   auth to lift anonymous GitHub API rate limits
#   BELT_VERSION              install a specific tag (e.g. v0.1.0); default: latest
#   BELT_BIN_DIR              install destination (default $HOME\.local\bin)
#   BELT_API_URL              override release endpoint
[CmdletBinding()]
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Tools
)

$ErrorActionPreference = 'Stop'

$Repo = "alanrsoares/belt"
$AllTools = @("scaffold", "portkill", "jwt", "devclean", "fanout", "webdriver")
$DefaultBinDir = Join-Path $HOME ".local\bin"
$BinDir = if ($env:BELT_BIN_DIR) { $env:BELT_BIN_DIR } else { $DefaultBinDir }
$Token = if ($env:GITHUB_TOKEN) { $env:GITHUB_TOKEN } elseif ($env:GH_TOKEN) { $env:GH_TOKEN } else { "" }

if ($Tools.Count -gt 0) {
    foreach ($tool in $Tools) {
        if ($AllTools -notcontains $tool) {
            Write-Error "unknown tool: $tool (available: $($AllTools -join ' '))"
            exit 1
        }
    }
    $SelectedTools = $Tools
} else {
    $SelectedTools = $AllTools
}

$Arch = if ([Environment]::Is64BitOperatingSystem) { "x64" } else {
    Write-Error "unsupported architecture: only 64-bit Windows is supported"
    exit 1
}

$Asset = "belt-windows-$Arch.tar.gz"

$Headers = @{
    "User-Agent" = "belt-installer"
}
if ($Token) {
    $Headers["Authorization"] = "Bearer $Token"
}

$ApiUrl = if ($env:BELT_API_URL) {
    $env:BELT_API_URL
} elseif ($env:BELT_VERSION) {
    "https://api.github.com/repos/$Repo/releases/tags/$($env:BELT_VERSION)"
} else {
    "https://api.github.com/repos/$Repo/releases/latest"
}

$ReleaseVersion = if ($env:BELT_VERSION) { $env:BELT_VERSION } else { 'latest' }
Write-Host "» platform: windows-$Arch"
Write-Host "» looking up release ($ReleaseVersion)…"

try {
    $Release = Invoke-RestMethod -Uri $ApiUrl -Headers $Headers -Method Get
} catch {
    Write-Error "failed to look up release from $ApiUrl: $_"
    exit 1
}

$DownloadUrl = $null
$ShaUrl = $null

foreach ($a in $Release.assets) {
    if ($a.name -eq $Asset) {
        $DownloadUrl = if ($Token) { $a.url } else { $a.browser_download_url }
    } elseif ($a.name -eq "$Asset.sha256") {
        $ShaUrl = if ($Token) { $a.url } else { $a.browser_download_url }
    }
}

if (-not $DownloadUrl) {
    Write-Error "no asset found matching $Asset in release $($Release.tag_name)"
    exit 1
}

$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("belt-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    $TarFile = Join-Path $TmpDir $Asset
    Write-Host "» downloading $Asset…"

    $DownloadHeaders = @{ "User-Agent" = "belt-installer" }
    if ($Token) {
        $DownloadHeaders["Authorization"] = "Bearer $Token"
        $DownloadHeaders["Accept"] = "application/octet-stream"
    }

    Invoke-RestMethod -Uri $DownloadUrl -Headers $DownloadHeaders -OutFile $TarFile

    if ($ShaUrl) {
        $ShaFile = Join-Path $TmpDir "$Asset.sha256"
        Invoke-RestMethod -Uri $ShaUrl -Headers $DownloadHeaders -OutFile $ShaFile
        $ExpectedSha = (Get-Content $ShaFile).Trim().Split(" ")[0]
        $ActualSha = (Get-FileHash -Path $TarFile -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($ExpectedSha.ToLowerInvariant() -ne $ActualSha) {
            Write-Error "checksum mismatch:`n expected: $ExpectedSha`n actual:   $ActualSha"
            exit 1
        }
        Write-Host "» checksum ok"
    }

    if (-not (Test-Path $BinDir)) {
        New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
    }

    tar -xzf $TarFile -C $TmpDir

    foreach ($tool in $SelectedTools) {
        $ExeName = "$tool.exe"
        $Src = Join-Path $TmpDir $ExeName
        $Dest = Join-Path $BinDir $ExeName
        if (-not (Test-Path $Src)) {
            Write-Error "$ExeName is missing from $Asset"
            exit 1
        }
        Copy-Item -Path $Src -Destination $Dest -Force
        Write-Host "✓ $Dest"
    }

    $PathDirs = ($env:PATH -split [System.IO.Path]::PathSeparator)
    if ($PathDirs -notcontains $BinDir) {
        Write-Host "note: $BinDir is not in your PATH. Add it to your User PATH environment variable."
    }
} finally {
    if (Test-Path $TmpDir) {
        Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
    }
}
