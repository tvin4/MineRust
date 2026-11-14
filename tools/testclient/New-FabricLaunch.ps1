# Builds the launch command for the Fabric-modded 26.3 test client
# (MineRust testclient, step 9 stage B). Layers on New-ClientLaunch.ps1:
# the vanilla command stays untouched and the Fabric layer only adds the
# loader profile libraries (official Fabric maven, SHA1-verified), the
# KnotClient main class and the mod jars in <gameDir>/mods.
#
# Everything is downloaded from official Fabric/Mojang servers only and
# cached under -FabricLibsRoot (default E:\MineRust-data\testclient-libs).
# Dot-sourceable: returns a hashtable like New-ClientLaunch.ps1.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Nick,
    [string]$Address = '127.0.0.1:25565',
    [int]$Width = 1280,
    [int]$Height = 720,
    # Control socket port handed to the mod (-Dtestclient.port).
    [int]$ControlPort = 25571,
    # Fabric toolchain versions: same values as mod/gradle.properties
    # (official Fabric sources, checked 14.11.2026).
    [string]$GameVersion = '26.3',
    [string]$LoaderVersion = '0.20.1',
    [string]$FabricApiVersion = '0.166.2+26.3',
    # Official sources only.
    [string]$MetaBase = 'https://meta.fabricmc.net/v2',
    [string]$FabricMaven = 'https://maven.fabricmc.net',
    # Cache for the Fabric loader/libraries (E: - the C: drive is tight).
    [string]$FabricLibsRoot = 'E:\MineRust-data\testclient-libs',
    # Base dir for per-client game dirs, forwarded to New-ClientLaunch.ps1.
    [string]$ClientsRoot = 'E:\MineRust-data\clients',
    # Built mod jar to install into <gameDir>/mods.
    [string]$ModJar = (Join-Path $PSScriptRoot 'mod\build\libs\testclient-mod-0.1.0.jar')
)
$ErrorActionPreference = 'Stop'

# --- vanilla launch data (manifest, cache, natives - all already verified) --
$vanilla = & (Join-Path $PSScriptRoot 'New-ClientLaunch.ps1') -Nick $Nick -Address $Address `
    -Width $Width -Height $Height -ClientsRoot $ClientsRoot

if (-not (Test-Path $ModJar)) { throw "mod jar not built: $ModJar (run gradle build in tools/testclient/mod)" }
if (-not (Test-Path $FabricLibsRoot)) { New-Item -ItemType Directory -Path $FabricLibsRoot | Out-Null }

# --- official Fabric loader profile (meta.fabricmc.net) --------------------
# GET /v2/versions/loader/<game>/<loader>/profile/json is the same document
# the Fabric installer writes into .minecraft/versions.
$profilePath = Join-Path $FabricLibsRoot "fabric-loader-$LoaderVersion-$GameVersion.json"
if (-not (Test-Path $profilePath)) {
    $uri = "$MetaBase/versions/loader/$GameVersion/$LoaderVersion/profile/json"
    Invoke-WebRequest -Uri $uri -OutFile $profilePath -UseBasicParsing
}
$profile = Get-Content $profilePath -Raw | ConvertFrom-Json

# --- loader libraries: official Fabric maven, cached + SHA1-verified -------
function Get-MavenJar([string]$name, [string]$baseUrl, [string]$cacheRoot) {
    # name is "group:artifact:version"; the maven path derives from it.
    $parts = $name -split ':'
    if ($parts.Count -ne 3) { throw "bad maven coordinate: $name" }
    $rel = ($parts[0] -replace '\.', '/') + '/' + $parts[1] + '/' + $parts[2] + '/' + $parts[1] + '-' + $parts[2] + '.jar'
    $dest = Join-Path $cacheRoot ($parts[1] + '-' + $parts[2] + '.jar')
    if (-not (Test-Path $dest)) {
        Invoke-WebRequest -Uri ($baseUrl + '/' + $rel) -OutFile $dest -UseBasicParsing
        $sha1Resp = Invoke-WebRequest -Uri ($baseUrl + '/' + $rel + '.sha1') -UseBasicParsing
        $want = [System.Text.Encoding]::UTF8.GetString($sha1Resp.Content).Trim()
        $got = (Get-FileHash $dest -Algorithm SHA1).Hash.ToLower()
        if ($got -ne $want.ToLower()) {
            Remove-Item $dest -Force
            throw "SHA1 mismatch for $name ($got != $want)"
        }
    }
    return $dest
}

$fabricJars = New-Object System.Collections.Generic.List[string]
foreach ($lib in $profile.libraries) {
    $fabricJars.Add((Get-MavenJar ([string]$lib.name) $FabricMaven $FabricLibsRoot))
}

# --- mods: our mod jar + the Fabric API umbrella jar ------------------------
$modsDir = Join-Path $vanilla.GameDir 'mods'
if (-not (Test-Path $modsDir)) { New-Item -ItemType Directory -Path $modsDir | Out-Null }
Copy-Item $ModJar (Join-Path $modsDir 'testclient-mod.jar') -Force
$apiJar = Get-MavenJar "net.fabricmc.fabric-api:fabric-api:$FabricApiVersion" $FabricMaven $FabricLibsRoot
Copy-Item $apiJar (Join-Path $modsDir (Split-Path -Leaf $apiJar)) -Force

# --- classpath and jvm args: fabric loader libs in front of the vanilla set -
# The vanilla manifest's ${classpath} is already expanded inside JvmArgs;
# the classpath string itself is available as Classpath, so the Fabric jars
# are prepended by replacing the exact string.
$newClasspath = (($fabricJars -join ';') + ';' + $vanilla.Classpath)
$jvmArgs = New-Object System.Collections.Generic.List[string]
foreach ($a in $vanilla.JvmArgs) { $jvmArgs.Add(($a.Replace($vanilla.Classpath, $newClasspath))) }
# The loader profile's own jvm argument (emulates the vanilla main class),
# verbatim from the official profile document.
$jvmArgs.Add([string]($profile.arguments.jvm | Select-Object -First 1))
$jvmArgs.Add("-Dtestclient.port=$ControlPort")

return @{
    JavaExe    = $vanilla.JavaExe
    MainClass  = [string]$profile.mainClass
    Classpath  = $newClasspath
    JvmArgs    = $jvmArgs
    GameArgs   = $vanilla.GameArgs
    GameDir    = $vanilla.GameDir
    NativesDir = $vanilla.NativesDir
    Uuid       = $vanilla.Uuid
    ControlPort = $ControlPort
}
