<#
.SYNOPSIS
Generates the game's own data reports for MineRust (reports/packets.json,
reports/registries.json, ...).

.DESCRIPTION
The data generator (net.minecraft.data.Main) ships inside the client jar of
your own copy of the game. This script builds the classpath from the
launcher's version json (all libraries that exist on disk), writes it into a
permanent @argument file (PowerShell would break a semicolon-separated class
path into several arguments), and runs the generator with --reports.

No game data is committed to the repository: the output directory is ignored
by git and only pointed at by the optional packets_report setting of
server.toml.

.EXAMPLE
tools\generate-reports.ps1 -VersionJson "$env:APPDATA\ElyPrismLauncher\meta\net.minecraft\26.3.json" -Output "$env:TEMP\datagen"

.OUTPUTS
<Output>\reports\packets.json, <Output>\reports\registries.json and the
rest of the --reports output.

.NOTES
Requires Java on PATH (or -Java) able to run this game version.
#>
[CmdletBinding()]
param(
    # Version json of the installed game version (the launcher's meta file).
    [Parameter(Mandatory = $true)]
    [string]$VersionJson,

    # Root of the launcher's `libraries` folder. Guesses when omitted.
    [string]$LibrariesRoot,

    # Client jar; defaults to the one declared by the version json.
    [string]$ClientJar,

    # Java executable; defaults to whatever `java` resolves to.
    [string]$Java = "java",

    # Output directory for reports/ and the other generator output.
    [string]$Output = (Join-Path $PSScriptRoot "..\generated")
)

$ErrorActionPreference = "Stop"

function Resolve-LibrariesRoot([string]$jsonPath) {
    if ($LibrariesRoot) { return (Resolve-Path $LibrariesRoot).Path }
    # The launcher that owns the version json is somewhere above its folder
    # (`<root>\meta\<uid>\26.3.json`, `<root>\versions\...\). Walk a few
    # levels up first; guessing across launchers would pick the wrong
    # libraries folder.
    $candidates = @()
    $folder = Split-Path $jsonPath -Parent
    for ($depth = 0; $depth -lt 4; $depth++) {
        if (-not $folder) { break }
        $candidates += (Join-Path $folder "libraries")
        $folder = Split-Path $folder -Parent
    }
    $candidates += (Join-Path $env:APPDATA ".minecraft\libraries")
    $candidates += (Join-Path $env:APPDATA "ElyPrismLauncher\libraries")
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) {
            return (Resolve-Path $candidate).Path
        }
    }
    throw "libraries folder not found; pass -LibrariesRoot explicitly (tried: $($candidates -join '; '))"
}

# maven coordinate "group:artifact:version[:classifier]" -> jar path under
# the libraries root. The dots of the group stay dots in the folder names.
function Get-LibraryPath([string]$librariesRoot, [string]$coordinate) {
    $parts = $coordinate -split ':'
    if ($parts.Count -lt 3) {
        throw "unexpected maven coordinate in the version json: $coordinate"
    }
    $classifier = ""
    if ($parts.Count -ge 4) { $classifier = "-$($parts[3])" }
    $groupDir = $parts[0] -replace "\.", [IO.Path]::DirectorySeparatorChar
    $fileName = "$($parts[1])-$($parts[2])$classifier.jar"
    return (Join-Path (Join-Path (Join-Path $librariesRoot $groupDir) $parts[1]) (Join-Path $parts[2] $fileName))
}

if (-not (Test-Path $VersionJson)) {
    throw "version json not found: $VersionJson"
}
$VersionJson = (Resolve-Path $VersionJson).Path
$librariesRoot = Resolve-LibrariesRoot $VersionJson
Write-Host "libraries: $librariesRoot"

$version = Get-Content $VersionJson -Raw | ConvertFrom-Json
if (-not $version.libraries) {
    throw "no libraries section in $VersionJson (is this a launcher version json?)"
}

# The classpath: declared libraries that are present on disk (the platform
# ones the launcher did not download are skipped) plus the client jar.
$entries = @()
$skipped = 0
foreach ($library in $version.libraries) {
    if (-not $library.name) { continue }
    $path = Get-LibraryPath $librariesRoot $library.name
    if (Test-Path $path) {
        $entries += $path
    } else {
        $skipped++
        Write-Verbose "skipped (not downloaded): $($library.name)"
    }
}

if (-not $ClientJar) {
    if ($version.mainJar -and $version.mainJar.name) {
        $ClientJar = Get-LibraryPath $librariesRoot $version.mainJar.name
    }
}
if (-not $ClientJar -or -not (Test-Path $ClientJar)) {
    throw "client jar not found: $ClientJar; pass -ClientJar explicitly"
}
$entries = @((Resolve-Path $ClientJar).Path) + $entries
Write-Host "classpath entries: $($entries.Count) ($skipped libraries skipped)"

# An @argument file keeps the long semicolon-separated class path out of the
# command line. UTF-8 without a BOM: Java reads the file as UTF-8 and a BOM
# would become part of the first argument.
$Output = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
New-Item -ItemType Directory -Force -Path $Output | Out-Null
$argFile = Join-Path $Output "java-args.txt"
$classPath = [string]::Join([IO.Path]::PathSeparator, $entries)
[IO.File]::WriteAllText($argFile, "-cp $classPath", (New-Object Text.UTF8Encoding($false)))

# The data generator's entry point is stable across recent game versions and
# lives in the client jar itself. It runs with the output directory as the
# working directory: the generator drops its own `logs/` folder into the
# current directory, which must not pollute the repository.
Write-Host "generating reports into $Output ..."
Push-Location $Output
try {
    & $Java "@$argFile" net.minecraft.data.Main --reports --output $Output
    $code = $LASTEXITCODE
} finally {
    Pop-Location
}
if ($code -ne 0) {
    throw "the data generator exited with code $code"
}

$packets = Join-Path (Join-Path $Output "reports") "packets.json"
if (-not (Test-Path $packets)) {
    throw "expected report missing: $packets"
}
Write-Host ""
Write-Host "done. Point server.toml at the report:"
Write-Host "  packets_report = `"$($packets -replace '\\','/')`""