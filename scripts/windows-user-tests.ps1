# Run existing storage and real Host process contracts under an ordinary account.
[CmdletBinding()]
param(
    [switch] $AsStandardUser,
    [string] $ExpectedSid,
    [string] $StorageTest,
    [string] $AppTest,
    [string] $HostBinary,
    [string] $Workspace
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
if (-not $IsWindows) { throw 'These account tests require native Windows.' }

function New-StartInfo([string] $Executable, [string[]] $Arguments, [string] $Directory) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    $info.WorkingDirectory = $Directory
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    return $info
}

function Invoke-CapturedProcess([Diagnostics.ProcessStartInfo] $Info, [int] $TimeoutSeconds) {
    $Info.UseShellExecute = $false
    $Info.CreateNoWindow = $true
    $Info.RedirectStandardOutput = $true
    $Info.RedirectStandardError = $true
    $Info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $Info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $Info
    $started = $false
    try {
        $started = $process.Start()
        if (-not $started) { throw 'The requested process did not start.' }
        # Concurrent reads keep either pipe from blocking a verbose test process.
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $timedOut = -not $process.WaitForExit($TimeoutSeconds * 1000)
        if ($timedOut) {
            $process.Kill($true)
            if (-not $process.WaitForExit(10000)) { throw 'The timed-out process did not terminate.' }
        }
        $result = [pscustomobject]@{
            Stdout = $stdout.GetAwaiter().GetResult()
            Stderr = $stderr.GetAwaiter().GetResult()
            ExitCode = $process.ExitCode
        }
        if ($timedOut) {
            [Console]::Write($result.Stdout)
            [Console]::Error.Write($result.Stderr)
            throw "Process exceeded its $TimeoutSeconds-second timeout."
        }
        return $result
    }
    finally {
        try {
            if ($started -and -not $process.HasExited) {
                $process.Kill($true)
                if (-not $process.WaitForExit(10000)) { throw 'Process cleanup did not finish.' }
            }
        }
        finally { $process.Dispose() }
    }
}

if ($AsStandardUser) {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    $isAdministrator = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    Write-Output "Identity=$($identity.Name)"
    Write-Output "Sid=$($identity.User.Value)"
    Write-Output "IsAdministrator=$($isAdministrator.ToString().ToLowerInvariant())"
    if ($identity.User.Value -ne $ExpectedSid -or $isAdministrator) {
        throw 'The tests must run as the expected non-administrator account.'
    }
    if (-not (Test-Path -LiteralPath "Registry::HKEY_USERS\$ExpectedSid")) {
        throw 'The standard-user Windows profile was not loaded.'
    }
    $profileKey = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$ExpectedSid"
    $profilePath = (Get-ItemProperty -LiteralPath $profileKey -Name ProfileImagePath).ProfileImagePath
    $profilePath = [Environment]::ExpandEnvironmentVariables($profilePath)
    if ($env:USERPROFILE -ne $profilePath) {
        throw 'The test environment does not belong to its loaded Windows profile.'
    }
    $env:TEMP = Join-Path $profilePath 'AppData\Local\Temp'
    $env:TMP = $env:TEMP
    [IO.Directory]::CreateDirectory($env:TEMP) | Out-Null
    Write-Output 'ProfileLoaded=true'
    Write-Output "Profile=$profilePath"
    Write-Output "Temp=$env:TEMP"

    # These are the original Cargo artifacts, including the embedded Host path.
    foreach ($executable in @($StorageTest, $AppTest, $HostBinary)) {
        $denied = $false
        $handle = $null
        try {
            $handle = [IO.File]::Open($executable, 'Open', 'Write', 'ReadWrite, Delete')
        }
        catch [UnauthorizedAccessException] { $denied = $true }
        finally { if ($null -ne $handle) { $handle.Dispose() } }
        if (-not $denied) { throw "The test account can write an executable: $executable" }
    }
    Write-Output 'ExecutableWriteAccess=denied'
    $suites = @(
        @{
            Executable = $StorageTest
            Tests = @(
                'stable_identity_discovery_and_atomic_object_roundtrip',
                'writes_confirm_exact_boundaries_and_reopen_native_content',
                'close_is_stable_and_explicit_delete_never_recurses'
            )
        },
        @{
            Executable = $AppTest
            Tests = @(
                'actual_identity_is_saved_new_incarnation_is_generated_and_root_is_exclusive',
                'stdio_message_executes_production_http_adapter_and_persists_real_output'
            )
        }
    )
    $passed = 0
    foreach ($suite in $suites) {
        $info = New-StartInfo $suite.Executable @('--list', '--format', 'terse') $env:TEMP
        $listing = Invoke-CapturedProcess $info 60
        [Console]::Error.Write($listing.Stderr)
        if ($listing.ExitCode -ne 0) { exit $listing.ExitCode }
        foreach ($test in $suite.Tests) {
            if ($listing.Stdout -notmatch ('(?m)^' + [regex]::Escape($test) + ': test\r?$')) {
                throw "The compiled test executable does not list the required test: $test"
            }
            Write-Output "TestCase=$test"
            $info = New-StartInfo $suite.Executable @('--exact', $test, '--nocapture', '--color', 'never') $env:TEMP
            $result = Invoke-CapturedProcess $info 120
            [Console]::Write($result.Stdout)
            [Console]::Error.Write($result.Stderr)
            Write-Output "TestExitCode=$($result.ExitCode)"
            if ($result.ExitCode -ne 0) { exit $result.ExitCode }
            if ($result.Stdout -notmatch '(?m)^running 1 test\r?$' -or
                $result.Stdout -notmatch 'test result: ok\. 1 passed; 0 failed; 0 ignored;') {
                throw "The required test did not execute exactly once: $test"
            }
            $passed += 1
        }
    }
    if ($passed -ne 5) { throw 'The standard-user contract selection was incomplete.' }
    Write-Output "StandardUserTestsPassed=$passed"
    exit 0
}

$Workspace = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$account = $null
$stage = $null
$savedAcls = [ordered]@{}
$password = [Security.SecureString]::new()
$cleanupErrors = [Collections.Generic.List[string]]::new()
$exitCode = 1
try {
    $cargo = (Get-Command cargo -CommandType Application).Source
    $info = New-StartInfo $cargo @('test', '--locked', '--no-run', '--message-format=json',
        '--package', 'minicore-host-storage', '--package', 'minicore-host', '--tests') $Workspace
    $build = Invoke-CapturedProcess $info 300
    [Console]::Error.Write($build.Stderr)
    if ($build.ExitCode -ne 0) {
        [Console]::Write($build.Stdout)
        throw "Cargo test artifact discovery failed: $($build.ExitCode)"
    }
    $artifacts = @($build.Stdout -split '\r?\n' | Where-Object { $_ } |
        ForEach-Object { $_ | ConvertFrom-Json } |
        Where-Object { $_.reason -eq 'compiler-artifact' -and $_.executable })
    $selections = @(
        @{ Manifest = 'crates/host-storage/Cargo.toml'; Name = 'storage'; Kind = 'test'; Test = $true },
        @{ Manifest = 'crates/host-app/Cargo.toml'; Name = 'process'; Kind = 'test'; Test = $true },
        @{ Manifest = 'crates/host-app/Cargo.toml'; Name = 'minicore-host'; Kind = 'bin'; Test = $false }
    )
    $executables = foreach ($selection in $selections) {
        $manifest = [IO.Path]::GetFullPath((Join-Path $Workspace $selection.Manifest))
        $found = @($artifacts | Where-Object {
            $_.manifest_path -eq $manifest -and $_.target.name -eq $selection.Name -and
            $_.target.kind -contains $selection.Kind -and $_.profile.test -eq $selection.Test
        } | Select-Object -ExpandProperty executable -Unique)
        if ($found.Count -ne 1) { throw "Expected one Cargo artifact for $($selection.Name)." }
        $executable = (Resolve-Path -LiteralPath $found[0]).Path
        $relative = [IO.Path]::GetRelativePath($Workspace, $executable)
        if ([IO.Path]::IsPathRooted($relative) -or $relative -eq '..' -or $relative.StartsWith('..\')) {
            throw 'Standard-user test artifacts must be inside this CI checkout.'
        }
        $executable
    }
    $StorageTest, $AppTest, $HostBinary = $executables
    Write-Output "StorageTest=$StorageTest"
    Write-Output "AppTest=$AppTest"
    Write-Output "HostBinary=$HostBinary"

    $nonce = [Guid]::NewGuid().ToString('N')
    $userName = 'mctest' + $nonce.Substring(0, 12)
    foreach ($character in 'aA1!'.ToCharArray()) { $password.AppendChar($character) }
    $alphabet = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@'
    foreach ($value in [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)) {
        $password.AppendChar($alphabet[$value % $alphabet.Length])
    }
    $password.MakeReadOnly()
    $account = New-LocalUser -Name $userName -Password $password -AccountExpires (Get-Date).AddHours(1)
    $memberships = @(Get-LocalGroupMember -SID 'S-1-5-32-545' |
        Where-Object { $_.SID.Value -eq $account.SID.Value })
    if ($memberships.Count -eq 0) { Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $account }

    # Allow only reading/executing the actual artifacts and their directories.
    # A user-specific deny also prevents inherited broad grants from allowing writes.
    $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $paths.Add($Workspace) | Out-Null
    foreach ($executable in $executables) {
        $paths.Add($executable) | Out-Null
        $directory = [IO.Path]::GetDirectoryName($executable)
        while ($directory -ne $Workspace) {
            $paths.Add($directory) | Out-Null
            $directory = [IO.Path]::GetDirectoryName($directory)
        }
    }
    $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    $denyWrites = [Security.AccessControl.FileSystemRights]'Write, Delete, DeleteSubdirectoriesAndFiles, ChangePermissions, TakeOwnership'
    foreach ($path in @($Workspace) + @($paths | Where-Object { $_ -ne $Workspace })) {
        $savedAcls[$path] = Get-Acl -LiteralPath $path
        $acl = Get-Acl -LiteralPath $path
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            $account.SID, 'ReadAndExecute', 'Allow'))
        $flags = if ($path -eq $Workspace) { $inherit } else { [Security.AccessControl.InheritanceFlags]::None }
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            $account.SID, $denyWrites, $flags, 'None', 'Deny'))
        Set-Acl -LiteralPath $path -AclObject $acl
    }

    $stage = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) "minicore-user-tests-$nonce"
    [IO.Directory]::CreateDirectory($stage) | Out-Null
    $acl = Get-Acl -LiteralPath $stage
    $acl.SetAccessRuleProtection($true, $false)
    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    foreach ($sid in @($currentSid, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            $sid, 'FullControl', $inherit, 'None', 'Allow'))
    }
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $account.SID, 'ReadAndExecute', $inherit, 'None', 'Allow'))
    Set-Acl -LiteralPath $stage -AclObject $acl
    $stagedScript = Join-Path $stage 'windows-user-tests.ps1'
    Copy-Item -LiteralPath $PSCommandPath -Destination $stagedScript
    $info = New-StartInfo (Join-Path $PSHOME 'pwsh.exe') @('-NoLogo', '-NoProfile', '-NonInteractive',
        '-File', $stagedScript, '-AsStandardUser', '-ExpectedSid', $account.SID.Value,
        '-StorageTest', $StorageTest, '-AppTest', $AppTest, '-HostBinary', $HostBinary,
        '-Workspace', $Workspace) $stage
    $info.UserName = $userName
    $info.Domain = $env:COMPUTERNAME
    $info.Password = $password
    $info.LoadUserProfile = $true
    # Leave Environment untouched: Windows creates the new user's environment.
    $result = Invoke-CapturedProcess $info 600
    [Console]::WriteLine('--- Standard-user stdout ---')
    [Console]::Write($result.Stdout)
    [Console]::Error.WriteLine('--- Standard-user stderr ---')
    [Console]::Error.Write($result.Stderr)
    $exitCode = $result.ExitCode
    Write-Output "StandardUserExitCode=$exitCode"
    if ($exitCode -eq 0 -and ($result.Stdout -notmatch '(?m)^IsAdministrator=false\r?$' -or
        $result.Stdout -notmatch '(?m)^StandardUserTestsPassed=5\r?$')) {
        throw 'The standard-user identity and five successful contracts were not observed.'
    }
}
catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    $exitCode = 1
}
finally {
    if ($null -ne $account) {
        # A failing process test may leave a Host child after its test runner exits.
        try {
            $qualifiedName = "$env:COMPUTERNAME\$($account.Name)"
            foreach ($owned in @(Get-Process -IncludeUserName | Where-Object { $_.UserName -eq $qualifiedName })) {
                try {
                    $owned.Kill($true)
                    if (-not $owned.WaitForExit(10000)) { throw 'A standard-user child survived cleanup.' }
                }
                finally { $owned.Dispose() }
            }
        }
        catch { $cleanupErrors.Add($_.Exception.Message) }
    }
    $restore = @($savedAcls.Keys)
    [Array]::Reverse($restore)
    foreach ($path in $restore) {
        try { Set-Acl -LiteralPath $path -AclObject $savedAcls[$path] }
        catch { $cleanupErrors.Add($_.Exception.Message) }
    }
    if ($null -ne $account) {
        try {
            Remove-LocalUser -SID $account.SID
            Write-Output 'TemporaryAccountRemoved=true'
        }
        catch { $cleanupErrors.Add($_.Exception.Message) }
        try {
            Get-CimInstance -ClassName Win32_UserProfile -Filter "SID = '$($account.SID.Value)'" |
                Remove-CimInstance
        }
        catch { $cleanupErrors.Add($_.Exception.Message) }
    }
    if ($null -ne $stage) {
        try { Remove-Item -LiteralPath $stage -Recurse -Force }
        catch { $cleanupErrors.Add($_.Exception.Message) }
    }
    $password.Dispose()
    foreach ($message in $cleanupErrors) { [Console]::Error.WriteLine("Cleanup failed: $message") }
    if ($cleanupErrors.Count -gt 0 -and $exitCode -eq 0) { $exitCode = 1 }
}
exit $exitCode
