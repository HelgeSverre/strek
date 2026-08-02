param(
    [Parameter(Mandatory)]
    [string] $MsiPath,

    [Parameter(Mandatory)]
    [string] $InstallDirectory,

    [Parameter(Mandatory)]
    [string] $InstallLog,

    [Parameter(Mandatory)]
    [string] $UninstallLog
)

$ErrorActionPreference = "Stop"

function Invoke-MsiExec {
    param(
        [Parameter(Mandatory)]
        [string[]] $Arguments,

        [Parameter(Mandatory)]
        [string] $Operation,

        [Parameter(Mandatory)]
        [string] $LogPath
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
        if (-not $process.Start()) {
            throw "Could not start msiexec for MSI $Operation"
        }
        $process.WaitForExit()
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
