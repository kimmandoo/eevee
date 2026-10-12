# Install an explicitly supplied, verified Windows package without downloads or elevation.
# PowerShell 5.1+; never changes the execution policy or permanent PATH.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Archive,
    [Parameter(Mandatory = $true)][string]$Checksums,
    [string]$InstallDir
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'
$archiveLimit = 64MB
$extractedLimit = 96MB
$archiveName = 'ev-windows-x86_64.zip'
$recognizedArchive = 'ev-(?:(?:darwin|linux)-(?:arm64|x86_64)\.tar\.gz|windows-x86_64\.zip)'
$stage = $null
$keepStage = $false
$installLock = $null
$archiveStream = $null
$zip = $null

function Get-AbsolutePath([string]$Value, [string]$Label) {
    if ([string]::IsNullOrWhiteSpace($Value) -or
        $Value -notmatch '\A(?:[A-Za-z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+(?:[\\/]|\z))' -or
        $Value.StartsWith('\\?\') -or $Value.StartsWith('\\.\')) {
        throw "$Label must be an absolute Windows filesystem path."
    }
    $result = [IO.Path]::GetFullPath($Value)
    $root = [IO.Path]::GetPathRoot($result)
    # Reject alternate data streams and Win32 aliases, not ordinary spaces or Unicode.
    foreach ($part in $result.Substring($root.Length).Split([char]'\')) {
        if ($part.Contains(':') -or $part.EndsWith('.') -or $part.EndsWith(' ')) {
            throw "$Label contains an alternate stream or ambiguous path component."
        }
    }
    if ($result.Length -gt $root.Length) { $result = $result.TrimEnd([char]'\') }
    return $result
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
                throw 'SHA256SUMS must contain only SHA-256, two spaces, and recognized archive basenames.'
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
    $pattern = '\Aev-native-install-v1\r?\nversion=ev [0-9][0-9A-Za-z.+_-]*\r?\nsha256=([0-9a-f]{64})\r?\n\z'
    if ($marker -cnotmatch $pattern) { throw 'Invalid ev ownership marker; refusing to replace the executable.' }
    $ownedHash = $Matches[1]
    if ((Get-BinarySha256 $Binary) -cne $ownedHash) {
        throw 'Existing ev.exe does not match its ownership marker; refusing to replace it.'
    }
    return $true
}

function Get-StagedVersion([string]$Binary) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Binary
    $info.Arguments = '--version'
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
        if (-not $started) { throw 'Could not start the staged ev.exe --version.' }
        $stdoutBuffer = [char[]]::new(1024)
        $stderrBuffer = [char[]]::new(1024)
        $stdout = [Text.StringBuilder]::new()
        $stderr = [Text.StringBuilder]::new()
        $stdoutRead = $process.StandardOutput.ReadAsync($stdoutBuffer, 0, $stdoutBuffer.Length)
        $stderrRead = $process.StandardError.ReadAsync($stderrBuffer, 0, $stderrBuffer.Length)
        $clock = [Diagnostics.Stopwatch]::StartNew()
        while (-not $process.HasExited -or $null -ne $stdoutRead -or $null -ne $stderrRead) {
            if ($clock.ElapsedMilliseconds -gt 15000) { throw 'Staged ev.exe --version timed out.' }
            if ($null -ne $stdoutRead -and $stdoutRead.IsCompleted) {
                $count = $stdoutRead.GetAwaiter().GetResult()
                if ($count -eq 0) { $stdoutRead = $null } else {
                    if ($stdout.Length + $count -gt 4096) { throw 'Staged --version output is too large.' }
                    [void]$stdout.Append($stdoutBuffer, 0, $count)
                    $stdoutRead = $process.StandardOutput.ReadAsync($stdoutBuffer, 0, $stdoutBuffer.Length)
                }
            }
            if ($null -ne $stderrRead -and $stderrRead.IsCompleted) {
                $count = $stderrRead.GetAwaiter().GetResult()
                if ($count -eq 0) { $stderrRead = $null } else {
                    if ($stderr.Length + $count -gt 4096) { throw 'Staged --version error output is too large.' }
                    [void]$stderr.Append($stderrBuffer, 0, $count)
                    $stderrRead = $process.StandardError.ReadAsync($stderrBuffer, 0, $stderrBuffer.Length)
                }
            }
            if (-not $process.HasExited -or $null -ne $stdoutRead -or $null -ne $stderrRead) {
                Start-Sleep -Milliseconds 10
            }
        }
        $process.WaitForExit()
        $version = $stdout.ToString().TrimEnd([char[]]@([char]13, [char]10))
        if ($process.ExitCode -ne 0 -or $version -cnotmatch '\Aev [0-9][0-9A-Za-z.+_-]*\z') {
            throw "Staged ev.exe --version failed (exit $($process.ExitCode)); existing installation was not changed."
        }
        return $version
    } finally {
        # This handle belongs only to our staged --version, never the installed ev.
        if ($started -and -not $process.HasExited) {
            try { $process.Kill(); [void]$process.WaitForExit(5000) }
            catch { [Console]::Error.WriteLine('Could not stop the temporary staged --version process: ' + $_.Exception.Message) }
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
    $archivePath = Get-AbsolutePath $Archive 'Archive'
    $checksumPath = Get-AbsolutePath $Checksums 'Checksums'
    if ([IO.Path]::GetFileName($archivePath) -cne $archiveName) {
        throw "Archive basename must be exactly $archiveName."
    }
    if ([string]::IsNullOrEmpty($InstallDir)) {
        if (-not [string]::IsNullOrEmpty($env:EV_INSTALL_DIR)) { $InstallDir = $env:EV_INSTALL_DIR }
        elseif (-not [string]::IsNullOrEmpty($env:LOCALAPPDATA)) {
            $InstallDir = [IO.Path]::Combine($env:LOCALAPPDATA, 'ev', 'bin')
        } else { throw 'LOCALAPPDATA is unset; supply -InstallDir or EV_INSTALL_DIR.' }
    }
    $destination = Get-AbsolutePath $InstallDir 'InstallDir'
    Assert-SafeAncestry $destination
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

    New-SafeDirectory $destination
    Assert-SafeAncestry $destination
    $binary = [IO.Path]::Combine($destination, 'ev.exe')
    $metadata = [IO.Path]::Combine($destination, '.ev-install')
    $lockPath = [IO.Path]::Combine($destination, '.ev-install.lock')
    Assert-SafePath $lockPath $false $false
    # CreateNew and DeleteOnClose serialize installers without overwriting an existing lock.
    $installLock = [IO.FileStream]::new($lockPath, [IO.FileMode]::CreateNew,
        [IO.FileAccess]::ReadWrite, [IO.FileShare]::None, 4096, [IO.FileOptions]::DeleteOnClose)
    $hadInstall = Assert-OwnedInstall $binary $metadata
    $stage = [IO.Path]::Combine($destination, '.ev-stage-' + [Guid]::NewGuid().ToString('N'))
    if ($null -ne (Get-PathAttributes $stage)) { throw 'Staging path already exists.' }
    [void][IO.Directory]::CreateDirectory($stage)
    Assert-SafeAncestry $stage
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
    $binaryHash = Get-BinarySha256 $stageBinary
    $version = Get-StagedVersion $stageBinary
    if ((Get-BinarySha256 $stageBinary) -cne $binaryHash) { throw 'Staged executable changed during --version.' }
    $marker = "ev-native-install-v1`nversion=$version`nsha256=$binaryHash`n"
    [IO.File]::WriteAllText([IO.Path]::Combine($stageMetadata, '.native-install'), $marker, [Text.UTF8Encoding]::new($false))
    Assert-SafeAncestry $destination
    if ((Assert-OwnedInstall $binary $metadata) -ne $hadInstall) { throw 'Installation changed during staging.' }
    $backupBinary = [IO.Path]::Combine($stage, 'previous-ev.exe')
    $backupMetadata = [IO.Path]::Combine($stage, 'previous-metadata')
    $binaryPublished = $false
    $metadataBackedUp = $false
    $metadataPublished = $false
    try {
        # Publish metadata first; the executable's atomic rename is the final commit.
        # A locked executable therefore leaves the old binary untouched while metadata rolls back.
        if ($hadInstall) {
            [IO.Directory]::Move($metadata, $backupMetadata)
            $metadataBackedUp = $true
        }
        [IO.Directory]::Move($stageMetadata, $metadata)
        $metadataPublished = $true
        if ($hadInstall) {
            # ReplaceFile is atomic and preserves the previous executable on lock/access failure.
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
            if ($binaryPublished) {
                if ($hadInstall) { [IO.File]::Replace($backupBinary, $binary, $stageBinary, $false) }
                else { [IO.File]::Move($binary, $stageBinary) }
            }
        } catch { $rollbackErrors.Add('executable: ' + $_.Exception.Message) }
        if ($rollbackErrors.Count -gt 0) {
            $keepStage = $true
            throw "Publication failed: $publicationError. Rollback also failed: $($rollbackErrors -join '; '). Recovery files retained at $stage."
        }
        throw "Could not publish ev; previous installation was preserved. Close any running ev and retry. The installer never stops installed ev processes. Details: $publicationError"
    }
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
        Write-Output 'For future terminals, add this directory to your user PATH using Windows Environment Variables settings:'
        Write-Output $destination
    }
} catch {
    [Console]::Error.WriteLine('ev installer: ' + $_.Exception.Message)
    exit 1
} finally {
    if ($null -ne $zip) { $zip.Dispose() }
    if ($null -ne $archiveStream) { $archiveStream.Dispose() }
    if ($null -ne $stage -and -not $keepStage) {
        try { Remove-PrivateTree $stage }
        catch { [Console]::Error.WriteLine("Temporary installation files remain at ${stage}: " + $_.Exception.Message) }
    }
    if ($null -ne $installLock) { $installLock.Dispose() }
}
