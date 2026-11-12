$base = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture'
$fxName = '{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5'
$nameName = '{a45c254e-df1c-4efd-8020-67d146a850e0},2'
foreach ($id in (Get-ChildItem $base).PSChildName) {
    $key = "$base\$id\Properties"
    $name = (Get-ItemProperty $key -Name $nameName -ErrorAction SilentlyContinue).$nameName
    $fx = (Get-ItemProperty $key -Name $fxName -ErrorAction SilentlyContinue).$fxName
    if ($null -eq $fx) { $fx = 'not-set' }
    Write-Output "$id | name=$name | fx_disabled=$fx"
}
