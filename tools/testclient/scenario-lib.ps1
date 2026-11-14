# tools/testclient - helpers for run-scenario.ps1 (stage V orchestrator).
#
# Dot-source this file. Everything here is test-side only: control-socket
# calls, trace parsing of mcproxy JSONL, vanilla-log and RCON helpers.
# Packet names/ids in traces come from E:\MineRust-data\reports\packets.json
# (serverbound: player_input=43, player_action=41, punch=46;
#  clientbound: swing_animation=123, player_rotation=74, player_position=73).

$ErrorActionPreference = 'Stop'

# --- control socket --------------------------------------------------------

# Sends one JSON command line to a testclient control port and returns the
# parsed response object. One request per connection (the mod serves one
# request per connection).
function Invoke-TestCmd {
	param([int]$Port, [string]$Json, [int]$TimeoutMs = 32000)
	$client = New-Object System.Net.Sockets.TcpClient
	$client.Connect('127.0.0.1', $Port)
	try {
		$stream = $client.GetStream()
		$writer = New-Object System.IO.StreamWriter($stream)
		$writer.AutoFlush = $true
		$reader = New-Object System.IO.StreamReader($stream)
		$writer.WriteLine($Json)
		$stream.ReadTimeout = $TimeoutMs
		$line = $reader.ReadLine()
		if (-not $line) { throw "no answer from control port $Port" }
		return $line | ConvertFrom-Json
	} finally {
		$client.Close()
	}
}

# Maps a player UUID to the 26.3 playerdata file name: the first three
# UUID groups are byte-reversed in the file name (verified on Alice/Bob:
# 08059210-d8d5-ed3e-... -> 10920508-d5d8-3eed-...). The file lives in
# <world>/players/data/<name>.dat.
function Get-PlayerDataPath {
	param([string]$WorldDir, [string]$Uuid)
	$groups = $Uuid -split '-'
	$rev = @()
	foreach ($g in $groups[0..2]) {
		$pairs = @()
		for ($i = 0; $i -lt $g.Length; $i += 2) { $pairs += $g.Substring($i, 2) }
		[Array]::Reverse($pairs)
		$rev += , (-join $pairs)
	}
	$name = ($rev -join '-') + '-' + ($groups[3..4] -join '-')
	return Join-Path $WorldDir "players\data\$name.dat"
}

# --- trace reading ---------------------------------------------------------

# Loads a mcproxy JSONL trace (one packet per line) into objects with
# seq, t_ms, dir, state, id, name, b64. Lines are already in seq order.
# Parsing is done in one ConvertFrom-Json call (an array) - line-by-line
# parsing times out on 40k-line traces.
function Read-Trace {
	param([string]$Path)
	$text = Get-Content $Path -Raw
	$lines = $text -split "`n" | Where-Object { $_.Trim() }
	if ($lines.Count -eq 0) { return @() }
	return @(('[' + ($lines -join ',') + ']') | ConvertFrom-Json)
}

# Counts serverbound rows whose name matches the pattern (regex).
function Get-TraceCount {
	param($Rows, [string]$NamePattern)
	return @($Rows | Where-Object { $_.dir -eq 'serverbound' -and $_.name -match $NamePattern }).Count
}

# Returns the seq numbers of serverbound rows matching the pattern.
function Get-TraceSeqs {
	param($Rows, [string]$NamePattern)
	return @($Rows | Where-Object { $_.dir -eq 'serverbound' -and $_.name -match $NamePattern } | ForEach-Object { [int]$_.seq })
}

# Checks that every seq in $First precedes every seq in $Second (phase order).
function Test-SeqOrder {
	param([int[]]$First, [int[]]$Second)
	if ($First.Count -eq 0 -or $Second.Count -eq 0) { return $false }
	return (($First | Measure-Object -Maximum).Maximum -lt ($Second | Measure-Object -Minimum).Minimum)
}

# Decodes one base64 payload into a byte array.
function ConvertFrom-TraceB64 {
	param([string]$B64)
	return [Convert]::FromBase64String($B64)
}

# Finds a float32 pair (yaw, pitch) in clientbound rotation-bearing packets
# matching the expected rotation within tolerance. Floats sit at arbitrary
# byte offsets (varint id stripped), so the scan steps by 1 byte over pairs
# of consecutive floats, read big-endian (the Minecraft wire format).
# Rotation packets in 26.3 (reports/packets.json): player_position 73,
# player_rotation 74, entity_position_sync 35. The server quantizes angles
# to 1.40625 deg, so callers pass the expected angle and a tolerance of at
# least 0.75 deg. Returns $true on any hit.
function Find-RotationInTrace {
	param($Rows, [float]$Yaw, [float]$Pitch, [float]$ToleranceDeg = 0.75)
	foreach ($row in $Rows) {
		if ($row.dir -ne 'clientbound') { continue }
		if ($row.name -notmatch 'player_position|player_rotation|entity_position_sync') { continue }
		$bytes = ConvertFrom-TraceB64 $row.b64
		if ($bytes.Length -lt 8) { continue }
		for ($off = 0; $off + 8 -le $bytes.Length; $off++) {
			$yawBytes = [byte[]]@($bytes[$off + 3], $bytes[$off + 2], $bytes[$off + 1], $bytes[$off])
			$pitchBytes = [byte[]]@($bytes[$off + 7], $bytes[$off + 6], $bytes[$off + 5], $bytes[$off + 4])
			$a = [BitConverter]::ToSingle($yawBytes, 0)
			$b = [BitConverter]::ToSingle($pitchBytes, 0)
			if ([Math]::Abs($a - $Yaw) -le $ToleranceDeg -and [Math]::Abs($b - $Pitch) -le $ToleranceDeg) {
				return $true
			}
		}
	}
	return $false
}

# Reads the saved rotation (yaw, pitch) from a gzipped playerdata NBT file.
# Structure verified against the real 26.3 playerdata: TAG_List "Rotation"
# (type byte 9, name-length 2 bytes BE, name, element type 5, count 4 bytes
# BE, then two float32 big-endian values). Returns $null when the tag is
# not found.
function Get-PlayerRotationFromNbt {
	param([string]$Path)
	$fs = [IO.File]::OpenRead($Path)
	try {
		$gz = New-Object IO.Compression.GzipStream($fs, [IO.Compression.CompressionMode]::Decompress)
		$ms = New-Object IO.MemoryStream
		$gz.CopyTo($ms)
		$gz.Close()
	} finally {
		$fs.Close()
	}
	$data = $ms.ToArray()
	$ascii = [Text.Encoding]::ASCII.GetString($data)
	$i = $ascii.IndexOf('Rotation')
	if ($i -lt 0) { return $null }
	$yawBytes = [byte[]]@($data[$i + 16], $data[$i + 15], $data[$i + 14], $data[$i + 13])
	$pitchBytes = [byte[]]@($data[$i + 20], $data[$i + 19], $data[$i + 18], $data[$i + 17])
	return [PSCustomObject]@{
		yaw   = [BitConverter]::ToSingle($yawBytes, 0)
		pitch = [BitConverter]::ToSingle($pitchBytes, 0)
	}
}

# Trace files created after the given moment, oldest first. mcproxy names
# them proxy-conn-N.jsonl and creates the file when the connection starts,
# so creation time attributes a file to a scenario phase.
function Get-NewTraceFiles {
	param([string]$Dir, [datetime]$Since)
	return @(Get-ChildItem $Dir -Filter 'proxy-conn-*.jsonl' -ErrorAction SilentlyContinue |
		Where-Object { $_.CreationTime -gt $Since } | Sort-Object CreationTime)
}

# --- vanilla log -----------------------------------------------------------

# Waits for a FRESH "<nick> joined the game" line in the vanilla log:
# only lines after $Since are considered (old lines caused false hits twice).
function Wait-FreshJoin {
	param([string]$Log, [string]$Nick, [datetime]$Since, [int]$TimeoutSec = 90)
	$deadline = (Get-Date).AddSeconds($TimeoutSec)
	# Log lines carry only HH:mm:ss; stamp them with today's date so they
	# compare against $Since (a real DateTime) correctly, even past midnight.
	$today = Get-Date -Format 'yyyy-MM-dd'
	while ((Get-Date) -lt $deadline) {
		$hit = Get-Content $Log -ErrorAction SilentlyContinue | Where-Object {
			$stamp = [regex]::Match($_, '^\[(\d\d:\d\d:\d\d)\]')
			if (-not $stamp.Success) { return $false }
			$lineTime = [datetime]::ParseExact("$today $($stamp.Groups[1].Value)", 'yyyy-MM-dd HH:mm:ss', $null)
			if ($lineTime -lt $Since.AddSeconds(-1)) { return $false }
			$_ -match [regex]::Escape("System chat: $Nick joined the game")
		}
		if ($hit) { return $true }
		Start-Sleep -Milliseconds 200
	}
	return $false
}

# --- process control -------------------------------------------------------

# Stops a test client by the gameDir marker in its command line.
function Stop-ClientByName {
	param([string]$Nick)
	Get-CimInstance Win32_Process -Filter "Name='java.exe'" |
		Where-Object { $_.CommandLine -match "clients\\$Nick" } |
		ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
}

# --- command JSON building -------------------------------------------------
# JSON fragments are concatenated here (never -f format strings): plain
# braces in a format string are format errors in PowerShell.

function New-LookCmd {
	param([string]$Id, $Dx, $Dy)
	return '{"id":"' + $Id + '","cmd":"look","args":{"dx":' + $Dx + ',"dy":' + $Dy + '}}'
}

function New-ObserveCmd {
	param([string]$Id, [string]$Player = '')
	if ($Player) {
		return '{"id":"' + $Id + '","cmd":"observe","args":{"player":"' + $Player + '"}}'
	}
	return '{"id":"' + $Id + '","cmd":"observe"}'
}

function New-KeyCmd {
	param([string]$Id, [string]$Key, [string]$Action = 'tap')
	return '{"id":"' + $Id + '","cmd":"key","args":{"key":"' + $Key + '","action":"' + $Action + '"}}'
}

function New-SessionCmd {
	param([string]$Id, [string]$Cmd)
	return '{"id":"' + $Id + '","cmd":"' + $Cmd + '"}'
}

# --- result recording ------------------------------------------------------

# Records one check into the scenario result object.
function Add-Check {
	param($Scenario, [string]$Name, [bool]$Pass, [string]$Fact)
	$Scenario.checks.Add([PSCustomObject]@{ name = $Name; pass = $Pass; fact = $Fact }) | Out-Null
	if (-not $Pass) { $Scenario.failed = $true }
	Write-Host ("  [{0}] {1} - {2}" -f $(if ($Pass) { 'PASS' } else { 'FAIL' }), $Name, $Fact)
}
