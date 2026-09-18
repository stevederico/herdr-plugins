# open-git-panel.ps1 -- Windows launcher for the herdr-aa-git source control pane.
#
# Idempotent "launch-or-focus, toggle on repeat", scoped to the current tab:
#   - no Source Control pane in the current tab      -> open one, RIGHT OF THE AGENT
#   - a Source Control pane exists but isn't focused -> focus it
#   - the focused pane IS the Source Control pane    -> close it (toggle off)
#
# Right dock: split the agent pane to the right. `--ratio` is the ORIGINAL
# pane's share, so a large ratio (~0.75) leaves the new pane ~32 columns on
# the agent's right. No swap.
#
# Windows caveats inherited from herdr-file-viewer (see its herdr-plugin.toml):
# herdr cannot spawn a relative [[panes]] command on Windows (ERROR_PATH_NOT_FOUND),
# so we spawn the binary BY ABSOLUTE PATH via `pane split` + `pane run`, and the
# pane-id / target / ratio decisions come from the binary's tested stdin modes
# (--launch-decision git / --focused-pane / --open-plan), never from ad-hoc parsing.

$ErrorActionPreference = 'Continue'

# PowerShell 5.1 otherwise decodes herdr's UTF-8 JSON with the legacy console code
# page; non-ASCII pane titles or paths would corrupt the JSON.
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[Console]::OutputEncoding = $Utf8NoBom
$OutputEncoding = $Utf8NoBom

$HerdrBin = if ($env:HERDR_BIN_PATH) { $env:HERDR_BIN_PATH } else { 'herdr' }

function Strip-Verbatim([string]$p) {
    if ($p -and $p.StartsWith('\\?\')) { return $p.Substring(4) }
    return $p
}
$PluginRoot = Strip-Verbatim (Split-Path -Parent $PSScriptRoot)
$Bin = Join-Path $PluginRoot 'target\release\herdr-sidebar.exe'

if (-not (Test-Path $Bin)) {
    Write-Error "herdr-sidebar.exe not found at $Bin -- run 'cargo build --release' in the plugin directory first."
    exit 1
}

# Extract the first `pane_id` from a herdr CLI JSON reply.
function Get-PaneId([string]$json) {
    return ([regex]'"pane_id":"([^"]+)"').Match($json).Groups[1].Value
}

$PanesJson = (& $HerdrBin pane list | Out-String)

function Open-Pane {
    # Focused pane = where the user is working; its cwd picks the repository.
    $fp = ($PanesJson | & $Bin --focused-pane).Trim()
    if (-not $fp) {
        # No focused pane known: best-effort plain split beside the current pane.
        $out = (& $HerdrBin pane split --current --direction right --ratio 0.75 | Out-String)
        $np = Get-PaneId $out
        if ($np) { & $HerdrBin pane run $np "& \`"$Bin\`" --view git" }
        exit 0
    }
    $FocusedId, $FocusedCwd = $fp -split "`t", 2

    $Target = ($PanesJson | & $Bin --agent-pane).Trim()
    if (-not $Target) { $Target = $FocusedId }
    $Ratio = '0.75'
    $plan = ((& $HerdrBin pane layout --pane $Target | Out-String) | & $Bin --open-plan $Target).Trim()
    if ($plan) { $Target, $Ratio = $plan -split "`t", 2 }

    $splitArgs = @('pane', 'split', $Target, '--direction', 'right', '--ratio', $Ratio, '--no-focus')
    if ($FocusedCwd) { $splitArgs += @('--cwd', $FocusedCwd) }
    $out = (& $HerdrBin @splitArgs | Out-String)
    $np = Get-PaneId $out
    if (-not $np) { exit 1 }

    # Absolute path via the PowerShell CALL OPERATOR: a bare path splits on spaces
    # in the install path, and the `\"` escaping survives PS 5.1's native-arg
    # quote-stripping so herdr receives the quotes intact (herdr-file-viewer GH #58).
    & $HerdrBin pane run $np "& \`"$Bin\`" --view git"
    & $HerdrBin pane rename $np 'Source Control' *> $null
    # Wait for the TUI's identity token so queued ensure hooks see a LIVE
    # pane (the corpse rule replaces label-without-token panes).
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 200
        $tok = ((& $HerdrBin pane list --json | ConvertFrom-Json).result.panes |
            Where-Object { $_.pane_id -eq $np }).tokens
        if ($tok) { break }
    }
    # herdr has no focus-by-id; a zoom on/off cycle focuses deterministically.
    & $HerdrBin pane zoom $np --on *> $null
    & $HerdrBin pane zoom $np --off *> $null
    exit 0
}

$Decision = ($PanesJson | & $Bin --launch-decision git 2>$null)
if ($LASTEXITCODE -ne 0 -or -not $Decision) { $Decision = 'OPEN' }
$Decision = $Decision.Trim()

if ($Decision -like 'FOCUS *') {
    $PaneId = $Decision.Substring(6)
    & $HerdrBin pane zoom $PaneId --on *> $null
    & $HerdrBin pane zoom $PaneId --off
    exit $LASTEXITCODE
} elseif ($Decision -like 'CLOSE *') {
    $PaneId = $Decision.Substring(6)
    & $HerdrBin pane close $PaneId
    exit $LASTEXITCODE
} elseif ($Decision -like 'REPLACE *') {
    # Dead pane (stale heartbeat): close the corpse, then dock a fresh one.
    $PaneId = $Decision.Substring(8)
    & $HerdrBin pane close $PaneId *> $null
    Open-Pane
} else {
    Open-Pane
}
