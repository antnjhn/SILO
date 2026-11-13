# SILO Setup — build script (standalone)
# ---------------------------------------
# Compiles installer.nsi with makensis into dist\SILO-Setup-<version>.exe.
# Nothing in the SILO app source is built, modified or touched.
#
# Usage:
#   .\build.ps1
#   .\build.ps1 -Makensis "C:\Program Files (x86)\NSIS\makensis.exe"

param(
    [string]$Makensis = "",
    [string]$AppSource = ""
)

$ErrorActionPreference = 'Stop'

$here = $PSScriptRoot
$repo = Split-Path $here -Parent
$source = if ($AppSource) { $AppSource } else { Join-Path $repo 'src-tauri\target\release\silo.exe' }
$icon   = Join-Path $here 'assets\installer-icon.png'
if (-not (Test-Path $icon)) { $icon = Join-Path $repo 'assets\icon.png' }
$gen    = Join-Path $here 'gen'
$dist   = Join-Path $here 'dist'

Write-Host "SILO Setup build" -ForegroundColor Cyan

# ── 1. Prerequisites ───────────────────────────────────────────────────────
if (-not (Test-Path $source)) {
    throw "silo.exe not found at $source. Build SILO first (npm run build) or pass -AppSource <path>."
}
if (-not (Test-Path $icon)) {
    throw "Logo not found at $icon."
}
if (-not $Makensis) {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'),
        'C:\Program Files (x86)\NSIS\makensis.exe',
        'C:\Program Files\NSIS\makensis.exe'
    )
    $Makensis = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Makensis) {
        throw "makensis.exe not found. Install NSIS or pass -Makensis <path>."
    }
}
Write-Host "  makensis : $Makensis"
Write-Host "  silo.exe : $source"

# ── 2. Logo bitmaps ────────────────────────────────────────────────────────
# The installer draws its logo as a plain 24-bit BMP. The brand PNG sits on a
# white field, so light desaturated pixels (the white field and its
# anti-aliased fringe) are flattened to black; the purple mark and the plate
# are preserved, which leaves the mark floating on the installer's OLED black.
New-Item -ItemType Directory -Force -Path $gen | Out-Null

Add-Type -AssemblyName System.Drawing

function New-LogoBmp {
    param([string]$Source, [string]$Out, [int]$CanvasW, [int]$CanvasH, [int]$MaxW)

    $img = [System.Drawing.Image]::FromFile($Source)
    $bmp = New-Object System.Drawing.Bitmap($CanvasW, $CanvasH, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb(255, 13, 13, 16))
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

    $scale = [Math]::Min($MaxW / $img.Width, $CanvasH / $img.Height)
    $dw = [int]($img.Width * $scale)
    $dh = [int]($img.Height * $scale)
    $g.DrawImage($img, [int](($CanvasW - $dw) / 2), [int](($CanvasH - $dh) / 2), $dw, $dh)
    $g.Dispose()
    $img.Dispose()

    $rect = New-Object System.Drawing.Rectangle(0, 0, $CanvasW, $CanvasH)
    $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadWrite, $bmp.PixelFormat)
    $bytes = New-Object byte[] ($data.Stride * $CanvasH)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)

    for ($i = 0; $i -lt $bytes.Length - 3; $i += 3) {
        $b = $bytes[$i]; $gr = $bytes[$i + 1]; $r = $bytes[$i + 2]
        $max = [Math]::Max($r, [Math]::Max($gr, $b))
        $min = [Math]::Min($r, [Math]::Min($gr, $b))
        if ($min -gt 90 -and ($max - $min) -lt 40) {
            $bytes[$i] = 0; $bytes[$i + 1] = 0; $bytes[$i + 2] = 0
        }
    }

    [System.Runtime.InteropServices.Marshal]::Copy($bytes, 0, $data.Scan0, $bytes.Length)
    $bmp.UnlockBits($data)
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $bmp.Dispose()
}


# Small 18x18 toggle bitmaps for the "Launch SILO" row.
function New-CheckBmp {
    param([string]$Out, [bool]$Checked)
    $bmp = New-Object System.Drawing.Bitmap(18, 18)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Black)
    if ($Checked) {
        $g.FillRectangle((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255,124,77,255))), 0, 0, 17, 17)
        $pen = New-Object System.Drawing.Pen([System.Drawing.Color]::White, 2)
        $g.DrawLines($pen, @(
            (New-Object System.Drawing.Point(4, 9)),
            (New-Object System.Drawing.Point(7, 12)),
            (New-Object System.Drawing.Point(13, 5))
        ))
        $pen.Dispose()
    } else {
        $g.FillRectangle((New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255,15,15,17))), 0, 0, 17, 17)
        $pen = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(255,70,70,80), 1)
        $g.DrawRectangle($pen, 0, 0, 16, 16)
        $pen.Dispose()
    }
    $g.Dispose()
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $bmp.Dispose()
}
# Soft violet gradient behind the left rail artwork.
function New-RailBmp {
    param([string]$Out, [int]$W = 250, [int]$H = 560, [string]$Icon = '')
    $bmp = New-Object System.Drawing.Bitmap($W, $H, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $rect = New-Object System.Drawing.Rectangle(0, 0, $W, $H)
    $top  = [System.Drawing.Color]::FromArgb(255, 29, 24, 52)
    $mid  = [System.Drawing.Color]::FromArgb(255, 14, 12, 22)
    $flat = [System.Drawing.Color]::FromArgb(255, 14, 12, 22)
    $band = New-Object System.Drawing.Rectangle(0, 0, $W, 250)
    $grad = New-Object System.Drawing.Drawing2D.LinearGradientBrush($band, $top, $mid, [System.Drawing.Drawing2D.LinearGradientMode]::Vertical)
    $g.FillRectangle($grad, $band)
    $grad.Dispose()
    $restH = $H - 250
    $rest = New-Object System.Drawing.Rectangle(0, 250, $W, $restH)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush($flat)), $rest)

    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $r = 132
    $path.AddEllipse(125 - $r, 168 - $r, 2 * $r, 2 * $r)
    $glow = New-Object System.Drawing.Drawing2D.PathGradientBrush($path)
    $glow.CenterColor = [System.Drawing.Color]::FromArgb(60, 124, 92, 250)
    $glow.SurroundColors = @([System.Drawing.Color]::FromArgb(0, 124, 92, 250))
    $g.FillPath($glow, $path)
    $glow.Dispose()
    $path.Dispose()

    if ($Icon -and (Test-Path $Icon)) {
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $img = [System.Drawing.Image]::FromFile($Icon)
        $g.DrawImage($img, 51, 100, 148, 148)
        $img.Dispose()
    }

    $g.Dispose()

    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $bmp.Dispose()
}
New-RailBmp -Out (Join-Path $gen 'rail_bg.bmp') -Icon $icon

New-CheckBmp -Out (Join-Path $gen 'check_on.bmp')  -Checked $true
New-CheckBmp -Out (Join-Path $gen 'check_off.bmp') -Checked $false
Write-Host "  logo bmps: $gen"

# ── 3. Compile ─────────────────────────────────────────────────────────────
New-Item -ItemType Directory -Force -Path $dist | Out-Null

Push-Location $here
try {
    & $Makensis /V2 'installer.nsi'
    $code = $LASTEXITCODE
}
finally {
    Pop-Location
}

if ($code -ne 0) {
    throw "makensis failed with exit code $code"
}

# ── 4. Report ──────────────────────────────────────────────────────────────
$out = Get-ChildItem -Path $dist -Filter 'SILO-Setup-*.exe' |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1

if (-not $out) {
    throw "Installer was not produced in $dist"
}

$hash = (Get-FileHash $out.FullName -Algorithm SHA256).Hash.ToLower()
Write-Host ""
Write-Host "Installer: $($out.FullName)" -ForegroundColor Green
Write-Host "Size:      $([math]::Round($out.Length / 1MB, 2)) MB"
Write-Host "SHA-256:   $hash"
