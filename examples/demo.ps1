# recallweave demo (PowerShell) — mirrors examples/demo.sh.
# Builds a small store with the real binary, drives the full lifecycle, and
# exports a memory-pack. Deterministic: every command pins --now.
#
# Run from the project root:  pwsh examples/demo.ps1

$ErrorActionPreference = "Stop"

$root  = Split-Path -Parent $PSScriptRoot
$bin   = Join-Path $root "target\release\recallweave.exe"
$store = Join-Path $root "examples\demo-store"
$pack  = Join-Path $root "examples\memory-pack.json"

if (-not (Test-Path $bin)) {
    Write-Host "building release binary..."
    Push-Location $root; cargo build --release; Pop-Location
}

function rw { & $bin --dir $store @args }

if (Test-Path $store) { Remove-Item -Recurse -Force $store }

Write-Host "== weaving semantic + preference + procedural + episodic threads =="
rw add --kind semantic --content "prod region is us-east-1" --tags infra,region --source user --now 1710000000 | Out-Null
$oldId = (rw query --kind semantic --json --now 1710000050 | Select-String -Pattern 'mem_[0-9a-f]+' | ForEach-Object { $_.Matches[0].Value } | Select-Object -First 1)

Write-Host "== the region changed: supersede the old fact =="
rw supersede --old $oldId --content "prod region is eu-west-1" --confidence 0.99 --tags infra,region --now 1710000100

Write-Host "== record some preferences (two of them will conflict) =="
rw add --kind preference --content "prefers concise answers" --tags style --confidence 0.8 --now 1710000200 | Out-Null
rw add --kind preference --content "prefers dark theme"      --tags ui    --confidence 0.7 --now 1710000200 | Out-Null
rw add --kind preference --content "prefers light theme"     --tags ui    --confidence 0.6 --now 1710000200 | Out-Null

Write-Host "== a procedural recipe and an episodic note with a 1-day TTL =="
rw add --kind procedural --content "rotate the API token by running ops runbook step 3" --tags ops --confidence 0.9 --now 1710000300 | Out-Null
rw add --kind episodic   --content "deploy failed at 14:03 due to bad env var" --tags infra --confidence 0.6 --ttl 86400 --now 1710000400 | Out-Null

Write-Host "`n== dedupe: re-adding normalized-identical content is a no-op =="
rw add --kind semantic --content "PROD region   is  eu-west-1" --now 1710000450

Write-Host "`n== list live memories =="
rw list --now 1710000500

Write-Host "`n== detected conflicts =="
rw conflicts --now 1710000500

Write-Host "`n== integrity check =="
rw verify

Write-Host "`n== stats =="
rw stats --now 1710000500

Write-Host "`n== export a portable memory-pack -> $pack =="
rw export --out $pack --now 1710000500

Write-Host "`ndone. render it with: cd viewer; npm install; npm run build; node dist/cli.js ../examples/memory-pack.json --svg ../docs/assets/tapestry-demo.svg"
