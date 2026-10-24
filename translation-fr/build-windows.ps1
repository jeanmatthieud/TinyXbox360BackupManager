param(
    [Parameter(Mandatory = $true)][string]$MingwBin,
    [switch]$TestCore,
    [string]$LlvmWindres
)
$ErrorActionPreference = 'Stop'
$compiler = (Resolve-Path -LiteralPath $MingwBin).Path
if ($compiler.Contains(' ')) {
    throw 'Placez MinGW dans un chemin sans espaces (limitation de GCC).'
}
if (!(Test-Path -LiteralPath (Join-Path $compiler 'gcc.exe'))) {
    throw 'Le dossier doit contenir gcc.exe, ar.exe et windres.exe.'
}
$env:PATH = "$compiler;$env:PATH"
$env:CC = 'gcc'
$env:AR = 'ar'
if ($LlvmWindres) { $env:WINDRES = (Resolve-Path -LiteralPath $LlvmWindres).Path }
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = 'gcc'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = '-C link-arg=-Wl,--stack,8000000'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    if ($TestCore) {
        cargo test --locked --release --target x86_64-pc-windows-gnu -p txbm-core --lib
    } else {
        cargo build --locked --release --target x86_64-pc-windows-gnu
    }
    if ($LASTEXITCODE -ne 0) { throw "Compilation : code $LASTEXITCODE" }
} finally {
    Pop-Location
}
