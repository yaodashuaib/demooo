param([string]$Event = 'unknown')

$body = [Console]::In.ReadToEnd()
$stamp = Get-Date -Format 'HH:mm:ss.fff'
$line = "[$stamp] event=$Event bytes=$($body.Length) body=$body"
Add-Content -LiteralPath 'D:\demo\tmp\hook-events.log' -Value $line -Encoding UTF8
exit 0
