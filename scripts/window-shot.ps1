# window-shot starts an irori build on a scratch IRORI_HOME, captures its
# window to a PNG, and closes it again by process id (never by name: the
# installed Go Irori is irori.exe too).
#
#   powershell -File scripts/window-shot.ps1 -Exe target/debug/irori.exe -HomeDir <scratch> -Out <png>
param(
  [Parameter(Mandatory = $true)][string]$Exe,
  [Parameter(Mandatory = $true)][string]$HomeDir,
  [Parameter(Mandatory = $true)][string]$Out,
  [int]$WaitSec = 8
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class IroriShot {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
}
"@
# Without this the capture is in scaled coordinates and clips on a HiDPI screen.
[IroriShot]::SetProcessDPIAware() | Out-Null

New-Item -ItemType Directory -Force $HomeDir | Out-Null
$env:IRORI_HOME = (Resolve-Path $HomeDir).Path
$p = Start-Process -FilePath (Resolve-Path $Exe).Path -PassThru
Write-Output "pid=$($p.Id)"
try {
  $deadline = (Get-Date).AddSeconds(30)
  while ($p.MainWindowHandle -eq 0 -and -not $p.HasExited -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 300
    $p.Refresh()
  }
  if ($p.MainWindowHandle -eq 0) { throw "no window (exited=$($p.HasExited))" }
  Start-Sleep -Seconds $WaitSec
  $h = $p.MainWindowHandle
  [IroriShot]::SetForegroundWindow($h) | Out-Null
  Start-Sleep -Milliseconds 800
  $r = New-Object IroriShot+RECT
  [IroriShot]::GetWindowRect($h, [ref]$r) | Out-Null
  $w = $r.R - $r.L
  $hh = $r.B - $r.T
  Write-Output "rect=$($r.L),$($r.T) ${w}x$hh title=$($p.MainWindowTitle)"
  $bmp = New-Object System.Drawing.Bitmap $w, $hh
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
  $g.Dispose()
  $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  Write-Output "saved=$Out"
}
finally {
  if (-not $p.HasExited) {
    $p.CloseMainWindow() | Out-Null
    if (-not $p.WaitForExit(5000)) { Stop-Process -Id $p.Id -Force }
  }
  Write-Output "exited=$($p.HasExited)"
}
