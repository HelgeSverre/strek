param(
    [Parameter(Mandatory)]
    [string] $MsiPath,

    [Parameter(Mandatory)]
    [string] $InstallDirectory,

    [Parameter(Mandatory)]
    [string] $InstallLog,

    [Parameter(Mandatory)]
    [string] $UninstallLog,

    [int] $TimeoutSeconds = 120
)

$ErrorActionPreference = "Stop"

function Invoke-MsiExec {
    param(
        [Parameter(Mandatory)]
        [string[]] $Arguments,

        [Parameter(Mandatory)]
        [string] $Operation,

        [Parameter(Mandatory)]
        [string] $LogPath,

        [int] $TimeoutSeconds = 120
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = Join-Path $env:WINDIR "System32\msiexec.exe"
    $startInfo.UseShellExecute = $false
    foreach ($argument in $Arguments) {
        $startInfo.ArgumentList.Add($argument)
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    try {
        Get-CimInstance Win32_Process -Filter "Name = 'msiexec.exe'" |
            ForEach-Object { Write-Host "Existing MSI process: $($_.CommandLine)" }
        if (-not $process.Start()) {
            throw "Could not start msiexec for MSI $Operation"
        }
        Start-Sleep -Seconds 2
        $processInfo = Get-CimInstance Win32_Process -Filter "ProcessId = $($process.Id)"
        if ($null -ne $processInfo) {
            Write-Host "MSI $Operation command: $($processInfo.CommandLine)"
        }
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill($true)
            $process.WaitForExit()
            if (Test-Path $LogPath -PathType Leaf) {
                Get-Content $LogPath -Tail 200
            }
            throw "MSI $Operation timed out after $TimeoutSeconds seconds"
        }
        $exitCode = $process.ExitCode
    }
    finally {
        $process.Dispose()
    }

    if ($exitCode -ne 0) {
        if (Test-Path $LogPath -PathType Leaf) {
            Get-Content $LogPath -Tail 200
        }
        throw "MSI $Operation failed with exit code $exitCode"
    }
}

$msi = Get-Item $MsiPath
$installedExecutable = Join-Path $InstallDirectory "bin\strek.exe"
$startMenuShortcut = Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs\Strek\Strek.lnk"

Invoke-MsiExec `
    -Operation "installation" `
    -LogPath $InstallLog `
    -TimeoutSeconds $TimeoutSeconds `
    -Arguments @(
        "/i"
        $msi.FullName
        "/qn"
        "/norestart"
        "APPLICATIONFOLDER=$InstallDirectory"
        "/L*v"
        $InstallLog
    )

try {
    if (-not (Test-Path $installedExecutable -PathType Leaf)) {
        throw "MSI did not install strek.exe at $installedExecutable"
    }
    if (-not (Test-Path $startMenuShortcut -PathType Leaf)) {
        throw "MSI did not create the Start Menu shortcut"
    }
}
finally {
    Invoke-MsiExec `
        -Operation "uninstall" `
        -LogPath $UninstallLog `
        -TimeoutSeconds $TimeoutSeconds `
        -Arguments @(
            "/x"
            $msi.FullName
            "/qn"
            "/norestart"
            "/L*v"
            $UninstallLog
        )
}

if (Test-Path $installedExecutable) {
    throw "MSI uninstall left strek.exe behind"
}
if (Test-Path $startMenuShortcut) {
    throw "MSI uninstall left the Start Menu shortcut behind"
}
