<#
.SYNOPSIS
  Native Windows test of the NSIS installer: install, run, upgrade, uninstall, "data survives".

.DESCRIPTION
  Runs on the windows-latest CI runner against the installer `tauri build --bundles nsis` produced
  (see .github/workflows/release.yml and desktop-native.yml), and can be run the same way on a
  Windows 11 machine. It changes the current user's profile (per-user install, HKCU, Start Menu), so
  use a disposable machine or a user that has no Symphony Desktop installed. It verifies:

    1. the installer runs silently and exits 0,
    2. the per-user uninstall entry exists with the product name, version and publisher that
       tauri.conf.json declares,
    3. the application and its uninstaller are in the install directory,
    4. the INSTALLED application passes scripts/windows-native-smoke.ps1,
    5. installing the same installer again (the upgrade path) succeeds without a second entry,
    6. the silent uninstall removes the application and the uninstall entry,
    7. user data in %LOCALAPPDATA%\symphony-desktop survives the uninstall.

  Shortcuts are checked with warnings only: their exact placement is a Tauri/NSIS decision, and a
  missing shortcut should be seen, not block a release.

.EXAMPLE
  pwsh scripts/windows-installer-smoke.ps1 -Installer "target/release/bundle/nsis/Symphony Desktop_2.0.0_x64-setup.exe" -CliExe target/release/symphony-cli.exe -ExpectedVersion 2.0.0
#>
param(
  [Parameter(Mandatory = $true)] [string] $Installer,
  [Parameter(Mandatory = $true)] [string] $CliExe,
  [string] $ExpectedVersion,
  # Require a valid Authenticode signature on the installer and the installed application.
  [switch] $RequireSignature
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Installer = (Resolve-Path $Installer).Path
$CliExe = (Resolve-Path $CliExe).Path
$smokeScript = Join-Path $PSScriptRoot 'windows-native-smoke.ps1'
# The identity the installer must register comes from the Tauri config, not from this script.
$tauriConfig = Get-Content (Join-Path $PSScriptRoot '..\apps\desktop\src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json
$productName = [string] $tauriConfig.productName
$expectedPublisher = [string] $tauriConfig.bundle.publisher
if (-not $productName -or -not $expectedPublisher) { throw 'tauri.conf.json has no productName or bundle.publisher' }
$uninstallRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$dataDir = Join-Path $env:LOCALAPPDATA 'symphony-desktop'
$marker = Join-Path $dataDir 'installer-smoke-marker.txt'
$createdDataDir = $false
$createdMarker = $false
$defaultInstallDir = Join-Path $env:LOCALAPPDATA $productName
$createdInstallDir = $false

function Step([string] $message) { Write-Host "==> $message" }
function Fail([string] $message) { throw "INSTALLER SMOKE TEST FAILED: $message" }
function Warn([string] $message) { Write-Host "::warning::$message" }

function Wait-Until([scriptblock] $condition, [string] $what, [int] $seconds = 60) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    $value = & $condition
    if ($value) { return $value }
    Start-Sleep -Milliseconds 500
  }
  Fail "timed out waiting for $what"
}

# The uninstall entries of this product (normally exactly one).
function Get-UninstallEntries {
  if (-not (Test-Path $uninstallRoot)) { return @() }
  @(Get-ChildItem $uninstallRoot -ErrorAction SilentlyContinue | ForEach-Object {
    $props = Get-ItemProperty -Path $_.PSPath -ErrorAction SilentlyContinue
    if ($props -and $props.PSObject.Properties['DisplayName'] -and $props.DisplayName -like "$productName*") { $props }
  })
}

# NOT Start-Process -Wait: that also waits for every process the installer starts, and a freshly
# installed application may be launched by the installer and keep running.
function Invoke-Silent([string] $file, [string[]] $arguments, [string] $what, [int] $seconds = 300) {
  $process = Start-Process -FilePath $file -ArgumentList $arguments -PassThru
  if (-not $process.WaitForExit($seconds * 1000)) {
    try { $process.Kill() } catch { }
    Fail "$what did not finish within $seconds seconds"
  }
  if ($process.ExitCode -ne 0) { Fail "$what exited with code $($process.ExitCode)" }
}

function Stop-InstalledApp([string] $dir) {
  Get-Process -ErrorAction SilentlyContinue | Where-Object {
    try { $_.Path -and $_.Path.StartsWith($dir, [StringComparison]::OrdinalIgnoreCase) } catch { $false }
  } | ForEach-Object {
    Write-Host "stopping $($_.ProcessName) ($($_.Id))"
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Seconds 2
}

function Assert-Signed([string] $path) {
  $signature = Get-AuthenticodeSignature $path
  if ($signature.Status -ne 'Valid') { Fail "$path has signature status $($signature.Status), expected Valid" }
}

# A registry value that may not exist (StrictMode would throw on a missing property).
function Get-Value($entry, [string] $name) {
  $property = $entry.PSObject.Properties[$name]
  if ($property) { [string] $property.Value } else { '' }
}

function Get-InstallDir($entry) {
  $location = Get-Value $entry 'InstallLocation'
  $location = $location.Trim().Trim('"')
  if (-not $location) { $location = Join-Path $env:LOCALAPPDATA $productName }
  $location
}

$installDir = $null
try {
  Step 'Preconditions'
  if (@(Get-UninstallEntries).Count -gt 0) { Fail "$productName is already installed for this user; run this test on a clean profile" }
  # The cleanup below removes the install directory and the marker: only ever what this run created.
  if (Test-Path $defaultInstallDir) { Fail "$defaultInstallDir already exists; run this test on a clean profile" }
  if (Test-Path $marker) { Fail "$marker already exists; refusing to overwrite it" }
  if ($RequireSignature) { Assert-Signed $Installer }

  # Data that an uninstall must not remove: the real default data directory.
  if (-not (Test-Path $dataDir)) { New-Item -ItemType Directory -Path $dataDir | Out-Null; $createdDataDir = $true }
  Set-Content -Path $marker -Value 'installer smoke test marker' -Encoding ascii
  $createdMarker = $true

  Step 'Silent install'
  Invoke-Silent $Installer @('/S') 'the installer'
  $entry = Wait-Until { @(Get-UninstallEntries) | Select-Object -First 1 } 'the uninstall entry'
  $installDir = Get-InstallDir $entry
  Write-Host "installed to $installDir"
  # Only a directory that did not exist before the install is this run's to delete.
  $createdInstallDir = $true
  Stop-InstalledApp $installDir

  Step 'Uninstall entry and installed files'
  if (@(Get-UninstallEntries).Count -ne 1) { Fail 'expected exactly one uninstall entry' }
  $version = Get-Value $entry 'DisplayVersion'
  $publisher = Get-Value $entry 'Publisher'
  if ($ExpectedVersion -and $version -ne $ExpectedVersion) {
    Fail "uninstall entry version is '$version', expected '$ExpectedVersion'"
  }
  $displayName = Get-Value $entry 'DisplayName'
  if ($displayName -ne $productName) { Fail "uninstall entry is named '$displayName', expected '$productName'" }
  if ($publisher -ne $expectedPublisher) { Fail "uninstall entry publisher is '$publisher', expected '$expectedPublisher'" }
  Write-Host "entry: $displayName $version by $publisher"
  if (-not (Test-Path $installDir)) { Fail "install directory $installDir does not exist" }
  $uninstaller = Join-Path $installDir 'uninstall.exe'
  if (-not (Test-Path $uninstaller)) { Fail "no uninstall.exe in $installDir" }
  $app = Get-ChildItem $installDir -Filter '*.exe' | Where-Object { $_.Name -ne 'uninstall.exe' } |
    Sort-Object { if ($_.Name -eq 'symphony-desktop.exe') { 0 } else { 1 } } | Select-Object -First 1
  if (-not $app) { Fail "no application executable in $installDir" }
  Write-Host "application: $($app.FullName) ($([math]::Round($app.Length / 1MB, 1)) MB)"
  if ($RequireSignature) { Assert-Signed $app.FullName }

  $shortcuts = @(Get-ChildItem (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs') -Recurse -Filter '*.lnk' -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like "$productName*" })
  if ($shortcuts.Count -eq 0) { Warn 'no Start Menu shortcut for Symphony Desktop was found after the install' }
  else { Write-Host "Start Menu shortcut: $($shortcuts[0].FullName)" }

  Step 'The installed application passes the native runtime smoke test'
  & pwsh -NoProfile -File $smokeScript -DesktopExe $app.FullName -CliExe $CliExe
  if ($LASTEXITCODE -ne 0) { Fail "native smoke test of the installed application failed ($LASTEXITCODE)" }
  Stop-InstalledApp $installDir

  Step 'Upgrade: the same installer over the installed application'
  Invoke-Silent $Installer @('/S') 'the second (upgrade) install'
  Stop-InstalledApp $installDir
  if (@(Get-UninstallEntries).Count -ne 1) { Fail 'the upgrade left more than one uninstall entry' }
  if (-not (Test-Path $app.FullName)) { Fail 'the application is missing after the upgrade' }

  Step 'Silent uninstall'
  # `_?=<dir>` keeps the uninstaller in this process (otherwise it copies itself to %TEMP% and
  # returns immediately); the side effect is that uninstall.exe stays behind and is removed below.
  Invoke-Silent $uninstaller @('/S', "_?=$installDir") 'the uninstaller'
  Wait-Until { -not (Test-Path $app.FullName) } 'the application to be removed'
  Wait-Until { @(Get-UninstallEntries).Count -eq 0 } 'the uninstall entry to be removed'

  Step 'User data survives the uninstall'
  if (-not (Test-Path $marker)) { Fail "$marker was removed by the uninstaller" }

  Write-Host 'INSTALLER SMOKE TEST PASSED'
}
finally {
  if ($installDir -and (Test-Path $installDir)) {
    Stop-InstalledApp $installDir
    if ($createdInstallDir) {
      # A run that failed half-way still has its uninstall entry: remove it properly first, so the
      # profile is not left with an entry that points at a deleted directory.
      $leftover = Join-Path $installDir 'uninstall.exe'
      if ((Test-Path $leftover) -and @(Get-UninstallEntries).Count -gt 0) {
        try { Invoke-Silent $leftover @('/S', "_?=$installDir") 'the cleanup uninstall' 120 }
        catch { Write-Host "cleanup uninstall failed: $_" }
      }
      Remove-Item $installDir -Recurse -Force -ErrorAction SilentlyContinue
    }
  }
  if ($createdMarker) { Remove-Item $marker -Force -ErrorAction SilentlyContinue }
  if ($createdDataDir) { Remove-Item $dataDir -Recurse -Force -ErrorAction SilentlyContinue }
}
