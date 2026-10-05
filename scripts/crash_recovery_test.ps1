# Local SQLite crash-recovery test. Uses only a temporary database; never touches production data.
param(
    [string]$Binary = "target\release\campaign.exe",
    [uint64]$End = 3000000,
    [uint64]$ChunkSize = 2000,
    [uint64]$BlockSize = 1000000
)
$ErrorActionPreference = "Stop"
$dir = Join-Path ([IO.Path]::GetTempPath()) ("a220091-crash-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $dir | Out-Null
try {
    $common = @("--start", "1", "--end", $End, "--chunk-size", $ChunkSize, "--block-size", $BlockSize, "--threads", "2", "--no-discord")
    $clean = Join-Path $dir "clean.db"
    & $Binary run --db $clean @common | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "clean run failed" }
    $cleanRoot = (& $Binary status --db $clean | Select-String "result root: (\w+)").Matches[0].Groups[1].Value

    $crash = Join-Path $dir "crash.db"
    $proc = Start-Process -FilePath $Binary -ArgumentList (@("run", "--db", $crash) + $common) -PassThru -WindowStyle Hidden
    $deadline = (Get-Date).AddSeconds(120)
    do {
        Start-Sleep -Milliseconds 200
        $done = 0
        if (Test-Path $crash) {
            $m = (& $Binary status --db $crash 2>$null | Select-String "chunks completed: (\d+)")
            if ($m) { $done = [int]$m.Matches[0].Groups[1].Value }
        }
    } until ($done -ge 5 -or $proc.HasExited -or (Get-Date) -gt $deadline)
    if ($proc.HasExited) { throw "run finished before it could be killed; increase -End" }
    Stop-Process -Id $proc.Id -Force
    $proc.WaitForExit()

    & $Binary status --db $crash | Select-String "status:|chunks completed:"
    & $Binary run --db $crash --recover @common | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "recovery run failed" }
    $status = & $Binary status --db $crash
    $crashRoot = ($status | Select-String "result root: (\w+)").Matches[0].Groups[1].Value
    $recoveries = ($status | Select-String "crash recoveries: (\d+)").Matches[0].Groups[1].Value
    if ($crashRoot -ne $cleanRoot) { throw "result root mismatch: $crashRoot vs $cleanRoot" }
    if ([int]$recoveries -lt 1) { throw "crash recovery was not recorded" }
    Write-Output "PASS: root $crashRoot identical after kill + recovery (crash recoveries: $recoveries)"
} finally {
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
}
