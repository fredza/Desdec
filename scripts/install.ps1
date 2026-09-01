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

    It also writes a Start menu shortcut, with the icon asked of the binary
    that was just installed — the same three things install.sh gives a Linux
    install. -NoShortcut leaves both out.

    The icon is asked for as a .ico, which is the only kind of file a shortcut
    can point at: handed a PNG, Windows shows the generic mark for an unknown
    document. A Desdec from before 2026-09-01 writes a PNG whatever extension
    it is given, and then no shortcut is written at all — a tile with no mark
    on it is worse than no tile.

.EXAMPLE
    .\install.ps1
.EXAMPLE
    .\install.ps1 -Version v0.4.65 -Prefix C:\Tools
.EXAMPLE
    .\install.ps1 -NoShortcut
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
    [switch] $AddToPath,
    [switch] $NoShortcut
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
# The icon and the Start menu shortcut.
#
# The icon is asked of the binary that was just installed rather than carried
# beside this script: it is the only arrangement in which the menu cannot come
# to show an older mark than the window. Every failure here is a warning — the
# binary is installed and runs, and a shell that would not take a shortcut is
# no reason to call the install failed.
function Add-StartMenuShortcut([string] $Target) {
    $ico = Join-Path $Prefix "$Name.ico"

    # Start-Process rather than a call: a release build of desdec-app.exe is a
    # GUI subsystem executable, so `& $Target ...` hands the prompt straight
    # back and the file would be looked for before it exists. The arguments go
    # as one quoted string, which is the form both Windows PowerShell 5.1 and
    # PowerShell 7 pass through unchanged when a path holds a space.
    try {
        $writing = Start-Process -FilePath $Target `
            -ArgumentList "--write-icon `"$ico`"" `
            -Wait -PassThru -WindowStyle Hidden -ErrorAction Stop
    } catch {
        Write-Warning "$Target could not be run to write its icon — skipping the shortcut"
        return
    }
    if ($writing.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $ico)) {
        Write-Warning "$Target could not write its icon — skipping the shortcut"
        Write-Warning '    (a Desdec from before 2026-09-01 writes a PNG whatever the extension)'
        return
    }
    # It has to be an icon rather than a PNG under that name, which is exactly
    # what an older Desdec leaves here: the first four bytes of an .ico are
    # 00 00 01 00, and a PNG begins with 89 50 4E 47.
    # Read through .NET rather than Get-Content: `-AsByteStream` is PowerShell
    # 6 and later, `-Encoding Byte` is Windows PowerShell 5.1, and the machines
    # this script is fetched onto carry both.
    try {
        $head = [System.IO.File]::ReadAllBytes($ico)
    } catch {
        Write-Warning "could not read $ico — skipping the shortcut"
        return
    }
    if ($head.Length -lt 4 -or $head[0] -ne 0 -or $head[1] -ne 0 -or $head[2] -ne 1 -or $head[3] -ne 0) {
        Remove-Item -LiteralPath $ico -Force -ErrorAction SilentlyContinue
        Write-Warning "$Target wrote a PNG rather than an icon — skipping the shortcut"
        Write-Warning '    (upgrade to a Desdec that knows how to write an .ico)'
        return
    }

    # An install under another name gets a tile of its own so it does not
    # overwrite the ordinary one, the same way the Linux entry does.
    $label = if ($Name -eq 'desdec') { 'Desdec' } else { "Desdec ($Name)" }
    $programs = [Environment]::GetFolderPath('Programs')
    if (-not $programs) {
        Write-Warning 'no Start menu folder for this user — skipping the shortcut'
        return
    }
    $lnk = Join-Path $programs "$label.lnk"

    try {
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($lnk)
        $shortcut.TargetPath = $Target
        # `,0` is the index of the icon within the file, and this file holds
        # one.
        $shortcut.IconLocation = "$ico,0"
        $shortcut.WorkingDirectory = $Prefix
        $shortcut.Description = 'Read what a program is made of'
        $shortcut.Save()
    } catch {
        Write-Warning "could not write $lnk : $($_.Exception.Message)"
        return
    }

    Write-Host "Added the icon $ico and the Start menu shortcut $lnk"
    Write-Host 'To remove them later, delete those two files; nothing else is left behind.'
}

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

    if (-not $NoShortcut) { Add-StartMenuShortcut $target }
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
