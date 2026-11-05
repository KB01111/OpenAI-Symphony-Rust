<#
.SYNOPSIS
  Parses every PowerShell script under scripts/ and fails on the first syntax error.

.DESCRIPTION
  The installer and native smoke tests run only after a full Tauri/NSIS build. A script PowerShell
  cannot parse would otherwise fail tens of minutes into a CI or release run; this check runs in
  seconds, first. It only parses the files (no code is executed).

.EXAMPLE
  pwsh -NoProfile -File scripts/check-powershell-syntax.ps1
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$scripts = @(Get-ChildItem -Path $PSScriptRoot -Filter '*.ps1' -File -Recurse)
if ($scripts.Count -eq 0) { throw "no PowerShell scripts found under $PSScriptRoot" }

$failed = 0
foreach ($script in $scripts) {
  $tokens = $null
  $errors = $null
  [void] [System.Management.Automation.Language.Parser]::ParseFile($script.FullName, [ref] $tokens, [ref] $errors)
  if ($errors -and $errors.Count -gt 0) {
    $failed++
    foreach ($e in $errors) {
      $line = $e.Extent.StartLineNumber
      $column = $e.Extent.StartColumnNumber
      Write-Host "::error file=$($script.FullName),line=$line,col=$column::$($e.Message)"
    }
  }
  else {
    Write-Host "ok  $($script.Name)"
  }
}
if ($failed -gt 0) { throw "$failed PowerShell script(s) failed to parse" }
Write-Host "All $($scripts.Count) PowerShell scripts parse."
