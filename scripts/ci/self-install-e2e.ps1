# End-to-end check of skill/references/install-templates/install.ps1 against a
# local release served on loopback (ADR 0080). Windows twin of
# self-install-e2e.sh.
#
#   cargo build --features self-install --bin cfw-self-install-demo
#   pwsh scripts/ci/self-install-e2e.ps1 [path\to\cfw-self-install-demo.exe]
#
# It renders the template for the demo app, builds the release layout the
# script expects (<base>/latest.json, <base>/v<ver>/<asset>, SHA256SUMS),
# runs the rendered script with a temporary install dir, and checks that the
# binary, the receipt and a clean uninstall all behave. CI=1 keeps the user
# Path untouched. The receipt lives in the real %LOCALAPPDATA% (the known-folder
# API ignores environment overrides) and is removed by the uninstall step.
param(
  [string]$Demo
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Demo) {
  $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root 'target' }
  $Demo = Join-Path $targetDir 'debug\cfw-self-install-demo.exe'
}
if (-not (Test-Path $Demo)) { throw "demo binary not found: $Demo" }
$app = 'cfw-self-install-demo'

function Fail([string]$msg) { Write-Error $msg; exit 1 }

# Runs the rendered installer in a child pwsh so its `throw` becomes an exit
# code and stderr text instead of ending this script.
function Invoke-Installer([hashtable]$extraEnv) {
  $saved = @{}
  foreach ($k in $extraEnv.Keys) {
    $saved[$k] = [Environment]::GetEnvironmentVariable($k)
    [Environment]::SetEnvironmentVariable($k, $extraEnv[$k])
  }
  try {
    $out = & pwsh -NoProfile -NonInteractive -File $script:installer 2>&1 | Out-String
    return @{ Code = $LASTEXITCODE; Output = $out }
  } finally {
    foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
  }
}

$version = ((& $Demo --version) -split '\s+')[1]
$arch = switch ([string][System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
  'Arm64' { 'aarch64' }
  default { 'x86_64' }
}
$asset = "$app-$arch-pc-windows-msvc.zip"

$work = Join-Path ([IO.Path]::GetTempPath()) ("cfw-si-e2e-" + [guid]::NewGuid().ToString('N'))
$server = $null
try {
  # Release layout.
  $rel = Join-Path $work "release\v$version"
  $stage = Join-Path $work 'stage'
  New-Item -ItemType Directory -Force -Path $rel, $stage | Out-Null
  Copy-Item $Demo (Join-Path $stage "$app.exe")
  Compress-Archive -Path (Join-Path $stage "$app.exe") -DestinationPath (Join-Path $rel $asset)
  $hash = (Get-FileHash -Algorithm SHA256 (Join-Path $rel $asset)).Hash.ToLowerInvariant()
  Set-Content -NoNewline -Path (Join-Path $rel 'SHA256SUMS') -Value "$hash  $asset`n"
  Set-Content -Path (Join-Path $work 'release\latest.json') -Value "{`"version`": `"$version`"}"

  # Rendered script. __APP_ENV__ first: it contains __APP__ as a prefix.
  $template = Get-Content -Raw (Join-Path $root 'skill\references\install-templates\install.ps1')
  $rendered = $template.Replace('__APP_ENV__', 'CFW_SELF_INSTALL_DEMO').
    Replace('__APP__', $app).
    Replace('__REPO__', 'aroff/cli-framework').
    Replace('__TAG_PREFIX__', 'v').
    Replace('__SELF_CMD__', 'self')
  if ($rendered -match '__[A-Z_]+__') { Fail "unrendered placeholder left in install.ps1: $($Matches[0])" }
  # Make sure the rendered result parses before running it.
  $tokens = $null; $errors = $null
  [System.Management.Automation.Language.Parser]::ParseInput($rendered, [ref]$tokens, [ref]$errors) | Out-Null
  if ($errors.Count -gt 0) { Fail "rendered install.ps1 does not parse: $($errors[0])" }
  $script:installer = Join-Path $work 'install.ps1'
  Set-Content -Path $script:installer -Value $rendered

  # Loopback server on a free port.
  $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
  $listener.Start(); $port = $listener.LocalEndpoint.Port; $listener.Stop()
  $python = if (Get-Command python3 -ErrorAction SilentlyContinue) { 'python3' } else { 'python' }
  $server = Start-Process -PassThru -WindowStyle Hidden -FilePath $python `
    -ArgumentList @('-m', 'http.server', "$port", '--bind', '127.0.0.1', '--directory', (Join-Path $work 'release'))
  $base = "http://127.0.0.1:$port"
  $up = $false
  foreach ($i in 1..100) {
    try { Invoke-WebRequest -UseBasicParsing "$base/latest.json" | Out-Null; $up = $true; break } catch { Start-Sleep -Milliseconds 100 }
  }
  if (-not $up) { Fail "release server did not start on $base" }

  $binDir = Join-Path $work 'bin'
  $env:CI = '1'
  $env:CFW_SELF_INSTALL_DEMO_INSTALL_DIR = $binDir
  $env:CFW_SELF_INSTALL_DEMO_INSTALLER_BASE_URL = $base
  $pathBefore = [Environment]::GetEnvironmentVariable('Path', 'User')

  Write-Host '== install'
  $r = Invoke-Installer @{}
  Write-Host $r.Output
  if ($r.Code -ne 0) { Fail "install failed with exit code $($r.Code)" }
  $bin = Join-Path $binDir "$app.exe"
  if (-not (Test-Path $bin)) { Fail "binary not installed at $bin" }
  if (Get-ChildItem -Force $binDir | Where-Object Name -like ".$app-install-*") { Fail 'installer temp dir left behind' }
  if ([Environment]::GetEnvironmentVariable('Path', 'User') -ne $pathBefore) { Fail 'user Path was edited although CI=1' }

  Write-Host '== status'
  $status = (& $bin self status --json | Out-String)
  Write-Host $status
  $report = $status | ConvertFrom-Json
  if ($report.method -ne 'script') { Fail "method is $($report.method), not script" }
  $receipt = $report.receipt_path
  if (-not $receipt) { $receipt = Join-Path $env:LOCALAPPDATA "$app\install-receipt.json" }

  Write-Host '== checksum mismatch is refused'
  Set-Content -NoNewline -Path (Join-Path $rel 'SHA256SUMS') -Value ("0" * 64 + "  $asset`n")
  $r = Invoke-Installer @{}
  if ($r.Code -eq 0) { Fail 'install succeeded with a bad checksum' }
  if ($r.Output -notmatch 'checksum mismatch') { Fail "unexpected output: $($r.Output)" }

  Write-Host '== plain http to a non-loopback mirror is refused'
  $r = Invoke-Installer @{ CFW_SELF_INSTALL_DEMO_INSTALLER_BASE_URL = 'http://example.com' }
  if ($r.Code -eq 0) { Fail 'install accepted a plain http mirror' }
  if ($r.Output -notmatch 'must be https') { Fail "unexpected output: $($r.Output)" }

  Write-Host '== uninstall'
  & $bin self uninstall
  if ($LASTEXITCODE -ne 0) { Fail "uninstall exited with $LASTEXITCODE" }
  # The running binary is deleted by a helper after it exits.
  $deadline = (Get-Date).AddSeconds(20)
  while ((Test-Path $bin) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
  if (Test-Path $bin) { Fail 'binary still present after uninstall' }
  if (Test-Path $receipt) { Fail "receipt still present at $receipt" }
  Write-Host 'self-install e2e passed'
}
finally {
  if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
  Remove-Item -Recurse -Force -Path $work -ErrorAction SilentlyContinue
}
