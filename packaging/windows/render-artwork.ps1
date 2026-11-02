# Mechanical resizing/format conversion of the generated illustration and icon.
# No image generation service is needed to reproduce the installer bitmaps.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$art = Join-Path $PSScriptRoot 'artwork'
function Export-Bitmap([string] $Source, [string] $Destination, [int] $Width, [int] $Height, [string] $Background) {
    $sourceImage = [Drawing.Image]::FromFile($Source)
    $bitmap = [Drawing.Bitmap]::new($Width, $Height, [Drawing.Imaging.PixelFormat]::Format24bppRgb)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.Clear([Drawing.ColorTranslator]::FromHtml($Background))
        $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.DrawImage($sourceImage, 0, 0, $Width, $Height)
        $bitmap.Save($Destination, [Drawing.Imaging.ImageFormat]::Bmp)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
        $sourceImage.Dispose()
    }
}
foreach ($scale in 1, 2) {
    $suffix = if ($scale -eq 1) { '' } else { '-2x' }
    Export-Bitmap "$art/sidebar-source.png" "$art/sidebar$suffix.bmp" (202 * $scale) (386 * $scale) '#102333'
    Export-Bitmap "$PSScriptRoot/../../assets/stile.png" "$art/small$suffix.bmp" (58 * $scale) (58 * $scale) '#ffffff'
}
