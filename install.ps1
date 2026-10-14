# Install, update, or remove an owned Windows release without elevation.
# PowerShell 5.1+; never changes the execution policy or permanent PATH.
# Online: .\install.ps1 [-Version v0.1.0] [-InstallDir C:\Tools\ev]
# Offline: .\install.ps1 -Archive C:\Downloads\ev-windows-x86_64.zip -Checksums C:\Downloads\SHA256SUMS
# Online installs populate all nine pinned sprites in the user's cache before publication.
# Offline archives make no network requests; ev --download-artwork fills the cache later.
# Remove only hash-matching owned files: .\install.ps1 -Uninstall [-InstallDir C:\Tools\ev]
# -OwnerProcessId is reserved for the synchronously waiting `ev update`/`ev uninstall`.
[CmdletBinding()]
param(
    [string]$Archive,
    [string]$Checksums,
    [string]$InstallDir,
    [string]$Version,
    [switch]$Uninstall,
    [int]$OwnerProcessId = 0
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
$archiveLimit = 64MB
$extractedLimit = 96MB
$archiveName = 'ev-windows-x86_64.zip'
$recognizedArchive = '(?:ev-(?:(?:darwin|linux)-(?:arm64|x86_64)\.tar\.gz|windows-x86_64\.zip)|install\.(?:sh|ps1))'
$stage = $null
$keepStage = $false
$installLock = $null
$archiveStream = $null
$zip = $null
# Functions share these fields in both -File and embedded ScriptBlock execution.
$script:animation = $false
$script:animationClock = [Diagnostics.Stopwatch]::new()
$script:animationFrame = 0
$script:animationTop = 0
$script:oldCursorVisible = $true
$oldSecurityProtocol = [Net.ServicePointManager]::SecurityProtocol
$script:ownerProcess = $null
$script:ownerStartTicks = 0L
$deferCleanup = $false

function Assert-RunningOwner([string]$Binary) {
    if ($OwnerProcessId -lt 0) { throw 'OwnerProcessId must be a positive process ID.' }
    if ($OwnerProcessId -ne 0 -and $null -eq $script:ownerProcess) {
        $script:ownerProcess = [Diagnostics.Process]::GetProcessById($OwnerProcessId)
        $script:ownerStartTicks = $script:ownerProcess.StartTime.ToUniversalTime().Ticks
    }
    if ($null -ne $script:ownerProcess) {
        if ($script:ownerProcess.HasExited -or
            -not [string]::Equals($script:ownerProcess.MainModule.FileName, $Binary, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'OwnerProcessId must identify the running owned ev.exe being changed.'
        }
    }
    foreach ($process in [Diagnostics.Process]::GetProcessesByName('ev')) {
        try {
            if ($process.HasExited) { continue }
            if ([string]::Equals($process.MainModule.FileName, $Binary, [StringComparison]::OrdinalIgnoreCase) -and
                $process.Id -ne $OwnerProcessId) {
                throw 'Another ev process is using this installation. Close it and retry.'
            }
        } finally { $process.Dispose() }
    }
}

function Start-RetiredCleanup([string]$Directory) {
    # Publication is already complete. This helper only deletes the retired, owned tree.
    # Never wait for it from ev: ev's own running image keeps the retired file mapped.
    $source = "`$ErrorActionPreference = 'Stop'; Set-StrictMode -Version 2.0;`n"
    foreach ($name in @('Get-PathAttributes', 'Assert-SafePath', 'Assert-SafeAncestry', 'Remove-PrivateTree')) {
        $source += "function $name {`n" + (Get-Item -LiteralPath "Function:\$name").Definition + "`n}`n"
    }
    $literal = $Directory.Replace("'", "''")
    $source += @"
`$directory = '$literal'
try {
    `$owner = `$null
    try { `$owner = [Diagnostics.Process]::GetProcessById($OwnerProcessId) }
    catch [ArgumentException] { }
    if (`$null -ne `$owner) {
        try {
            if (`$owner.StartTime.ToUniversalTime().Ticks -eq $ownerStartTicks -and -not `$owner.WaitForExit(120000)) {
                throw 'Original updater did not exit within 120 seconds.'
            }
        } finally { `$owner.Dispose() }
    }
    Remove-PrivateTree `$directory
} catch {
    [Console]::Error.WriteLine('ev: retired installation cleanup failed; remove this owned recovery directory after ev exits: ' + `$directory + '. ' + `$_.Exception.Message)
    exit 1
}
"@
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = [IO.Path]::Combine([Environment]::GetFolderPath([Environment+SpecialFolder]::System),
        'WindowsPowerShell', 'v1.0', 'powershell.exe')
    $info.Arguments = '-NoLogo -NoProfile -NonInteractive -EncodedCommand ' +
        [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($source))
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $helper = [Diagnostics.Process]::Start($info)
    if ($null -eq $helper) { throw 'Could not start retired-installation cleanup.' }
    $helper.Dispose()
}

function Start-InstallAnimation([string]$Phase) {
    Stop-InstallAnimation
    if ([Console]::IsOutputRedirected -or [Console]::IsErrorRedirected -or
        -not [string]::IsNullOrEmpty($env:CI) -or $env:TERM -eq 'dumb' -or
        $Host.Name -ne 'ConsoleHost') {
        [Console]::Error.WriteLine('ev installer: ' + $Phase)
        return
    }
    try {
        $width = [Console]::WindowWidth
        [Console]::Error.WriteLine($Phase.Substring(0, [Math]::Min($Phase.Length, [Math]::Max(1, $width - 1))))
        if ($width -lt 16 -or [Console]::WindowHeight -lt 8) { return }
        $script:oldCursorVisible = [Console]::CursorVisible
        $script:animation = $true
        [Console]::CursorVisible = $false
        [Console]::Error.Write("`n`n`n")
        $script:animationTop = [Console]::CursorTop - 3
        $script:animationFrame = 0
        $script:animationClock.Restart()
        Update-InstallAnimation -Force
    } catch { Stop-InstallAnimation }
}

function Update-InstallAnimation([switch]$Force) {
    if (-not $script:animation -or (-not $Force -and $script:animationClock.ElapsedMilliseconds -lt 160)) { return }
    try {
        $width = [Math]::Min([Console]::WindowWidth, [Console]::BufferWidth)
        # The phase is ordinary text above a small redraw area, so reflow of
        # long phase text never becomes part of a moving animation frame.
        $script:animationTop = [Math]::Max(0, [Console]::CursorTop - 3)
        $step = $script:animationFrame % 8
        $indent = 1
        $button = 'o'
        if ($step -eq 1 -or $step -eq 2) { $indent = 0 }
        elseif ($step -eq 5 -or $step -eq 6) { $indent = 2 }
        elseif ($step -eq 3 -or $step -eq 7) { $button = 'O' }
        $lines = @('   .-----. ', '  /#######\', ('  |--(' + $button + ')--|'), '   \_____/ ')
        $colored = $null -eq [Environment]::GetEnvironmentVariable('NO_COLOR')
        $oldColor = [Console]::ForegroundColor
        try {
            for ($row = 0; $row -lt 4; $row++) {
                [Console]::SetCursorPosition(0, $script:animationTop + $row)
                $line = ''
                if ($width -ge 16 -and [Console]::WindowHeight -ge 8) {
                    $line = (' ' * $indent) + $lines[$row]
                }
                if ($colored) {
                    if ($row -lt 2) { [Console]::ForegroundColor = [ConsoleColor]::Red }
                    elseif ($row -eq 3) { [Console]::ForegroundColor = [ConsoleColor]::White }
                    else { [Console]::ForegroundColor = $oldColor }
                }
                [Console]::Error.Write($line.PadRight([Math]::Max(0, [Math]::Min(14, $width - 1))))
            }
        } finally {
            if ($colored) { [Console]::ForegroundColor = $oldColor }
        }
        [Console]::SetCursorPosition(0, $script:animationTop + 3)
        $script:animationFrame++
        $script:animationClock.Restart()
    } catch { Stop-InstallAnimation }
}

function Stop-InstallAnimation {
    if (-not $script:animation) { return }
    $script:animation = $false
    try {
        $width = [Math]::Min([Console]::WindowWidth, [Console]::BufferWidth)
        $top = [Math]::Max(0, [Console]::CursorTop - 3)
        for ($row = 0; $row -lt 4; $row++) {
            [Console]::SetCursorPosition(0, $top + $row)
            [Console]::Error.Write(' ' * [Math]::Max(0, [Math]::Min(14, $width - 1)))
        }
        [Console]::SetCursorPosition(0, $top)
    } catch { }
    try { [Console]::CursorVisible = $script:oldCursorVisible } catch { }
}

function Get-ReleaseResponse([uri]$Uri, [Diagnostics.Stopwatch]$Clock, [string]$Method) {
    for ($redirect = 0; $redirect -le 10; $redirect++) {
        if ($Uri.Scheme -cne 'https' -or -not [string]::IsNullOrEmpty($Uri.UserInfo)) {
            throw 'Release downloads require HTTPS without URL credentials.'
        }
        $request = [Net.HttpWebRequest]::Create($Uri)
        $request.Method = $Method
        $request.AllowAutoRedirect = $false
        $request.Timeout = 15000
        $request.ReadWriteTimeout = 15000
        $request.UserAgent = 'eevee-native-installer'
        $response = $null
        try {
            $pending = $request.GetResponseAsync()
            $requestClock = [Diagnostics.Stopwatch]::StartNew()
            while (-not $pending.IsCompleted) {
                if ($Clock.ElapsedMilliseconds -gt 180000 -or $requestClock.ElapsedMilliseconds -gt 15000) {
                    throw 'Release request timed out.'
                }
                Update-InstallAnimation
                Start-Sleep -Milliseconds 50
            }
            $response = $pending.GetAwaiter().GetResult()
            $status = [int]$response.StatusCode
            if ($status -ge 300 -and $status -lt 400) {
                $location = $response.Headers['Location']
                if ([string]::IsNullOrEmpty($location)) { throw 'Release redirect has no destination.' }
                $Uri = [uri]::new($Uri, $location)
                $response.Dispose()
                $response = $null
                # Resolve latest without downloading a release's HTML page.
                if ($Method -eq 'HEAD' -and $Uri.AbsoluteUri -cmatch '\Ahttps://github\.com/kimmandoo/eevee/releases/tag/(v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))\z') {
                    return $Matches[1]
                }
                continue
            }
            if ($status -ne 200) { throw "Unexpected release HTTP status: $status" }
            if ($Method -eq 'HEAD') { throw 'Latest release did not resolve to an exact version tag.' }
            return $response
        } catch {
            if ($null -ne $response) { $response.Dispose() }
            $request.Abort()
            throw
        } finally {
            # Also abort a pending request on Ctrl+C, which may skip catch.
            if ($null -eq $response) { $request.Abort() }
        }
    }
    throw 'Too many release redirects.'
}

function Save-ReleaseFile([string]$Url, [string]$Path, [long]$Limit) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $response = $null
    $inputStream = $null
    $outputStream = $null
    try {
        $response = Get-ReleaseResponse ([uri]$Url) $clock 'GET'
        if ($response.ContentLength -gt $Limit) { throw 'Release download exceeds its size limit.' }
        $inputStream = $response.GetResponseStream()
        $outputStream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        $buffer = [byte[]]::new(65536)
        [long]$total = 0
        while ($true) {
            $pending = $inputStream.ReadAsync($buffer, 0, $buffer.Length)
            while (-not $pending.IsCompleted) {
                if ($clock.ElapsedMilliseconds -gt 180000) { throw 'Release download timed out.' }
                Update-InstallAnimation
                Start-Sleep -Milliseconds 50
            }
            if ($clock.ElapsedMilliseconds -gt 180000) { throw 'Release download timed out.' }
            $count = $pending.GetAwaiter().GetResult()
            if ($count -eq 0) { break }
            $total += $count
            if ($total -gt $Limit) { throw 'Release download exceeds its size limit.' }
            $outputStream.Write($buffer, 0, $count)
            Update-InstallAnimation
        }
        if ($total -eq 0) { throw 'Release download is empty.' }
        $outputStream.Flush($true)
    } finally {
        if ($null -ne $response) { $response.Dispose() }
        if ($null -ne $inputStream) { $inputStream.Dispose() }
        if ($null -ne $outputStream) { $outputStream.Dispose() }
    }
}

function Get-AbsolutePath([string]$Value, [string]$Label) {
    if ([string]::IsNullOrWhiteSpace($Value) -or
        $Value -notmatch '\A(?:[A-Za-z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+(?:[\\/]|\z))' -or
        $Value.StartsWith('\\?\') -or $Value.StartsWith('\\.\')) {
        throw "$Label must be an absolute Windows filesystem path."
    }
    # Validate before normalization, which can erase trailing-space/dot aliases.
    $valueRoot = [IO.Path]::GetPathRoot($Value)
    foreach ($part in ($Value.Substring($valueRoot.Length) -split '[\\/]')) {
        if ($part.Contains(':') -or $part.EndsWith('.') -or $part.EndsWith(' ') -or
            $part -match '\A(?:CON|PRN|AUX|NUL|COM[1-9\u00b9\u00b2\u00b3]|LPT[1-9\u00b9\u00b2\u00b3])(?:\.|\z)') {
            throw "$Label contains an alternate stream or ambiguous path component."
        }
    }
    $result = [IO.Path]::GetFullPath($Value)
    $root = [IO.Path]::GetPathRoot($result)
    if ($result.Length -gt $root.Length) { $result = $result.TrimEnd([char]'\') }
    return $result
}

# Command-line paths may be relative to the PowerShell location (not the .NET
# process directory); PATH entries stay strictly absolute.
function Get-ArgumentPath([string]$Value, [string]$Label) {
    if (-not [string]::IsNullOrWhiteSpace($Value) -and -not [IO.Path]::IsPathRooted($Value)) {
        # Check the user's own components before . and .. are normalized away.
        $parts = @($Value -split '[\\/]' | Where-Object { $_ -notin @('', '.', '..') })
        $null = Get-AbsolutePath ('C:\' + ($parts -join '\')) $Label
        $base = (Get-Location -PSProvider FileSystem).ProviderPath
        $Value = [IO.Path]::GetFullPath([IO.Path]::Combine($base, $Value))
    }
    return Get-AbsolutePath $Value $Label
}

function Get-PathAttributes([string]$Path) {
    try { return [IO.File]::GetAttributes($Path) }
    catch [IO.FileNotFoundException] { return $null }
    catch [IO.DirectoryNotFoundException] { return $null }
}

function Assert-SafePath([string]$Path, [bool]$Directory, [bool]$Required) {
    $attributes = Get-PathAttributes $Path
    if ($null -eq $attributes) {
        if ($Required) { throw "Required path is missing: $Path" }
        return
    }
    if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Refusing a reparse point: $Path"
    }
    $isDirectory = ($attributes -band [IO.FileAttributes]::Directory) -ne 0
    if ($isDirectory -ne $Directory) { throw "Unexpected path type: $Path" }
    if (($attributes -band [IO.FileAttributes]::Device) -ne 0) {
        throw "Refusing a device path: $Path"
    }
}

function Assert-SafeAncestry([string]$Directory) {
    $current = $Directory
    while ($null -ne $current) {
        Assert-SafePath $current $true $false
        $parent = [IO.Directory]::GetParent($current)
        if ($null -eq $parent) { $current = $null } else { $current = $parent.FullName }
    }
}

function New-SafeDirectory([string]$Directory) {
    Assert-SafeAncestry $Directory
    if ($null -ne (Get-PathAttributes $Directory)) { return }
    $parent = [IO.Directory]::GetParent($Directory)
    if ($null -eq $parent) { throw "Filesystem root is unavailable: $Directory" }
    New-SafeDirectory $parent.FullName
    Assert-SafeAncestry $parent.FullName
    [void][IO.Directory]::CreateDirectory($Directory)
    Assert-SafePath $Directory $true $true
}

function Get-StreamSha256([IO.Stream]$Stream) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash($Stream)).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function Get-BinarySha256([string]$Path) {
    Assert-SafePath $Path $false $true
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try { return Get-StreamSha256 $stream }
    finally { $stream.Dispose() }
}

function Read-ExpectedChecksum([string]$Path) {
    Assert-SafeAncestry ([IO.Path]::GetDirectoryName($Path))
    Assert-SafePath $Path $false $true
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $reader = $null
    try {
        if ($stream.Length -gt 1MB) { throw 'SHA256SUMS is too large.' }
        $reader = [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false, $true), $true)
        $seen = @{}
        $selected = $null
        while ($null -ne ($line = $reader.ReadLine())) {
            if ($line -cnotmatch ('\A([0-9a-fA-F]{64})  (' + $recognizedArchive + ')\z')) {
                throw 'SHA256SUMS must contain only SHA-256, two spaces, and recognized release basenames.'
            }
            $hash = $Matches[1].ToLowerInvariant()
            $name = $Matches[2]
            if ($seen.ContainsKey($name)) { throw "Duplicate checksum entry: $name" }
            $seen[$name] = $true
            if ($name -ceq $archiveName) { $selected = $hash }
        }
        if ($seen.Count -eq 0 -or $null -eq $selected) {
            throw "SHA256SUMS must contain exactly one entry for $archiveName."
        }
        return $selected
    } finally {
        if ($null -ne $reader) { $reader.Dispose() } else { $stream.Dispose() }
    }
}

function Assert-OwnedInstall([string]$Binary, [string]$Metadata) {
    Assert-SafePath $Binary $false $false
    Assert-SafePath $Metadata $true $false
    $hasBinary = $null -ne (Get-PathAttributes $Binary)
    $hasMetadata = $null -ne (Get-PathAttributes $Metadata)
    if (-not $hasBinary -and -not $hasMetadata) { return $false }
    if (-not $hasBinary -or -not $hasMetadata) {
        throw 'Refusing an unowned or incomplete existing ev installation.'
    }
    $expectedFiles = @('LICENSE', 'THIRD_PARTY_NOTICES', '.native-install')
    $files = [IO.Directory]::GetFileSystemEntries($Metadata)
    if ($files.Length -ne 3) { throw 'Existing .ev-install contains unexpected files.' }
    foreach ($file in $files) {
        if ($expectedFiles -cnotcontains [IO.Path]::GetFileName($file)) {
            throw "Unowned metadata entry: $file"
        }
        Assert-SafePath $file $false $true
    }
    $markerPath = [IO.Path]::Combine($Metadata, '.native-install')
    if ([IO.FileInfo]::new($markerPath).Length -gt 4096) { throw 'Invalid ev ownership marker.' }
    $marker = [IO.File]::ReadAllText($markerPath, [Text.UTF8Encoding]::new($false, $true))
    $pattern = '\Aev-native-install-v2\r?\nversion=ev [0-9][0-9A-Za-z.+_-]*\r?\nsha256=([0-9a-f]{64})\r?\nlicense-sha256=([0-9a-f]{64})\r?\nnotices-sha256=([0-9a-f]{64})\r?\n\z'
    if ($marker -cnotmatch $pattern) { throw 'Invalid ev ownership marker; refusing to replace the executable.' }
    $ownedHash = $Matches[1]
    $licenseHash = $Matches[2]
    $noticesHash = $Matches[3]
    if ((Get-BinarySha256 $Binary) -cne $ownedHash) {
        throw 'Existing ev.exe does not match its ownership marker; refusing to replace it.'
    }
    if ((Get-BinarySha256 ([IO.Path]::Combine($Metadata, 'LICENSE'))) -cne $licenseHash -or
        (Get-BinarySha256 ([IO.Path]::Combine($Metadata, 'THIRD_PARTY_NOTICES'))) -cne $noticesHash) {
        throw 'Existing license metadata does not match its ownership marker; refusing to change it.'
    }
    return $true
}

function Invoke-StagedCommand([string]$Binary, [string]$Argument, [int]$TimeoutMilliseconds) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Binary
    $info.Arguments = $Argument
    $info.WorkingDirectory = [IO.Path]::GetDirectoryName($Binary)
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $started = $false
    try {
        $started = $process.Start()
        if (-not $started) { throw "Could not start the staged ev.exe $Argument." }
        $stdoutBuffer = [char[]]::new(1024)
        $stderrBuffer = [char[]]::new(1024)
        $stdout = [Text.StringBuilder]::new()
        $stderr = [Text.StringBuilder]::new()
        $stdoutRead = $process.StandardOutput.ReadAsync($stdoutBuffer, 0, $stdoutBuffer.Length)
        $stderrRead = $process.StandardError.ReadAsync($stderrBuffer, 0, $stderrBuffer.Length)
        $clock = [Diagnostics.Stopwatch]::StartNew()
        while (-not $process.HasExited -or $null -ne $stdoutRead -or $null -ne $stderrRead) {
            if ($clock.ElapsedMilliseconds -gt $TimeoutMilliseconds) { throw "Staged ev.exe $Argument timed out." }
            if ($null -ne $stdoutRead -and $stdoutRead.IsCompleted) {
                $count = $stdoutRead.GetAwaiter().GetResult()
                if ($count -eq 0) { $stdoutRead = $null } else {
                    if ($stdout.Length + $count -gt 4096) { throw "Staged $Argument output is too large." }
                    [void]$stdout.Append($stdoutBuffer, 0, $count)
                    $stdoutRead = $process.StandardOutput.ReadAsync($stdoutBuffer, 0, $stdoutBuffer.Length)
                }
            }
            if ($null -ne $stderrRead -and $stderrRead.IsCompleted) {
                $count = $stderrRead.GetAwaiter().GetResult()
                if ($count -eq 0) { $stderrRead = $null } else {
                    if ($stderr.Length + $count -gt 4096) { throw "Staged $Argument error output is too large." }
                    [void]$stderr.Append($stderrBuffer, 0, $count)
                    $stderrRead = $process.StandardError.ReadAsync($stderrBuffer, 0, $stderrBuffer.Length)
                }
            }
            if (-not $process.HasExited -or $null -ne $stdoutRead -or $null -ne $stderrRead) {
                Start-Sleep -Milliseconds 10
                Update-InstallAnimation
            }
        }
        $process.WaitForExit()
        $result = $stdout.ToString().TrimEnd([char[]]@([char]13, [char]10))
        if ($process.ExitCode -ne 0) {
            throw "Staged ev.exe $Argument failed (exit $($process.ExitCode)); existing installation was not changed. $($stderr.ToString().Trim())"
        }
        return $result
    } finally {
        # This handle belongs only to our staged command, never the installed ev.
        if ($started -and -not $process.HasExited) {
            try { $process.Kill(); [void]$process.WaitForExit(5000) }
            catch { [Console]::Error.WriteLine('Could not stop the temporary staged process: ' + $_.Exception.Message) }
        }
        $process.Dispose()
    }
}

function Remove-PrivateTree([string]$Directory) {
    Assert-SafeAncestry $Directory
    Assert-SafePath $Directory $true $true
    foreach ($entry in [IO.Directory]::GetFileSystemEntries($Directory)) {
        $attributes = Get-PathAttributes $entry
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing to follow a reparse point during cleanup: $entry"
        }
        if (($attributes -band [IO.FileAttributes]::Directory) -ne 0) {
            Remove-PrivateTree $entry
        } else {
            Assert-SafePath $entry $false $true
            [IO.File]::Delete($entry)
        }
    }
    [IO.Directory]::Delete($Directory)
}

try {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
        throw 'This installer supports only native Windows x86_64.'
    }
    $nativeArchitecture = [Microsoft.Win32.Registry]::GetValue(
        'HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
        'PROCESSOR_ARCHITECTURE', $null)
    if (-not [Environment]::Is64BitOperatingSystem -or $nativeArchitecture -cne 'AMD64') {
        throw 'Only native Windows x86_64 is supported; ARM64/emulation is not supported.'
    }
    $offline = -not [string]::IsNullOrEmpty($Archive) -or -not [string]::IsNullOrEmpty($Checksums)
    if ($offline -and ([string]::IsNullOrEmpty($Archive) -or [string]::IsNullOrEmpty($Checksums))) {
        throw 'Supply both -Archive and -Checksums for an offline installation.'
    }
    if ($Uninstall -and ($offline -or -not [string]::IsNullOrEmpty($Version))) {
        throw '-Uninstall cannot be combined with -Archive, -Checksums, or -Version.'
    }
    $releaseTag = $Version
    if (-not $Uninstall -and [string]::IsNullOrEmpty($releaseTag)) { $releaseTag = $env:EV_VERSION }
    if (-not [string]::IsNullOrEmpty($releaseTag) -and
        ($releaseTag.Length -gt 64 -or $releaseTag -cnotmatch '\Av(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\z')) {
        throw 'Version must be an exact release tag such as v0.1.0.'
    }
    if ([string]::IsNullOrEmpty($InstallDir)) {
        if (-not [string]::IsNullOrEmpty($env:EV_INSTALL_DIR)) { $InstallDir = $env:EV_INSTALL_DIR }
        elseif (-not [string]::IsNullOrEmpty($env:LOCALAPPDATA)) {
            $InstallDir = [IO.Path]::Combine($env:LOCALAPPDATA, 'ev', 'bin')
        } else { throw 'LOCALAPPDATA is unset; supply -InstallDir or EV_INSTALL_DIR.' }
    }
    $destination = Get-ArgumentPath $InstallDir 'InstallDir'
    Assert-SafeAncestry $destination
    if ($Uninstall) { Assert-SafePath $destination $true $true }
    else { New-SafeDirectory $destination }
    Assert-SafeAncestry $destination
    $binary = [IO.Path]::Combine($destination, 'ev.exe')
    $metadata = [IO.Path]::Combine($destination, '.ev-install')
    $lockPath = [IO.Path]::Combine($destination, '.ev-install.lock')
    Assert-SafePath $lockPath $false $false
    # CreateNew and DeleteOnClose serialize installers without overwriting an existing lock.
    $installLock = [IO.FileStream]::new($lockPath, [IO.FileMode]::CreateNew,
        [IO.FileAccess]::ReadWrite, [IO.FileShare]::None, 4096, [IO.FileOptions]::DeleteOnClose)
    $hadInstall = Assert-OwnedInstall $binary $metadata
    if (($Uninstall -or $OwnerProcessId -ne 0) -and -not $hadInstall) {
        throw 'No owned ev installation exists at this destination.'
    }
    Assert-RunningOwner $binary
    $stage = [IO.Path]::Combine($destination, '.ev-stage-' + [Guid]::NewGuid().ToString('N'))
    if ($null -ne (Get-PathAttributes $stage)) { throw 'Staging path already exists.' }
    [void][IO.Directory]::CreateDirectory($stage)
    Assert-SafeAncestry $stage
    if ($Uninstall) {
        $backupBinary = [IO.Path]::Combine($stage, 'previous-ev.exe')
        $backupMetadata = [IO.Path]::Combine($stage, 'previous-metadata')
        $metadataRetired = $false
        $binaryRetired = $false
        $keepStage = $true
        try {
            [IO.Directory]::Move($metadata, $backupMetadata)
            $metadataRetired = $true
            [IO.File]::Move($binary, $backupBinary)
            $binaryRetired = $true
        } catch {
            $uninstallError = $_.Exception.Message
            try {
                if ($binaryRetired) { [IO.File]::Move($backupBinary, $binary) }
                if ($metadataRetired) { [IO.Directory]::Move($backupMetadata, $metadata) }
                $keepStage = $false
            } catch {
                throw "Uninstall failed: $uninstallError. Recovery files retained at ${stage}: $($_.Exception.Message)"
            }
            throw "Could not uninstall ev; previous installation was preserved. Close running ev and retry. Details: $uninstallError"
        }
        $keepStage = $false
        $deferCleanup = $null -ne $ownerProcess
        Write-Output "Uninstalled ev from $binary. Task, cache, and provider data were retained."
        return
    }
    if ($offline) {
        $archivePath = Get-ArgumentPath $Archive 'Archive'
        $checksumPath = Get-ArgumentPath $Checksums 'Checksums'
        if ([IO.Path]::GetFileName($archivePath) -cne $archiveName) {
            throw "Archive basename must be exactly $archiveName."
        }
    } else {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        if ([string]::IsNullOrEmpty($releaseTag)) {
            Start-InstallAnimation 'Resolving latest Eevee release'
            $releaseTag = Get-ReleaseResponse ([uri]'https://github.com/kimmandoo/eevee/releases/latest') ([Diagnostics.Stopwatch]::StartNew()) 'HEAD'
        }
        $releaseBase = "https://github.com/kimmandoo/eevee/releases/download/$releaseTag"
        $archivePath = [IO.Path]::Combine($stage, $archiveName)
        $checksumPath = [IO.Path]::Combine($stage, 'SHA256SUMS')
        Start-InstallAnimation "Downloading Eevee $releaseTag for Windows x86_64"
        Save-ReleaseFile "$releaseBase/SHA256SUMS" $checksumPath 1MB
        Save-ReleaseFile "$releaseBase/$archiveName" $archivePath $archiveLimit
    }
    Start-InstallAnimation 'Verifying release checksum and archive'
    $expectedHash = Read-ExpectedChecksum $checksumPath
    Assert-SafeAncestry ([IO.Path]::GetDirectoryName($archivePath))
    Assert-SafePath $archivePath $false $true
    # Keep the same read-only, non-write/non-delete-shared file open for hashing and extraction.
    $archiveStream = [IO.File]::Open($archivePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    if ($archiveStream.Length -gt $archiveLimit) { throw 'Archive exceeds the 64 MiB limit.' }
    if ((Get-StreamSha256 $archiveStream) -cne $expectedHash) { throw 'Archive SHA-256 mismatch; nothing was installed.' }
    $archiveStream.Position = 0
    Add-Type -AssemblyName System.IO.Compression
    $zip = [IO.Compression.ZipArchive]::new($archiveStream, [IO.Compression.ZipArchiveMode]::Read, $true)
    if ($zip.Entries.Count -ne 3) { throw 'Archive must contain exactly ev.exe, LICENSE, and THIRD_PARTY_NOTICES.' }
    $allowedNames = @('ev.exe', 'LICENSE', 'THIRD_PARTY_NOTICES')
    $members = @{}
    [long]$declaredTotal = 0
    foreach ($entry in $zip.Entries) {
        if ($allowedNames -cnotcontains $entry.FullName -or $members.ContainsKey($entry.FullName)) {
            throw "Unsafe, unexpected, or duplicate archive member: $($entry.FullName)"
        }
        if ($null -eq $entry.PSObject.Properties['ExternalAttributes']) {
            throw 'This .NET runtime cannot inspect ZIP file types; install a current Windows .NET runtime.'
        }
        # ExternalAttributes is signed Int32; widen before inspecting DOS and Unix mode bits.
        $attributes = ([long]$entry.ExternalAttributes) -band 4294967295L
        $unixType = ($attributes -shr 16) -band 61440
        if (($unixType -ne 0 -and $unixType -ne 32768) -or ($attributes -band 1104) -ne 0) {
            throw "Archive member is not a regular file (links and directories are forbidden): $($entry.FullName)"
        }
        if ($entry.Length -le 0 -or $entry.Length -gt $extractedLimit) {
            throw "Archive member has an invalid or excessive size: $($entry.FullName)"
        }
        $declaredTotal += $entry.Length
        if ($declaredTotal -gt $extractedLimit) { throw 'Extracted package exceeds the 96 MiB limit.' }
        $members[$entry.FullName] = $entry
    }

    Start-InstallAnimation 'Staging verified Eevee installation'
    $stageMetadata = [IO.Path]::Combine($stage, '.ev-install')
    [void][IO.Directory]::CreateDirectory($stageMetadata)
    $stageBinary = [IO.Path]::Combine($stage, 'ev.exe')
    $buffer = [byte[]]::new(65536)
    [long]$actualTotal = 0
    foreach ($name in $allowedNames) {
        $entry = $members[$name]
        if ($name -ceq 'ev.exe') { $outputPath = $stageBinary }
        else { $outputPath = [IO.Path]::Combine($stageMetadata, $name) }
        $input = $null
        $output = $null
        try {
            $input = $entry.Open()
            $output = [IO.File]::Open($outputPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
            [long]$written = 0
            while (($count = $input.Read($buffer, 0, $buffer.Length)) -gt 0) {
                $written += $count
                $actualTotal += $count
                if ($written -gt $entry.Length -or $actualTotal -gt $extractedLimit) {
                    throw 'Archive expands beyond its declared size or the 96 MiB limit.'
                }
                $output.Write($buffer, 0, $count)
                Update-InstallAnimation
            }
            if ($written -ne $entry.Length) { throw "Truncated archive member: $name" }
            $output.Flush($true)
        } finally {
            if ($null -ne $output) { $output.Dispose() }
            if ($null -ne $input) { $input.Dispose() }
        }
    }
    $zip.Dispose(); $zip = $null
    $archiveStream.Dispose(); $archiveStream = $null
    Start-InstallAnimation 'Checking staged Eevee version'
    $binaryHash = Get-BinarySha256 $stageBinary
    $version = Invoke-StagedCommand $stageBinary '--version' 15000
    if ($version -cnotmatch '\Aev [0-9][0-9A-Za-z.+_-]*\z') {
        throw 'Staged ev.exe --version must return one valid version line.'
    }
    if (-not [string]::IsNullOrEmpty($releaseTag) -and $version -cne ('ev ' + $releaseTag.Substring(1))) {
        throw 'Staged executable version does not match the pinned release tag.'
    }
    if ((Get-BinarySha256 $stageBinary) -cne $binaryHash) { throw 'Staged executable changed during --version.' }
    if ($offline) {
        Stop-InstallAnimation
        Write-Output 'Offline install: sprite cache unchanged. Missing images? Run ev --download-artwork when online, or provide --assets-dir.'
    } else {
        Start-InstallAnimation 'Downloading and verifying all nine original sprites'
        try {
            # Run only the SHA256-verified release, with the same cache defaults as startup.
            $artworkOutput = Invoke-StagedCommand $stageBinary '--download-artwork' 300000
            Stop-InstallAnimation
            Write-Output $artworkOutput
        } catch {
            throw "Sprite setup failed; prior installation is unchanged. Completed sprites remain cached; rerun to resume, or use -Archive and -Checksums for an explicit offline installation. $($_.Exception.Message)"
        }
        if ((Get-BinarySha256 $stageBinary) -cne $binaryHash) { throw 'Staged executable changed during sprite setup.' }
    }
    Start-InstallAnimation 'Publishing verified Eevee installation'
    $licenseHash = Get-BinarySha256 ([IO.Path]::Combine($stageMetadata, 'LICENSE'))
    $noticesHash = Get-BinarySha256 ([IO.Path]::Combine($stageMetadata, 'THIRD_PARTY_NOTICES'))
    $marker = "ev-native-install-v2`nversion=$version`nsha256=$binaryHash`nlicense-sha256=$licenseHash`nnotices-sha256=$noticesHash`n"
    [IO.File]::WriteAllText([IO.Path]::Combine($stageMetadata, '.native-install'), $marker, [Text.UTF8Encoding]::new($false))
    Assert-SafeAncestry $destination
    if ((Assert-OwnedInstall $binary $metadata) -ne $hadInstall) { throw 'Installation changed during staging.' }
    Assert-RunningOwner $binary
    $backupBinary = [IO.Path]::Combine($stage, 'previous-ev.exe')
    $backupMetadata = [IO.Path]::Combine($stage, 'previous-metadata')
    $binaryPublished = $false
    $metadataBackedUp = $false
    $metadataPublished = $false
    $keepStage = $true
    try {
        # Publish metadata first; the executable rename is the final commit.
        # A locked executable leaves the old binary untouched while metadata rolls back.
        if ($hadInstall) {
            [IO.Directory]::Move($metadata, $backupMetadata)
            $metadataBackedUp = $true
        }
        [IO.Directory]::Move($stageMetadata, $metadata)
        $metadataPublished = $true
        if ($hadInstall -and $null -ne $ownerProcess) {
            # A running image can be renamed, but not deleted. Only this verified
            # updater's image may take this path; other running ev instances were rejected.
            [IO.File]::Move($binary, $backupBinary)
            [IO.File]::Move($stageBinary, $binary)
        } elseif ($hadInstall) {
            # ReplaceFile may move the old executable before reporting a failure.
            [IO.File]::Replace($stageBinary, $binary, $backupBinary, $false)
        } else { [IO.File]::Move($stageBinary, $binary) }
        $binaryPublished = $true
    } catch {
        $publicationError = $_.Exception.Message
        $rollbackErrors = [Collections.Generic.List[string]]::new()
        try {
            if ($metadataPublished) { [IO.Directory]::Move($metadata, $stageMetadata) }
            if ($metadataBackedUp) { [IO.Directory]::Move($backupMetadata, $metadata) }
        } catch { $rollbackErrors.Add('metadata: ' + $_.Exception.Message) }
        try {
            if ($hadInstall -and [IO.File]::Exists($backupBinary)) {
                if ([IO.File]::Exists($binary)) {
                    if ($null -ne $ownerProcess) {
                        [IO.File]::Move($binary, $stageBinary)
                        [IO.File]::Move($backupBinary, $binary)
                    } else { [IO.File]::Replace($backupBinary, $binary, $stageBinary, $false) }
                } else {
                    [IO.File]::Move($backupBinary, $binary)
                }
            } elseif ($binaryPublished -and -not $hadInstall) {
                [IO.File]::Move($binary, $stageBinary)
            }
        } catch { $rollbackErrors.Add('executable: ' + $_.Exception.Message) }
        if ($rollbackErrors.Count -gt 0) {
            $keepStage = $true
            throw "Publication failed: $publicationError. Rollback also failed: $($rollbackErrors -join '; '). Recovery files retained at $stage."
        }
        $keepStage = $false
        throw "Could not publish ev; previous installation was preserved. Close any running ev and retry. The installer never stops installed ev processes. Details: $publicationError"
    }
    $keepStage = $false
    $deferCleanup = $hadInstall -and $null -ne $ownerProcess
    Stop-InstallAnimation
    Write-Output "Installed $version at $binary"
    $onPath = $false
    foreach ($pathEntry in ($env:PATH -split ';')) {
        if ([string]::IsNullOrWhiteSpace($pathEntry)) { continue }
        try {
            $normalized = Get-AbsolutePath ($pathEntry.Trim().Trim([char]'"')) 'PATH entry'
            if ([string]::Equals($normalized, $destination, [StringComparison]::OrdinalIgnoreCase)) { $onPath = $true }
        } catch { }
    }
    if (-not $onPath) {
        $quotedDestination = $destination.Replace("'", "''")
        Write-Output "For this PowerShell session: `$env:PATH = '$quotedDestination;' + `$env:PATH"
        # Printed, never run: the installer does not change the persistent PATH.
        Write-Output 'For future terminals (adds to your user PATH once):'
        Write-Output "  [Environment]::SetEnvironmentVariable('Path', '$quotedDestination;' + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')"
    }
} catch {
    Stop-InstallAnimation
    [Console]::Error.WriteLine('ev installer: ' + $_.Exception.Message)
    exit 1
} finally {
    Stop-InstallAnimation
    [Net.ServicePointManager]::SecurityProtocol = $oldSecurityProtocol
    if ($null -ne $zip) { $zip.Dispose() }
    if ($null -ne $archiveStream) { $archiveStream.Dispose() }
    if ($null -ne $stage -and -not $keepStage) {
        try {
            if ($deferCleanup) { Start-RetiredCleanup $stage }
            else { Remove-PrivateTree $stage }
        } catch { [Console]::Error.WriteLine("Temporary installation files remain at ${stage}: " + $_.Exception.Message) }
    } elseif ($keepStage) {
        [Console]::Error.WriteLine("Installation transaction was interrupted or could not roll back; recovery files retained at $stage.")
    }
    if ($null -ne $installLock) { $installLock.Dispose() }
    if ($null -ne $ownerProcess) { $ownerProcess.Dispose() }
}
