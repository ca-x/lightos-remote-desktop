$ErrorActionPreference = 'Stop'
$action = '__ACTION__'
$cfg = Join-Path ([Environment]::SystemDirectory) 'powercfg.exe'
$sleep = '238c9fa8-0aad-41ed-83f4-97be242c8f20'
$buttons = '4f971e89-eebd-4455-a8de-9e59040e7347'
# Guid, subgroup, normal AC/DC values used only when the original is unknown.
$settings = @(
    @{ id='29f6c1db-86da-48c5-9fdb-f2b67b1f44da'; group=$sleep; ac=1800; dc=900 },
    @{ id='9d7815a6-7ee4-497e-8888-515a05f02364'; group=$sleep; ac=0; dc=0 },
    @{ id='7bc4a2f9-d8fc-4469-b07b-33eb785aaca0'; group=$sleep; ac=120; dc=120 },
    @{ id='5ca83367-6e45-459f-a27b-476b1d01c936'; group=$buttons; ac=1; dc=1 }
)
function Power([string[]]$arguments) {
    $lines = & $cfg @arguments
    if ($LASTEXITCODE -ne 0) { throw 'Power policy command failed' }
    $lines
}
function Active {
    $text = (Power @('/getactivescheme')) -join ' '
    if ($text -notmatch '[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}') { throw 'Unknown active scheme' }
    $Matches[0]
}
# The last two hexadecimal indexes for each setting are AC and DC in all locales.
function Values($scheme) {
    $values = @{}
    $current = $null
    foreach ($line in (Power @('/qh',$scheme))) {
        if ($line -match '([0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12})') {
            $current = $Matches[1].ToLowerInvariant()
        } elseif ($current -and $line -match '0x([0-9a-f]{1,8})\s*$') {
            $values[$current] = @($values[$current] | Where-Object { $null -ne $_ }) + [Convert]::ToUInt32($Matches[1],16)
        }
    }
    $result = @{}
    foreach ($s in $settings) {
        $pair = $values[$s.id]
        if ($null -eq $pair -or $pair.Count -lt 2) { throw 'Incomplete power scheme' }
        $result[$s.id] = @($pair[-2],$pair[-1])
    }
    $result
}
function Prevented($values) {
    foreach ($s in $settings) {
        $policy = Get-ItemProperty -LiteralPath ("HKLM:\SOFTWARE\Policies\Microsoft\Power\PowerSettings\" + $s.id) -ErrorAction SilentlyContinue
        foreach ($index in 0,1) {
            $name = @('ACSettingIndex','DCSettingIndex')[$index]
            $value = $values[$s.id][$index]
            if ($policy -and $null -ne $policy.$name) { $value = $policy.$name }
            if ($value -ne 0) { return $false }
        }
    }
    return $true
}
function Apply($scheme,$desired,[switch]$rollback) {
    foreach ($s in $settings) {
        Power @('/setacvalueindex',$scheme,$s.group,$s.id,([string]$desired[$s.id][0])) | Out-Null
        Power @('/setdcvalueindex',$scheme,$s.group,$s.id,([string]$desired[$s.id][1])) | Out-Null
    }
    if ((Active) -ne $scheme) {
        if ($rollback) { return }
        throw 'Active power scheme changed'
    }
    Power @('/setactive',$scheme) | Out-Null
}
$scheme = Active
$old = Values $scheme
$enabled = Prevented $old
if ($action -eq 'read') { if ($enabled) { 'enabled' } else { 'disabled' }; exit 0 }
$want = $action -eq 'enable'
if ($want -eq $enabled) { exit 0 }
$store = "HKLM:\SOFTWARE\Lazycat\RemoteDesktop\Power\$scheme"
$saved = (Get-ItemProperty -LiteralPath $store -Name Previous -ErrorAction SilentlyContinue).Previous
if ($want -and !$saved) {
    New-Item -Path $store -Force | Out-Null
    New-ItemProperty -LiteralPath $store -Name Previous -Value ($old | ConvertTo-Json -Compress) -PropertyType String -Force | Out-Null
}
if ($saved) { $snapshot = ConvertFrom-Json $saved }
$desired = @{}
foreach ($s in $settings) {
    if ($want) { $desired[$s.id] = @(0,0) }
    elseif ($saved) {
        $pair = $snapshot.PSObject.Properties[$s.id].Value
        if ($pair.Count -ne 2) { throw 'Incomplete prior power policy' }
        $desired[$s.id] = @([uint32]$pair[0],[uint32]$pair[1])
    } else { $desired[$s.id] = @($s.ac,$s.dc) }
}
try {
    Apply $scheme $desired
    if ((Prevented (Values $scheme)) -ne $want) { throw 'System policy overrides this request' }
} catch {
    Apply $scheme $old -rollback
    if ($want -and !$saved) { Remove-ItemProperty -LiteralPath $store -Name Previous }
    throw
}
if (!$want -and $saved) { Remove-ItemProperty -LiteralPath $store -Name Previous }
