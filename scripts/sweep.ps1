# Prunes stale/oversized cargo build artifacts from target/ using cargo-sweep.
# Install once: cargo install cargo-sweep
#
# Usage:
#   scripts\sweep.ps1              # cap target/ at 2GB, oldest artifacts first
#   scripts\sweep.ps1 -MaxSizeMB 500
#   scripts\sweep.ps1 -DryRun

param(
    [int]$MaxSizeMB = 2000,
    [switch]$DryRun
)

$repoRoot = Split-Path -Parent $PSScriptRoot
$sweepArgs = @('sweep', '--maxsize', $MaxSizeMB, $repoRoot)
if ($DryRun) {
    $sweepArgs = @('sweep', '--dry-run', '--maxsize', $MaxSizeMB, $repoRoot)
}

cargo @sweepArgs
