$ErrorActionPreference = 'Stop'

$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

# Remove the binary; Chocolatey removes the shim automatically.
$exePath = Join-Path $toolsDir '3va.exe'
if (Test-Path $exePath) {
  Remove-Item $exePath -Force
}
