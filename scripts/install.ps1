<#
.SYNOPSIS
    Installs Desdec from a published release, or builds it from source.

.DESCRIPTION
    The companion of scripts/install.sh, for a Windows without a POSIX shell.
    It downloads the archive published for this machine, checks its SHA-256,
    and only then puts the binary anywhere. A release whose checksum does not
    match is thrown away rather than installed with a warning printed above it.

    The checksum says the download is intact and nothing more. Releases are
    not signed from v0.4.1 on, so that is the whole of the check; up to v0.4.0
    they were, and those archives keep the detached .asc next to them for
    anyone who wants to check one with gpg and the key at the root of the
    repository.

    Nothing is written outside the prefix. The user PATH is left alone unless
    -AddToPath is given, and then only that one entry is appended.

    No Start menu shortcut is written, where install.sh gives a Linux install
    an icon and a menu entry. Two things are missing for it and neither is a
    line of PowerShell: desdec-app.exe embeds no icon resource, so a shortcut
    to it would carry the generic one, and `--write-icon` writes a PNG, which
    a shortcut cannot use. Saying so is better than a tile with no mark on it.

.EXAMPLE
    .\install.ps1
.EXAMPLE
    .\install.ps1 -Version v0.4.65 -Prefix C:\Tools
.EXAMPLE
    .\install.ps1 -FromSource
#>
[CmdletBinding()]
param(
    [string] $Version,
    [string] $Prefix = "$env:LOCALAPPDATA\Programs\Desdec",
    [string] $Name = 'desdec',
    [switch] $Pre,
    [switch] $FromSource,
    # Accepted and ignored: it was the way to install an unsigned release back
    # when a missing signature stopped the script. Nothing is signed now, so a
    # command that still carries it keeps working rather than failing on a
    # parameter that no longer exists.
    [switch] $SkipSignature,
    [switch] $AddToPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Repo = 'fredza/Desdec'
$Binary = 'desdec-app.exe'
$Asset = 'desdec-windows-x86_64-release.zip'

# Write-Error would frame a plain refusal in an eight-line parser trace and
# the position of the call that made it; the reader needs the sentence.
function Fail([string] $Message) {
    $host.UI.WriteErrorLine("error: $Message")
    exit 1
}

# The one architecture the workflow publishes for Windows. A machine that is
# not it is told so by name, with the source build as the way out, rather than
# handed an executable it cannot run.
$arch = $env:PROCESSOR_ARCHITECTURE
if (-not $FromSource -and $arch -ne 'AMD64') {
    Fail "no published archive for Windows/$arch; run with -FromSource to build it here"
}

$workspace = Join-Path ([System.IO.Path]::GetTempPath()) ("desdec-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $workspace | Out-Null

# Puts one built binary in place. Written beside the target and then moved, so
# a copy that is running is replaced rather than truncated under its own feet.
function Install-Binary([string] $Built) {
    if (-not (Test-Path $Prefix)) { New-Item -ItemType Directory -Path $Prefix -Force | Out-Null }
    $target = Join-Path $Prefix "$Name.exe"
    $staged = Join-Path $Prefix ".$Name.$PID.exe"
    Copy-Item -LiteralPath $Built -Destination $staged -Force
    Move-Item -LiteralPath $staged -Destination $target -Force
    Write-Host "Installed $target"

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $onPath = ($userPath -split ';') -contains $Prefix
    if ($AddToPath -and -not $onPath) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$Prefix", 'User')
        Write-Host "Added $Prefix to your user PATH — open a new terminal for it to take effect."
    } elseif (-not $onPath) {
        Write-Host ''
        Write-Warning "$Prefix is not on your PATH. Re-run with -AddToPath, or add it yourself."
    }
}

function Get-LatestTag {
    $url = if ($Pre) {
        "https://api.github.com/repos/$Repo/releases?per_page=1"
    } else {
        "https://api.github.com/repos/$Repo/releases/latest"
    }
    try {
        $response = Invoke-RestMethod -Uri $url -Headers @{ 'User-Agent' = 'desdec-install' }
    } catch {
        Fail "could not ask GitHub for the latest release: $($_.Exception.Message)"
    }
    if ($response -is [array]) { $response[0].tag_name } else { $response.tag_name }
}

function Install-FromRelease {
    if (-not $Version) { $script:Version = Get-LatestTag }
    if (-not $Version) { Fail 'could not work out which release to install' }

    $base = "https://github.com/$Repo/releases/download/$Version"
    $archive = Join-Path $workspace $Asset
    Write-Host "Installing Desdec $Version ($Asset)"

    # Progress rendering makes Invoke-WebRequest an order of magnitude slower
    # on a file this size, and nothing here reads it.
    $previousProgress = $ProgressPreference
    $ProgressPreference = 'SilentlyContinue'
    try {
        try {
            Invoke-WebRequest -Uri "$base/$Asset" -OutFile $archive
        } catch {
            Fail "$Version has no $Asset — check the release page, or use -FromSource"
        }
        try {
            Invoke-WebRequest -Uri "$base/$Asset.sha256" -OutFile "$archive.sha256"
        } catch {
            Fail "$Version publishes no checksum for $Asset; refusing to install it unchecked"
        }

        Write-Host 'Checking the SHA-256'
        # The file is `<hash>  <name>`, written on Windows without a trailing
        # newline and on the other platforms with one; only the first field is
        # read, so both forms parse.
        $published = ((Get-Content -LiteralPath "$archive.sha256" -Raw).Trim() -split '\s+')[0]
        $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash
        if ($published -ine $actual) {
            Fail "the SHA-256 of $Asset does not match what $Version published"
        }

        $unpacked = Join-Path $workspace 'unpacked'
        Expand-Archive -LiteralPath $archive -DestinationPath $unpacked -Force
        # The archive holds the executable at its root today; it is looked for
        # rather than assumed, so a layout that gains a directory still works.
        $built = Get-ChildItem -Path $unpacked -Filter $Binary -Recurse -File | Select-Object -First 1
        if (-not $built) { Fail "the archive does not hold $Binary" }

        Install-Binary $built.FullName
    } finally {
        $ProgressPreference = $previousProgress
    }
}

function Install-FromSource {
    foreach ($tool in 'cargo', 'git') {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            Fail "$tool is needed and is not installed"
        }
    }
    # Run from a checkout and it builds that checkout — the point of asking for
    # a source build is usually to install what is in front of you.
    if ((Test-Path 'Cargo.toml') -and (Select-String -Path 'Cargo.toml' -Pattern 'desdec-app' -Quiet)) {
        $sourceDir = (Get-Location).Path
        Write-Host "Building the checkout in $sourceDir"
    } else {
        $sourceDir = Join-Path $workspace 'Desdec'
        Write-Host "Cloning $Repo"
        if ($Version) {
            & git clone --depth 1 --branch $Version "https://github.com/$Repo.git" $sourceDir --quiet
        } else {
            & git clone --depth 1 "https://github.com/$Repo.git" $sourceDir --quiet
        }
        if ($LASTEXITCODE -ne 0) { Fail 'the clone failed' }
    }

    Write-Host 'Building with cargo — this takes a few minutes'
    Push-Location $sourceDir
    try {
        & cargo build --locked --release -p desdec-app
        if ($LASTEXITCODE -ne 0) { Fail 'the build failed' }
    } finally {
        Pop-Location
    }
    Install-Binary (Join-Path $sourceDir "target\release\$Binary")
}

try {
    if ($FromSource) { Install-FromSource } else { Install-FromRelease }

    Write-Host ''
    Write-Host "Run it with:  $Name                        # open the window"
    Write-Host "              $Name C:\Windows\notepad.exe  # or analyse a file straight away"
} finally {
    Remove-Item -LiteralPath $workspace -Recurse -Force -ErrorAction SilentlyContinue
}
