[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$checkMark = [char]0x2713

function Format-Duration {
  param([TimeSpan]$Duration)

  $culture = [Globalization.CultureInfo]::InvariantCulture

  if ($Duration.TotalMinutes -ge 1) {
    return [string]::Format($culture, "{0}m {1:00.00}s", [Math]::Floor($Duration.TotalMinutes), $Duration.TotalSeconds % 60)
  }

  return [string]::Format($culture, "{0:0.00}s", $Duration.TotalSeconds)
}

function Write-CenteredBox {
  param(
    [string]$Text,
    [int]$Width = 46
  )

  $content = if ($Text.Length -gt $Width) { $Text.Substring(0, $Width) } else { $Text }
  $remaining = $Width - $content.Length
  $line = $content.PadLeft($content.Length + [Math]::Floor($remaining / 2)).PadRight($Width)
  $border = "".PadRight($Width, [char]'-')

  Write-Host "  +$border+" -ForegroundColor Cyan
  Write-Host "  |$line|" -ForegroundColor Cyan
  Write-Host "  +$border+" -ForegroundColor Cyan
}

function Write-Divider {
  Write-Host ("  " + "".PadRight(46, [char]'-')) -ForegroundColor DarkGray
}

function Write-Summary {
  param(
    [string]$Artifact,
    [string]$Version,
    [string]$BuildTime,
    [bool]$Succeeded
  )

  Write-Divider
  Write-Host ""
  Write-Host "  Artifact" -ForegroundColor DarkGray
  Write-Host "  $Artifact" -ForegroundColor White
  Write-Host ""
  Write-Host "  Version" -ForegroundColor DarkGray
  Write-Host "  $Version" -ForegroundColor White
  Write-Host ""
  Write-Host "  Build Time" -ForegroundColor DarkGray
  Write-Host "  $BuildTime" -ForegroundColor White
  Write-Host ""
  Write-Host "  Status" -ForegroundColor DarkGray

  if ($Succeeded) {
    Write-Host "  SUCCESS" -ForegroundColor Green
  }
  else {
    Write-Host "  FAILED" -ForegroundColor Red
  }

  Write-Host ""
}

$projectRoot = Split-Path -Parent $PSScriptRoot
$tauriConfig = Get-Content (Join-Path $projectRoot "src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
$applicationName = $tauriConfig.productName
$applicationVersion = $tauriConfig.version
$mainBinaryName = $tauriConfig.mainBinaryName
$releaseExecutable = Join-Path $projectRoot "src-tauri\target\release\$mainBinaryName.exe"
$distDirectory = Join-Path $projectRoot "dist"
$destinationExecutable = Join-Path $distDirectory "$mainBinaryName.exe"
$outputLog = Join-Path ([IO.Path]::GetTempPath()) "unreal-tools-build-$PID-out.log"
$errorLog = Join-Path ([IO.Path]::GetTempPath()) "unreal-tools-build-$PID-err.log"

try {
  Clear-Host
  Write-Host ""
  Write-CenteredBox "$applicationName Builder $applicationVersion"
  Write-Host ""

  $startedAt = Get-Date
  $spinnerFrames = @("|", "/", "-", "\")
  $spinnerIndex = 0
  $activityPrefix = "  : Building application"

  $buildProcess = Start-Process `
    -FilePath "cmd.exe" `
    -ArgumentList @("/d", "/c", "pnpm tauri build") `
    -WorkingDirectory $projectRoot `
    -NoNewWindow `
    -PassThru `
    -RedirectStandardOutput $outputLog `
    -RedirectStandardError $errorLog

  while (-not $buildProcess.HasExited) {
    $elapsed = (Get-Date) - $startedAt
    $status = "{0}  [{1}]  {2}" -f $activityPrefix, $spinnerFrames[$spinnerIndex % $spinnerFrames.Count], (Format-Duration $elapsed)
    Write-Host -NoNewline ("`r" + $status.PadRight(54))
    Start-Sleep -Milliseconds 120
    $buildProcess.Refresh()
    $spinnerIndex++
  }

  $buildProcess.WaitForExit()
  $elapsed = (Get-Date) - $startedAt
  $buildTime = Format-Duration $elapsed

  if ($buildProcess.ExitCode -ne 0) {
    Write-Host ("`r  x Building application".PadRight(54)) -ForegroundColor Red
    Write-Summary -Artifact "No artifact generated" -Version $applicationVersion -BuildTime $buildTime -Succeeded $false
    Get-Content -LiteralPath $outputLog -ErrorAction SilentlyContinue
    Get-Content -LiteralPath $errorLog -ErrorAction SilentlyContinue | Write-Host -ForegroundColor Red
    exit $buildProcess.ExitCode
  }

  New-Item -ItemType Directory -Path $distDirectory -Force | Out-Null

  if (-not (Test-Path -LiteralPath $releaseExecutable -PathType Leaf)) {
    throw "The release executable was not found at '$releaseExecutable'."
  }

  Copy-Item -LiteralPath $releaseExecutable -Destination $destinationExecutable -Force
  Write-Host (("`r  $checkMark Building application").PadRight(54)) -ForegroundColor Green
  Write-Summary -Artifact "dist\$mainBinaryName.exe" -Version $applicationVersion -BuildTime $buildTime -Succeeded $true
}
finally {
  Remove-Item -LiteralPath $outputLog, $errorLog -Force -ErrorAction SilentlyContinue
}
