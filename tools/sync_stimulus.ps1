# Stimulus de synchro A/V : 6 flashs blancs + bips simultanés, espacés de silences.
# Lancer pendant que Clipper tourne, puis sauvegarder un clip et le mesurer :
#   cargo run --example sync_check -- <clip.mp4>
# Limite : SoundPlayer joue le bip 30 à 120 ms après Play() et le tronque (~40 ms).
# Le décalage A/V du clip inclut donc cette latence ; les instants QPC écrits dans
# target/stimulus.log permettent de la séparer de celle de Clipper (cf. docs/scope.md).

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$log = Join-Path $PSScriptRoot "../target/stimulus.log"
Remove-Item $log -ErrorAction SilentlyContinue

# Bip de 100 ms à 1 kHz, volume 30 %, en WAV PCM 48 kHz mono.
$rate = 48000; $n = $rate / 10
$ms = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter($ms)
$w.Write([Text.Encoding]::ASCII.GetBytes("RIFF")); $w.Write([int](36 + 2 * $n))
$w.Write([Text.Encoding]::ASCII.GetBytes("WAVEfmt ")); $w.Write([int]16); $w.Write([int16]1); $w.Write([int16]1)
$w.Write([int]$rate); $w.Write([int](2 * $rate)); $w.Write([int16]2); $w.Write([int16]16)
$w.Write([Text.Encoding]::ASCII.GetBytes("data")); $w.Write([int](2 * $n))
for ($i = 0; $i -lt $n; $i++) { $w.Write([int16](0.3 * 32767 * [Math]::Sin(2 * [Math]::PI * 1000 * $i / $rate))) }
$ms.Position = 0
$player = New-Object System.Media.SoundPlayer($ms); $player.Load()

$form = New-Object System.Windows.Forms.Form
# Écran principal : c'est celui que Clipper capture (CenterScreen peut choisir un autre écran).
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$form.FormBorderStyle = "None"; $form.TopMost = $true; $form.StartPosition = "Manual"
$form.Size = New-Object System.Drawing.Size(900, 900); $form.BackColor = "Black"
$form.Location = New-Object System.Drawing.Point(($screen.X + ($screen.Width - 900) / 2), ($screen.Y + ($screen.Height - 900) / 2))
$form.Show(); $form.Refresh(); Start-Sleep -Milliseconds 800

foreach ($pause in 900, 1300, 700, 2500, 1100, 1600) {
    Start-Sleep -Milliseconds $pause
    $form.BackColor = "White"; $form.Refresh()
    $flash = [Diagnostics.Stopwatch]::GetTimestamp()
    $player.Play()
    $play = [Diagnostics.Stopwatch]::GetTimestamp()
    # Instants QPC en 100 ns, même horloge que les ts de Clipper.
    $k = 1e7 / [Diagnostics.Stopwatch]::Frequency
    "flash {0:F0} play {1:F0}" -f ($flash * $k), ($play * $k) | Out-File -Append -Encoding utf8 $log
    Start-Sleep -Milliseconds 200
    $form.BackColor = "Black"; $form.Refresh()
}
Start-Sleep -Milliseconds 500
$form.Close()
