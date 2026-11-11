[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $ProbePath,
    [switch] $AsStandardUser,
    [string] $ExpectedSid
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

if (-not $IsWindows) {
    throw 'This probe launcher requires native Windows.'
}

if ($AsStandardUser) {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    $isAdministrator = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    Write-Output "Identity=$($identity.Name)"
    Write-Output "Sid=$($identity.User.Value)"
    Write-Output "IsAdministrator=$($isAdministrator.ToString().ToLowerInvariant())"
    if ($identity.User.Value -ne $ExpectedSid -or $isAdministrator) {
        throw 'The child must run as the expected non-administrator account.'
    }
    if (-not (Test-Path -LiteralPath "Registry::HKEY_USERS\$ExpectedSid")) {
        throw 'The standard-user Windows profile was not loaded.'
    }
    $profileKey = "HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\$ExpectedSid"
    $profilePath = (Get-ItemProperty -LiteralPath $profileKey -Name ProfileImagePath).ProfileImagePath
    $profilePath = [Environment]::ExpandEnvironmentVariables($profilePath)
    if ($env:USERPROFILE -ne $profilePath) {
        throw 'The child environment does not belong to its loaded Windows profile.'
    }
    $env:TEMP = Join-Path $profilePath 'AppData\Local\Temp'
    $env:TMP = $env:TEMP
    [IO.Directory]::CreateDirectory($env:TEMP) | Out-Null
    Write-Output 'ProfileLoaded=true'
    Write-Output "Profile=$profilePath"
    Write-Output "Temp=$env:TEMP"

    # Forward both native streams without PowerShell's native-error formatting.
    $probe = [Diagnostics.Process]::new()
    try {
        $probe.StartInfo.FileName = $ProbePath
        $probe.StartInfo.WorkingDirectory = $env:TEMP
        $probe.StartInfo.UseShellExecute = $false
        $probe.StartInfo.CreateNoWindow = $true
        $probe.StartInfo.RedirectStandardOutput = $true
        $probe.StartInfo.RedirectStandardError = $true
        $probe.StartInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
        $probe.StartInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
        if (-not $probe.Start()) {
            throw 'The filesystem probe did not start.'
        }
        $probeStdout = $probe.StandardOutput.ReadToEndAsync()
        $probeStderr = $probe.StandardError.ReadToEndAsync()
        $probeTimedOut = -not $probe.WaitForExit(120000)
        if ($probeTimedOut) {
            $probe.Kill($true)
            if (-not $probe.WaitForExit(10000)) {
                throw 'The timed-out filesystem probe did not terminate.'
            }
        }
        [Console]::Write($probeStdout.GetAwaiter().GetResult())
        [Console]::Error.Write($probeStderr.GetAwaiter().GetResult())
        $probeExitCode = $probe.ExitCode
        if ($probeTimedOut) {
            throw 'The filesystem probe exceeded its two-minute timeout.'
        }
    }
    finally {
        $probe.Dispose()
    }
    exit $probeExitCode
}

$account = $null
$stage = $null
$process = $null
$started = $false
$exitCode = 1
$password = [Security.SecureString]::new()
$cleanupErrors = [Collections.Generic.List[string]]::new()
try {
    $sourceProbe = (Resolve-Path -LiteralPath $ProbePath).Path
    $nonce = [Guid]::NewGuid().ToString('N')
    $userName = 'mcprobe' + $nonce.Substring(0, 12)

    # Keep the random password only in a SecureString, never in logs or arguments.
    foreach ($character in 'aA1!'.ToCharArray()) {
        $password.AppendChar($character)
    }
    $alphabet = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@'
    $randomBytes = [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)
    foreach ($value in $randomBytes) {
        $password.AppendChar($alphabet[$value % $alphabet.Length])
    }
    $password.MakeReadOnly()
    $account = New-LocalUser -Name $userName -Password $password -AccountExpires (Get-Date).AddHours(1)
    $memberships = @(Get-LocalGroupMember -SID 'S-1-5-32-545' |
        Where-Object { $_.SID.Value -eq $account.SID.Value })
    if ($memberships.Count -eq 0) {
        Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $account
    }

    $stage = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) "minicore-directory-probe-$nonce"
    [IO.Directory]::CreateDirectory($stage) | Out-Null
    $acl = Get-Acl -LiteralPath $stage
    $acl.SetAccessRuleProtection($true, $false)
    $inherit = [Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    $currentSid = [Security.Principal.WindowsIdentity]::GetCurrent().User
    foreach ($sid in @($currentSid, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            $sid, 'FullControl', $inherit, 'None', 'Allow'))
    }
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $account.SID, 'ReadAndExecute', $inherit, 'None', 'Allow'))
    Set-Acl -LiteralPath $stage -AclObject $acl
    $stagedProbe = Join-Path $stage 'windows-directory-sync.exe'
    $stagedScript = Join-Path $stage 'windows-storage-probe-user.ps1'
    Copy-Item -LiteralPath $sourceProbe -Destination $stagedProbe
    Copy-Item -LiteralPath $PSCommandPath -Destination $stagedScript
    $binaryHash = (Get-FileHash -LiteralPath $sourceProbe -Algorithm SHA256).Hash
    if ((Get-FileHash -LiteralPath $stagedProbe -Algorithm SHA256).Hash -ne $binaryHash) {
        throw 'The staged probe differs from the compiled probe.'
    }
    Write-Output "BinarySha256=$binaryHash"

    $process = [Diagnostics.Process]::new()
    $process.StartInfo.FileName = Join-Path $PSHOME 'pwsh.exe'
    foreach ($argument in @('-NoLogo', '-NoProfile', '-NonInteractive', '-File', $stagedScript,
            '-AsStandardUser', '-ExpectedSid', $account.SID.Value, '-ProbePath', $stagedProbe)) {
        $process.StartInfo.ArgumentList.Add($argument)
    }
    $process.StartInfo.WorkingDirectory = $stage
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.UserName = $userName
    $process.StartInfo.Domain = $env:COMPUTERNAME
    $process.StartInfo.Password = $password
    $process.StartInfo.LoadUserProfile = $true
    # Leave Environment untouched: Windows builds it from the new user's profile.
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    $process.StartInfo.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $process.StartInfo.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $started = $process.Start()
    if (-not $started) {
        throw 'The standard-user process did not start.'
    }
    # Drain both pipes concurrently so either stream can exceed a pipe buffer.
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $timedOut = -not $process.WaitForExit(180000)
    if ($timedOut) {
        $process.Kill($true)
        if (-not $process.WaitForExit(10000)) {
            throw 'The timed-out standard-user process did not terminate.'
        }
    }
    $output = $stdout.GetAwaiter().GetResult()
    $errors = $stderr.GetAwaiter().GetResult()
    [Console]::WriteLine('--- Standard-user stdout ---')
    [Console]::Write($output)
    [Console]::Error.WriteLine('--- Standard-user stderr ---')
    [Console]::Error.Write($errors)
    $exitCode = $process.ExitCode
    Write-Output "ProbeExitCode=$exitCode"
    if ($timedOut) {
        throw 'The standard-user process exceeded its three-minute timeout.'
    }
    if ($output -notmatch '(?m)^IsAdministrator=false\r?$') {
        throw 'The standard-user identity check was not observed.'
    }
}
catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    $exitCode = 1
}
finally {
    if ($null -ne $process) {
        try {
            if ($started -and -not $process.HasExited) {
                $process.Kill($true)
                if (-not $process.WaitForExit(10000)) {
                    throw 'The standard-user process did not terminate during cleanup.'
                }
            }
        }
        catch { $cleanupErrors.Add($_.Exception.Message) }
        finally { $process.Dispose() }
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
    foreach ($message in $cleanupErrors) {
        [Console]::Error.WriteLine("Cleanup failed: $message")
    }
    if ($cleanupErrors.Count -gt 0 -and $exitCode -eq 0) {
        $exitCode = 1
    }
}
exit $exitCode
