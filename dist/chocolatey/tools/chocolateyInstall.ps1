$ErrorActionPreference = 'Stop'

$packageName = '3va'
$version     = '2.1.3'
$url64       = "https://github.com/OdinoCano/3va/releases/download/v$version/3va-v$version-x86_64-pc-windows-msvc.zip"
$checksum64  = '30aa420c2e12635122432d0591c22e26f1c136c40eb05754f1607c3f90e7dff0'

$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$zipPath  = Join-Path $toolsDir '3va.zip'

Get-ChocolateyWebFile `
  -PackageName   $packageName `
  -FileFullPath  $zipPath `
  -Url64bit      $url64 `
  -Checksum64    $checksum64 `
  -ChecksumType64 'sha256'

Get-ChocolateyUnzip -FileFullPath $zipPath -Destination $toolsDir

# Remove the zip after extraction.
Remove-Item $zipPath -Force -ErrorAction SilentlyContinue

# Ensure the binary is on PATH via the shim Chocolatey creates automatically
# for any .exe in the tools directory.
$exePath = Join-Path $toolsDir '3va.exe'
if (-not (Test-Path $exePath)) {
  throw "3va.exe not found after extraction. The archive layout may have changed."
}
