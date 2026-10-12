# Validate the actual executable used by MSI, portable and standalone distributions.
param([Parameter(Mandatory)][string] $Path)
$ErrorActionPreference = 'Stop'
$bytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path))
function U16([int] $At) { if ($At -lt 0 -or $At + 2 -gt $bytes.Length) { throw 'truncated PE' }; [BitConverter]::ToUInt16($bytes, $At) }
function U32([int] $At) { if ($At -lt 0 -or $At + 4 -gt $bytes.Length) { throw 'truncated PE' }; [BitConverter]::ToUInt32($bytes, $At) }
if ((U16 0) -ne 0x5A4D) { throw 'not an executable' }
$pe = U32 0x3C
if ((U32 $pe) -ne 0x4550) { throw 'missing PE signature' }
$optional = $pe + 24
$magic = U16 $optional
$directories = switch ($magic) { 0x10B { $optional + 96 }; 0x20B { $optional + 112 }; default { throw 'unknown PE format' } }
$resourceRva = U32 ($directories + 16)
$resourceSize = U32 ($directories + 20)
if (!$resourceRva -or !$resourceSize) { throw 'executable has no resources' }
$sections = $optional + (U16 ($pe + 20))
function RvaOffset([uint32] $Rva) {
  for ($i = 0; $i -lt (U16 ($pe + 6)); $i++) {
    $section = $sections + 40 * $i
    $start = U32 ($section + 12)
    $rawSize = U32 ($section + 16)
    if ($Rva -ge $start -and [uint64]$Rva -lt [uint64]$start + $rawSize) { return (U32 ($section + 20)) + ($Rva - $start) }
  }
  throw 'resource RVA outside file sections'
}
$base = RvaOffset $resourceRva
function Entries([uint32] $Relative) {
  if ([uint64]$Relative + 16 -gt $resourceSize) { throw 'resource directory outside bounds' }
  $at = $base + $Relative
  $count = (U16 ($at + 12)) + (U16 ($at + 14))
  if ([uint64]$Relative + 16 + 8 * $count -gt $resourceSize) { throw 'resource entries outside bounds' }
  for ($i = 0; $i -lt $count; $i++) {
    $entry = $at + 16 + 8 * $i
    @{ Name = (U32 $entry); Offset = (U32 ($entry + 4)) }
  }
}
function Payloads($Entry) {
  if (!( $Entry.Offset -band 0x80000000L )) { throw 'expected resource directory' }
  foreach ($name in @(Entries ($Entry.Offset -band 0x7FFFFFFFL))) {
    if (!( $name.Offset -band 0x80000000L )) { throw 'expected resource language directory' }
    foreach ($language in @(Entries ($name.Offset -band 0x7FFFFFFFL))) {
      if ($language.Offset -band 0x80000000L) { throw 'unexpected nested resource directory' }
      if ([uint64]$language.Offset + 16 -gt $resourceSize) { throw 'resource data outside bounds' }
      $data = $base + $language.Offset
      $offset = RvaOffset (U32 $data)
      $length = U32 ($data + 4)
      if (!$length -or [uint64]$offset + $length -gt $bytes.Length) { throw 'empty or truncated icon resource' }
      @{ Id = $name.Name; At = $offset; Length = $length }
    }
  }
}
$root = @(Entries 0)
$iconType = @($root | Where-Object Name -eq 3)
$groupType = @($root | Where-Object Name -eq 14)
if ($iconType.Count -ne 1 -or $groupType.Count -ne 1) { throw 'missing RT_ICON or RT_GROUP_ICON' }
$icons = @(Payloads $iconType[0])
$groups = @(Payloads $groupType[0])
foreach ($group in $groups) {
  if ((U16 $group.At) -ne 0 -or (U16 ($group.At + 2)) -ne 1) { throw 'invalid icon group' }
  $count = U16 ($group.At + 4)
  if (!$count -or 6 + 14 * $count -gt $group.Length) { throw 'empty or truncated icon group' }
  for ($i = 0; $i -lt $count; $i++) {
    $entry = $group.At + 6 + 14 * $i
    $id = U16 ($entry + 12)
    $length = U32 ($entry + 8)
    if (!@($icons | Where-Object { $_.Id -eq $id -and $_.Length -eq $length }).Count) { throw 'icon group refers to missing image' }
  }
}
Write-Output "ok embedded app icon: $($icons.Count) images, $($groups.Count) group(s): $Path"
