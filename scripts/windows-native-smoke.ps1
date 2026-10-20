<#
.SYNOPSIS
  Native Windows smoke test of the packaged Symphony Desktop host and CLI.

.DESCRIPTION
  Runs on the windows-latest CI runner after `tauri build` (see .github/workflows/desktop-native.yml)
  and can be run locally on Windows 11 the same way. It starts the real release binaries against an
  isolated data directory whose ACL deliberately grants Everyone full control, and verifies:

    1. runtime initialization (desktop publishes runtime.json; process stays up),
    2. owner-only DACL on runtime.json and the credential vault despite the permissive directory,
    3. the authenticated local API (token required; remote access disabled by default),
    4. credential persistence across a restart (DPAPI vault),
    5. single scheduler: the CLI refuses to start a second runtime; a second desktop instance exits,
    6. shutdown and recovery: no orphaned Symphony processes; the lock is released after a crash,
    7. attachment: a desktop started while the CLI owns the data directory attaches without
       taking over (runtime.json still belongs to the CLI).

  Behaviour that needs an interactive user (tray menu, folder picker, notifications, window-state
  restore, login autostart at sign-in) is covered by docs/WINDOWS_NATIVE_SMOKE_TEST.md.

.EXAMPLE
  pwsh scripts/windows-native-smoke.ps1 -DesktopExe target/release/symphony-desktop.exe -CliExe target/release/symphony-cli.exe
#>
param(
  [Parameter(Mandatory = $true)] [string] $DesktopExe,
  [Parameter(Mandatory = $true)] [string] $CliExe
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$DesktopExe = (Resolve-Path $DesktopExe).Path
$CliExe = (Resolve-Path $CliExe).Path
$data = Join-Path ([IO.Path]::GetTempPath()) ("symphony-smoke-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $data | Out-Null
$env:SYMPHONY_DATA_DIR = $data
$env:RUST_LOG = 'info'
$discovery = Join-Path $data 'runtime.json'
$vault = Join-Path $data 'credentials.vault'
$started = New-Object System.Collections.Generic.List[System.Diagnostics.Process]

function Step([string] $message) { Write-Host "==> $message" }
function Fail([string] $message) { throw "SMOKE TEST FAILED: $message" }

function Wait-Until([scriptblock] $condition, [string] $what, [int] $seconds = 90) {
  $deadline = (Get-Date).AddSeconds($seconds)
  while ((Get-Date) -lt $deadline) {
    $value = & $condition
    if ($value) { return $value }
    Start-Sleep -Milliseconds 250
  }
  Fail "timed out waiting for $what"
}

function Read-Discovery {
  if (-not (Test-Path $discovery)) { return $null }
  try { return Get-Content $discovery -Raw | ConvertFrom-Json } catch { return $null }
}

function Assert-OwnerOnly([string] $path) {
  $acl = Get-Acl -LiteralPath $path
  if (-not $acl.AreAccessRulesProtected) { Fail "$path inherits its directory's permissions" }
  $me = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  $allowed = @($me, 'S-1-5-18', 'S-1-5-32-544')
  foreach ($rule in $acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier])) {
    if ($rule.AccessControlType -eq 'Allow' -and $allowed -notcontains $rule.IdentityReference.Value) {
      Fail "$path grants access to $($rule.IdentityReference.Value)"
    }
  }
  Write-Host "    $path is owner-only: $($acl.Sddl)"
}

function Invoke-Api([string] $method, [string] $url, [string] $token, $body = $null) {
  $headers = @{}
  if ($token) { $headers['Authorization'] = "Bearer $token" }
  $params = @{ Method = $method; Uri = $url; Headers = $headers; SkipHttpErrorCheck = $true; TimeoutSec = 15 }
  if ($null -ne $body) { $params['Body'] = ($body | ConvertTo-Json -Compress); $params['ContentType'] = 'application/json' }
  return Invoke-WebRequest @params
}

function Start-Desktop {
  $p = Start-Process -FilePath $DesktopExe -ArgumentList '--background' -WorkingDirectory $data -PassThru
  $started.Add($p)
  return $p
}

function Stop-All {
  foreach ($p in $started) {
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; $p.WaitForExit(15000) | Out-Null }
  }
  $started.Clear()
}

function Assert-NoOrphans {
  Start-Sleep -Seconds 2
  $left = @(Get-Process -ErrorAction SilentlyContinue | Where-Object {
      $_.Path -and ($_.Path -eq $DesktopExe -or $_.Path -eq $CliExe)
    })
  if ($left.Count -gt 0) { Fail "orphaned Symphony processes: $($left.Id -join ', ')" }
}

try {
  Step 'Granting Everyone full control on the data directory (worst-case shared --data-dir)'
  icacls $data /grant '*S-1-1-0:(OI)(CI)F' | Out-Null

  Step 'Starting the desktop host'
  $desktop = Start-Desktop
  $record = Wait-Until { $r = Read-Discovery; if ($r -and $r.pid -eq $desktop.Id) { $r } } 'runtime.json from the desktop host'
  if ($record.owner -ne 'symphony-desktop') { Fail "unexpected owner $($record.owner)" }
  if ($record.token.Length -ne 64) { Fail 'unexpected token format' }
  Assert-OwnerOnly $discovery

  Step 'Authenticated local API'
  $state = Invoke-Api GET "$($record.base_url)/api/v1/state" $record.token
  if ($state.StatusCode -ne 200) { Fail "GET /state with token returned $($state.StatusCode)" }
  $anon = Invoke-Api GET "$($record.base_url)/api/v1/state" $null
  if ($anon.StatusCode -ne 401) { Fail "GET /state without token returned $($anon.StatusCode)" }
  $remote = (Invoke-Api GET "$($record.base_url)/api/v1/remote-access" $record.token).Content | ConvertFrom-Json
  if ($remote.state -ne 'disabled') { Fail "remote access should be disabled by default (state $($remote.state))" }

  Step 'Credential vault (DPAPI, owner-only)'
  $secret = 'ghp_smoke' + [guid]::NewGuid().ToString('N')
  $saved = Invoke-Api POST "$($record.base_url)/api/v1/credentials" $record.token @{ key = 'GITHUB_TOKEN'; value = $secret }
  if ($saved.StatusCode -ne 200) { Fail "saving a credential returned $($saved.StatusCode)" }
  Assert-OwnerOnly $vault
  $vaultText = Get-Content $vault -Raw
  if ($vaultText.Contains($secret)) { Fail 'the vault contains the plaintext secret' }
  if (-not $vaultText.Contains('windows-dpapi-user')) { Fail 'the vault is not DPAPI-protected' }

  Step 'Single scheduler: the CLI must refuse the owned data directory'
  # Expected to fail: do not let its stderr turn into a terminating error.
  $ErrorActionPreference = 'Continue'
  Push-Location $data
  $cliOut = & $CliExe --data-dir $data --port 0 2>&1 | Out-String
  Pop-Location
  $cliExit = $LASTEXITCODE
  $ErrorActionPreference = 'Stop'
  if ($cliExit -eq 0) { Fail 'a second runtime (CLI) started' }
  if ($cliOut -notmatch 'already running') { Fail "unexpected CLI output: $cliOut" }
  if ($cliOut.Contains($record.token)) { Fail 'the CLI printed the access token' }

  Step 'Single instance: a second desktop process exits and the first keeps the runtime'
  $second = Start-Process -FilePath $DesktopExe -ArgumentList '--background' -WorkingDirectory $data -PassThru
  if (-not $second.WaitForExit(30000)) { Stop-Process -Id $second.Id -Force; Fail 'second desktop instance did not exit' }
  if ($desktop.HasExited) { Fail 'the first desktop instance exited' }
  if ((Read-Discovery).pid -ne $desktop.Id) { Fail 'runtime.json was taken over' }

  Step 'Crash and recovery'
  Stop-All
  Assert-NoOrphans
  $desktop = Start-Desktop
  $record2 = Wait-Until { $r = Read-Discovery; if ($r -and $r.pid -eq $desktop.Id) { $r } } 'runtime.json after restart'
  if ($record2.token -eq $record.token) { Fail 'the token was not regenerated' }
  $creds = (Invoke-Api GET "$($record2.base_url)/api/v1/credentials" $record2.token).Content | ConvertFrom-Json
  if ($creds.configured -notcontains 'GITHUB_TOKEN') { Fail 'the stored credential did not survive the restart' }
  if ((Invoke-Api GET "$($record2.base_url)/api/v1/state" $record.token).StatusCode -ne 401) { Fail 'the old token still works' }
  Stop-All
  Assert-NoOrphans

  Step 'Attachment: desktop started while the CLI owns the data directory'
  $cli = Start-Process -FilePath $CliExe -ArgumentList @('--data-dir', $data, '--port', '0') -WorkingDirectory $data -PassThru -WindowStyle Hidden
  $started.Add($cli)
  $cliRecord = Wait-Until { $r = Read-Discovery; if ($r -and $r.pid -eq $cli.Id) { $r } } 'runtime.json from the CLI'
  Assert-OwnerOnly $discovery
  $desktop = Start-Desktop
  Start-Sleep -Seconds 10
  if ($desktop.HasExited) { Fail "the attached desktop exited with code $($desktop.ExitCode)" }
  if ((Read-Discovery).pid -ne $cli.Id) { Fail 'the desktop took over a data directory owned by the CLI' }
  if ((Invoke-Api GET "$($cliRecord.base_url)/api/v1/state" $cliRecord.token).StatusCode -ne 200) { Fail 'CLI runtime unavailable' }
  Stop-All
  Assert-NoOrphans

  Write-Host 'Windows native smoke test passed.'
}
finally {
  Stop-All
  Remove-Item -LiteralPath $data -Recurse -Force -ErrorAction SilentlyContinue
}
