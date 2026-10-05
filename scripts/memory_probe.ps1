param(
    [string]$ExePath,
    [string]$BenchArgs
)

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = (Resolve-Path $ExePath).Path
$psi.Arguments = $BenchArgs
$psi.RedirectStandardOutput = $true
$psi.UseShellExecute = $false
$proc = [System.Diagnostics.Process]::Start($psi)
$peak = 0
while (-not $proc.HasExited) {
    try {
        $proc.Refresh()
        if ($proc.PeakWorkingSet64 -gt $peak) { $peak = $proc.PeakWorkingSet64 }
    } catch {}
    Start-Sleep -Milliseconds 20
}
$stdout = $proc.StandardOutput.ReadToEnd()
$proc.WaitForExit()
Write-Output "PEAK_WORKING_SET_BYTES=$peak"
Write-Output $stdout
