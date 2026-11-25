# Tries the MSI as a user would get it (#255), on a runner where the job is an administrator: installed silently, the
# two programs run from the installed folder, the folder in the PATH of the system, the shortcut in the Start menu, the
# types .lcf and .lct and the App Paths of labcraft (#332);
# then uninstalled, leaving none of them. A log of msiexec is printed when a step fails.
param([Parameter(Mandatory)] [string] $Msi)

$ErrorActionPreference = 'Stop'
$msi = (Resolve-Path $Msi).Path
$folder = Join-Path $env:ProgramFiles 'LabCraft'
$shortcut = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\LabCraft.lnk'
$logs = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$environment = 'HKLM:\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'

function Invoke-Msiexec([string] $Action, [string] $Log) {
    $process = Start-Process msiexec.exe -ArgumentList $Action, "`"$msi`"", '/qn', '/norestart', '/l*v', "`"$Log`"" -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        Get-Content $Log | Select-Object -Last 60
        throw "msiexec $Action ended with $($process.ExitCode)"
    }
}

$classes = 'HKLM:\SOFTWARE\Classes'
$appPaths = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\labcraft.exe'
$types = @{ '.lcf' = 'LabCraft.Function'; '.lct' = 'LabCraft.Type' }

# The default value of a key, or none when the key is not there.
function Get-Default([string] $Key) {
    if (-not (Test-Path -LiteralPath $Key)) { return $null }
    return (Get-Item -LiteralPath $Key).GetValue('')
}

# The two types (#332): each extension names its ProgId, whose command opens the file with the installed labcraft.
function Test-Types {
    foreach ($extension in $types.Keys) {
        $progId = $types[$extension]
        $named = Get-Default "$classes\$extension"
        if ($named -ne $progId) { throw "$extension names '$named', not $progId" }
        $command = Get-Default "$classes\$progId\shell\open\command"
        if ($command -ne "`"$folder\labcraft.exe`" -- `"%1`"") { throw "$progId opens with '$command'" }
    }
    $app = Get-Default $appPaths
    if ($app -ne "$folder\labcraft.exe") { throw "App Paths gives '$app'" }
}

function Test-InPath {
    $path = (Get-ItemProperty $environment -Name Path).Path
    return ($path -split ';') -contains $folder -or ($path -split ';') -contains "$folder\"
}

if (Test-Path $folder) { throw "$folder exists before the install" }

Invoke-Msiexec '/i' (Join-Path $logs 'install.log')
& "$folder\labcraft-cli.exe" version
if ($LASTEXITCODE -ne 0) { throw 'labcraft-cli version failed' }
& "$folder\labcraft-cli.exe" check "$folder\examples"
if ($LASTEXITCODE -ne 0) { throw 'labcraft-cli check of the installed examples failed' }
& "$folder\labcraft.exe" --version
if ($LASTEXITCODE -ne 0) { throw 'labcraft --version failed' }
if (-not (Test-InPath)) { throw "$folder is not in the PATH of the system" }
if (-not (Test-Path $shortcut)) { throw "no shortcut $shortcut" }
Test-Types
Write-Output "Installed in $folder, in the PATH, with $shortcut, .lcf and .lct opened by labcraft, and its App Paths"

Invoke-Msiexec '/x' (Join-Path $logs 'uninstall.log')
if (Test-Path $folder) { throw "$folder is left after the uninstall" }
if (Test-InPath) { throw "$folder is left in the PATH" }
if (Test-Path $shortcut) { throw "$shortcut is left" }
foreach ($key in @($types.Keys | ForEach-Object { "$classes\$_" }) + @($types.Values | ForEach-Object { "$classes\$_" }) + $appPaths) {
    if (Test-Path -LiteralPath $key) { throw "$key is left" }
}
Write-Output 'Uninstalled: nothing left'
