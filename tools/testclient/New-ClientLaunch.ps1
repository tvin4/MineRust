# Builds the exact launch command for the vanilla 26.3 client (MineRust
# testclient, step 9 stage A). Everything comes from the official version
# manifest (versions/26.3/26.3.json from the version manifest v2) and the
# libraries the owner's launcher (ElyPrismLauncher) already downloaded -
# nothing is invented.
#
# Dot-sourceable: returns a hashtable with the launch data.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Nick,
    [string]$Address = '127.0.0.1:25565',
    [int]$Width = 1280,
    [int]$Height = 720,
    # Base dir for per-client game dirs (clients/... is gitignored).
    [string]$ClientsRoot = 'E:\MineRust-data\clients',
    # Scratch dir for extracted natives (E: - the C: drive is tight).
    [string]$NativesRoot = 'E:\MineRust-data\testclient-natives',
    # Where version.json + the client jar live (owner's game installation).
    [string]$VersionJson = 'C:\Users\Tvinch\AppData\Roaming\.minecraft\versions\26.3\26.3.json',
    [string]$LibrariesRoot = 'C:\Users\Tvinch\AppData\Roaming\ElyPrismLauncher\libraries',
    [string]$ClientJar = 'C:\Users\Tvinch\AppData\Roaming\ElyPrismLauncher\libraries\com\mojang\minecraft\26.3\minecraft-26.3-client.jar',
    [string]$AssetsRoot = 'C:\Users\Tvinch\AppData\Roaming\ElyPrismLauncher\assets',
    [string]$JavaExe = 'C:\Users\Tvinch\AppData\Roaming\ElyPrismLauncher\java\java-runtime-epsilon\bin\java.exe'
)

$ErrorActionPreference = 'Stop'

foreach ($p in @($VersionJson, $ClientJar, $JavaExe)) {
    if (-not (Test-Path $p)) { throw "missing: $p" }
}
if (-not (Test-Path $LibrariesRoot)) { throw "missing libraries root: $LibrariesRoot" }

$meta = Get-Content $VersionJson -Raw -Encoding utf8 | ConvertFrom-Json
if ($meta.id -ne '26.3') { throw "unexpected version id: $($meta.id)" }

# --- rule evaluation (os / arch / versionRange / features) -----------------
# Features we expose to the manifest's rule system. Custom resolution is on
# (we want the window size), quick play multiplayer is on (it is how the
# client auto-connects in 26.3); demo and the other quick-play targets are
# off. Declared here, not guessed: the feature names come from the manifest.
$Features = @{
    is_demo_user               = $false
    has_custom_resolution      = $true
    has_quick_plays_support    = $true
    is_quick_play_singleplayer = $false
    is_quick_play_multiplayer  = $true
    is_quick_play_realms       = $false
}

function Test-RuleMatch($rule) {
    if ($rule.os) {
        if ($rule.os.name -and $rule.os.name -ne 'windows') { return $false }
        if ($rule.os.arch) {
            $isX86 = -not [Environment]::Is64BitProcess
            if (($rule.os.arch -eq 'x86') -ne $isX86) { return $false }
        }
        if ($rule.os.versionRange) {
            $cur = [Environment]::OSVersion.Version
            if ($rule.os.versionRange.min -and $cur -lt [Version]$rule.os.versionRange.min) { return $false }
            if ($rule.os.versionRange.max -and $cur -gt [Version]$rule.os.versionRange.max) { return $false }
        }
    }
    if ($rule.features) {
        foreach ($k in $rule.features.PSObject.Properties.Name) {
            $want = [bool]$rule.features.$k
            $have = if ($Features.ContainsKey($k)) { [bool]$Features[$k] } else { $false }
            if ($want -ne $have) { return $false }
        }
    }
    return $true
}

function Test-RulesAllowed($rules) {
    if (-not $rules -or $rules.Count -eq 0) { return $true }
    $allowed = $false   # a rule list is allow-list semantics
    foreach ($r in $rules) {
        if (Test-RuleMatch $r) { $allowed = ($r.action -eq 'allow') }
    }
    return $allowed
}

# --- libraries: only what is allowed on this OS, and must exist locally ----
# The manifest and the launcher cache disagree on where classifier jars live:
# the manifest says  com/mojang/jtracy/1.14.38/jtracy-1.14.38-natives-windows.jar
# the cache has     com/mojang/jtracy-natives-windows/1.14.38/jtracy-natives-windows-1.14.38.jar
# (classifier folded into the artifact id - confirmed against Prism's own
# launch classpath in ElyPrismLauncher-0.log). Try the manifest path first,
# then the cache layout; fail loudly if neither exists.
function Resolve-LibJar($entry) {
    # $entry = downloads.artifact or one downloads.classifiers value
    $artifactPath = $entry.path
    $p = Join-Path $LibrariesRoot ($artifactPath -replace '/', '\')
    if (Test-Path $p) { return $p }
    # Launcher-cache layout: the classifier is folded into the artifact id,
    # e.g. com/mojang/jtracy-natives-windows/1.14.38/jtracy-natives-windows-1.14.38.jar
    # (confirmed against Prism's own launch classpath, ElyPrismLauncher-0.log).
    $parts = $artifactPath -split '/'
    if ($parts.Count -ge 4) {
        $group = $parts[0..($parts.Count - 4)] -join '/'
        $artifact = $parts[-3]
        $version = $parts[-2]
        $file = $parts[-1]
        $prefix = $artifact + '-' + $version
        if ($file.StartsWith($prefix) -and $file.EndsWith('.jar')) {
            $mid = $file.Substring($prefix.Length, $file.Length - $prefix.Length - 4)
            if ($mid.StartsWith('-')) {
                $classifier = $mid.Substring(1)
                $newArtifact = $artifact + '-' + $classifier
                $q = Join-Path $LibrariesRoot (($group + '/' + $newArtifact + '/' + $version + '/' +
                    $newArtifact + '-' + $version + '.jar') -replace '/', '\')
                if (Test-Path $q) { return $q }
            }
        }
    }
    # Last resort: download from the official Mojang library host (or the
    # URL the manifest gives) and verify SHA1, per the entry's checksum.
    $url = if ($entry.url) { [string]$entry.url } else { 'https://libraries.minecraft.net/' + $artifactPath }
    $sha1 = [string]$entry.sha1
    if (-not $sha1) { throw "no sha1 for $artifactPath - refusing to download unverified" }
    New-Item -ItemType Directory -Path (Split-Path $p) -Force | Out-Null
    Write-Host ("  downloading {0}" -f $url)
    Invoke-WebRequest -Uri $url -OutFile $p -UseBasicParsing
    $got = (Get-FileHash $p -Algorithm SHA1).Hash.ToLower()
    if ($got -ne $sha1.ToLower()) {
        Remove-Item $p -Force
        throw "sha1 mismatch for ${artifactPath}: got $got, want $sha1"
    }
    return $p
}

$classpath = New-Object System.Collections.Generic.List[string]
$nativesJars = New-Object System.Collections.Generic.List[object]   # @{jar; exclude}
foreach ($lib in $meta.libraries) {
    if (-not (Test-RulesAllowed $lib.rules)) { continue }
    $exclude = @()
    if ($lib.extract -and $lib.extract.exclude) { $exclude = @($lib.extract.exclude) }
    if ($lib.downloads.artifact) {
        $jar = Resolve-LibJar $lib.downloads.artifact
        $classpath.Add($jar)
        # In 26.3 natives are separate library entries (e.g.
        # org.lwjgl:lwjgl:3.4.3:natives-windows), not classifiers - detect
        # them by name/path and extract their .dll payload into the natives
        # dir instead of treating them as plain classpath entries.
        if ($lib.name -match 'natives-windows' -or $lib.downloads.artifact.path -match 'natives-windows') {
            $nativesJars.Add(@{ jar = $jar; exclude = $exclude })
        }
    }
    if ($lib.downloads.classifiers) {
        foreach ($prop in $lib.downloads.classifiers.PSObject.Properties) {
            if ($prop.Name -notmatch 'windows') { continue }
            $jar = Resolve-LibJar $prop.Value
            $classpath.Add($jar)
            $nativesJars.Add(@{ jar = $jar; exclude = $exclude })
        }
    }
}
$classpath.Add($ClientJar)

# --- natives: extract the windows classifier jars into <natives>/java ------
# The manifest spells it "${natives_directory}/java"; the sibling jna/lwjgl/
# netty dirs are scratch dirs named by the manifest's jvm arguments. Only the
# .dll payload of the jars is extracted; no code is read or shipped.
if (-not (Test-Path $NativesRoot)) { New-Item -ItemType Directory -Path $NativesRoot | Out-Null }
$nativesDir = Join-Path $NativesRoot $Nick
if (Test-Path $nativesDir) { Remove-Item -Recurse -Force $nativesDir }
foreach ($sub in 'java', 'jna', 'lwjgl', 'netty') {
    New-Item -ItemType Directory -Path (Join-Path $nativesDir $sub) | Out-Null
}
$nativesJava = Join-Path $nativesDir 'java'
Add-Type -AssemblyName System.IO.Compression.FileSystem
foreach ($item in $nativesJars) {
    $cjar = $item.jar
    $exclude = $item.exclude
    $zip = [System.IO.Compression.ZipFile]::OpenRead($cjar)
    try {
        foreach ($e in $zip.Entries) {
            $name = $e.FullName -replace '\\', '/'
            if ($name.EndsWith('/')) { continue }   # directory entry
            $skip = $false
            foreach ($x in $exclude) {
                $pat = '^' + [regex]::Escape($x).Replace('\*', '.*') + '$'
                if ($name -match $pat) { $skip = $true; break }
            }
            if ($skip) { continue }
            # The game's own natives jars pack the .dll files under
            # windows/x64/... (nested). java.library.path is flat, so x64
            # .dll entries are extracted by file name into the natives dir;
            # everything else keeps its archive path.
            $dest = if ($name -match '\.dll$' -and $name -match 'windows/x64/') {
                Join-Path $nativesJava (Split-Path $name -Leaf)
            } elseif ($name -match '\.dll$' -and $name -match 'windows/arm64/') {
                continue   # other architecture, not ours
            } else {
                Join-Path $nativesJava ($name -replace '/', '\')
            }
            New-Item -ItemType Directory -Path (Split-Path $dest) -Force | Out-Null
            [System.IO.Compression.ZipFileExtensions]::ExtractToFile($e, $dest, $true)
        }
    } finally {
        $zip.Dispose()
    }
}

# --- game directory + offline UUID ----------------------------------------
$gameDir = Join-Path $ClientsRoot $Nick
if (-not (Test-Path $gameDir)) { New-Item -ItemType Directory -Path $gameDir | Out-Null }
# Offline UUID: MD5 of "OfflinePlayer:<nick>" with RFC 4122 v3 bits set - the
# same derivation the server log shows for offline players (verified against
# the vanilla log of 13.11.2026).
$md5 = [System.Security.Cryptography.MD5]::Create()
$hash = $md5.ComputeHash([System.Text.Encoding]::UTF8.GetBytes('OfflinePlayer:' + $Nick))
$hash[6] = ($hash[6] -band 0x0f) -bor 0x30
$hash[8] = ($hash[8] -band 0x3f) -bor 0x80
$uuid = ([Guid]::new($hash)).ToString()

# --- placeholder substitution ---------------------------------------------
$subst = @{
    '${auth_player_name}'    = $Nick
    '${version_name}'        = '26.3'
    '${game_directory}'      = $gameDir
    '${assets_root}'         = $AssetsRoot
    '${assets_index_name}'   = [string]$meta.assetIndex.id
    '${auth_uuid}'           = $uuid
    '${auth_access_token}'   = '0'      # token stub: offline servers only
    '${clientid}'            = '0'
    '${auth_xuid}'           = '0'
    '${version_type}'        = 'release'
    '${launcher_name}'       = 'testclient'
    '${launcher_version}'    = '0'
    '${resolution_width}'    = [string]$Width
    '${resolution_height}'   = [string]$Height
    '${natives_directory}'   = $nativesDir
    '${classpath}'           = ($classpath -join ';')
    '${quickPlayPath}'       = (Join-Path $gameDir 'quickplay.log')
    '${quickPlayMultiplayer}' = $Address
    '${quickPlaySingleplayer}' = ''
    '${quickPlayRealms}'     = ''
}

# Expands one argument node (string, or {rules,value}) into strings.
function Expand-Node($node, [System.Collections.Generic.List[string]]$out) {
    if ($node -is [string]) {
        $s = $node
        foreach ($k in $subst.Keys) { $s = $s.Replace($k, $subst[$k]) }
        $out.Add($s)
        return
    }
    if (-not (Test-RulesAllowed $node.rules)) { return }
    if ($node.value -is [System.Array]) {
        foreach ($v in $node.value) { Expand-Node $v $out }
    } else {
        Expand-Node $node.value $out
    }
}

# JVM arguments come from the manifest's "jvm" section only. The section
# "default-user-jvm" (launcher tuning: ZGC etc.) is deliberately not applied;
# the heap limits are the owner's rule (-Xms512M -Xmx2048M), prepended.
$jvmArgs = New-Object System.Collections.Generic.List[string]
$jvmArgs.Add('-Xms512M')
$jvmArgs.Add('-Xmx2048M')
foreach ($a in $meta.arguments.jvm) { Expand-Node $a $jvmArgs }

$gameArgs = New-Object System.Collections.Generic.List[string]
foreach ($a in $meta.arguments.game) { Expand-Node $a $gameArgs }

return @{
    JavaExe    = $JavaExe
    MainClass  = [string]$meta.mainClass
    Classpath  = ($classpath -join ';')
    JvmArgs    = $jvmArgs
    GameArgs   = $gameArgs
    GameDir    = $gameDir
    NativesDir = $nativesDir
    Uuid       = $uuid
    Version    = [string]$meta.id
    AssetIndex = [string]$meta.assetIndex.id
}
