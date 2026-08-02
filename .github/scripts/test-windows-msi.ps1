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

function Invoke-MsiOperation {
    param(
        [Parameter(Mandatory)]
        [string] $MsiPath,

        [Parameter(Mandatory)]
        [string] $Properties,

        [Parameter(Mandatory)]
        [string] $Operation,

        [Parameter(Mandatory)]
        [string] $LogPath,

        [int] $TimeoutSeconds = 120
    )

    $job = Start-Job -ScriptBlock {
        param($PackagePath, $PropertyValues, $InstallerLog)

        $installer = New-Object -ComObject WindowsInstaller.Installer
        $installer.UILevel = 2
        $installer.EnableLog("voicewarmupx", $InstallerLog)
        $installer.InstallProduct($PackagePath, $PropertyValues)
    } -ArgumentList $MsiPath, $Properties, $LogPath

    try {
        if ($null -eq (Wait-Job $job -Timeout $TimeoutSeconds)) {
            Stop-Job $job
            throw "MSI $Operation timed out after $TimeoutSeconds seconds"
        }
        Receive-Job $job -ErrorAction Stop
    }
    catch {
        if (Test-Path $LogPath -PathType Leaf) {
            Get-Content $LogPath -Tail 200
        }
        throw
    }
    finally {
        Remove-Job $job -Force -ErrorAction SilentlyContinue
    }
}

$msi = Get-Item $MsiPath
$installedExecutable = Join-Path $InstallDirectory "bin\strek.exe"
$startMenuShortcut = Join-Path $env:ProgramData "Microsoft\Windows\Start Menu\Programs\Strek\Strek.lnk"

Invoke-MsiOperation `
    -MsiPath $msi.FullName `
    -Properties "APPLICATIONFOLDER=`"$InstallDirectory`" REBOOT=ReallySuppress" `
    -Operation "installation" `
    -LogPath $InstallLog `
    -TimeoutSeconds $TimeoutSeconds

try {
    if (-not (Test-Path $installedExecutable -PathType Leaf)) {
        throw "MSI did not install strek.exe at $installedExecutable"
    }
    if (-not (Test-Path $startMenuShortcut -PathType Leaf)) {
        throw "MSI did not create the Start Menu shortcut"
    }
}
finally {
    Invoke-MsiOperation `
        -MsiPath $msi.FullName `
        -Properties "REMOVE=ALL REBOOT=ReallySuppress" `
        -Operation "uninstall" `
        -LogPath $UninstallLog `
        -TimeoutSeconds $TimeoutSeconds
}

if (Test-Path $installedExecutable) {
    throw "MSI uninstall left strek.exe behind"
}
if (Test-Path $startMenuShortcut) {
    throw "MSI uninstall left the Start Menu shortcut behind"
}
