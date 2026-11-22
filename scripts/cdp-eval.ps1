# CDP Runtime.evaluate 一次性查询（PowerShell ClientWebSocket，无第三方依赖）
param(
  [Parameter(Mandatory = $true)][string]$WsUrl,
  [Parameter(Mandatory = $true)][string]$Expr
)
Add-Type -AssemblyName System.Net.Http
$ws = New-Object System.Net.WebSockets.ClientWebSocket
$uri = New-Object Uri($WsUrl)
$ct = New-Object Threading.CancellationToken
$ws.ConnectAsync($uri, $ct).Wait()
$body = @{ id = 1; method = "Runtime.evaluate"; params = @{ expression = $Expr; returnByValue = $true } } | ConvertTo-Json -Compress
$bytes = [Text.Encoding]::UTF8.GetBytes($body)
$ws.SendAsync([ArraySegment[byte]]::new($bytes), [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).Wait()
$buf = New-Object byte[] 65536
$ms = New-Object IO.MemoryStream
while ($true) {
  $seg = [ArraySegment[byte]]::new($buf)
  $res = $ws.ReceiveAsync($seg, $ct).Result
  $ms.Write($buf, 0, $res.Count)
  if ($res.EndOfMessage) { break }
}
$ws.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, "bye", $ct).Wait()
$json = [Text.Encoding]::UTF8.GetString($ms.ToArray())
$obj = $json | ConvertFrom-Json
if ($obj.result.exceptionDetails) { Write-Output ("EXCEPTION: " + ($obj.result.exceptionDetails.text | ConvertTo-Json -Compress)); exit 1 }
Write-Output ($obj.result.result.value | ConvertTo-Json -Compress -Depth 8)
