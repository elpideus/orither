# Orither test VM agent: asks the host (10.0.2.2:8800) for PowerShell to run, runs it in this
# logged-on session (so windows it opens are on the desktop), and sends back what it printed.
$host_ = "http://10.0.2.2:8800"
$work = "C:\agent\work"
New-Item -ItemType Directory -Force $work | Out-Null
while ($true) {
  try {
    $job = Invoke-RestMethod -Uri "$host_/next" -TimeoutSec 40
    if ($job -and $job.id) {
      $file = Join-Path $work "$($job.id).ps1"
      Set-Content -Path $file -Value $job.script -Encoding UTF8
      $out = Join-Path $work "$($job.id).out"
      $p = Start-Process powershell -ArgumentList "-NoProfile","-ExecutionPolicy","Bypass","-File",$file -RedirectStandardOutput $out -RedirectStandardError "$out.err" -WindowStyle Hidden -PassThru
      $done = $p.WaitForExit([int]$job.timeout * 1000)
      if (-not $done) { $p.Kill(); $code = -1 } else { $code = $p.ExitCode }
      $text = ""
      if (Test-Path $out) { $text += Get-Content $out -Raw }
      if (Test-Path "$out.err") { $text += Get-Content "$out.err" -Raw }
      $body = @{ id = $job.id; code = $code; output = "$text" } | ConvertTo-Json -Compress
      Invoke-RestMethod -Uri "$host_/result" -Method Post -Body ([Text.Encoding]::UTF8.GetBytes($body)) -ContentType "application/json" | Out-Null
    }
  } catch {
    Start-Sleep -Seconds 3
  }
}
