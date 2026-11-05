# Packs the Stream Deck plugin into dist/com.fayewave.hyperlight.streamDeckPlugin
# (a zip of the .sdPlugin folder; double-click it to install).
#
#   pwsh tools/pack-streamdeck.ps1
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem

$root = Split-Path $PSScriptRoot -Parent
$src = Join-Path $root 'streamdeck\com.fayewave.hyperlight.sdPlugin'
$dist = Join-Path $root 'dist'
$out = Join-Path $dist 'com.fayewave.hyperlight.streamDeckPlugin'

New-Item -ItemType Directory -Force $dist | Out-Null
if (Test-Path $out) { Remove-Item $out }
[System.IO.Compression.ZipFile]::CreateFromDirectory($src, $out, 'Optimal', $true)
"packed $out ($((Get-Item $out).Length) bytes)"
