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

# The vanilla playerdata file name of a nick (Шаг 19a). The UUID is the
# offline one (MD5("OfflinePlayer:<name>") with the version-3 and RFC 4122
# bits); the file name is those 16 bytes in order with dashes after the 4th,
# 6th and 8th byte — the same rule `playerdata::file_name` uses, and the
# reason it is *not* the string the launcher prints (that one is a .NET GUID,
# whose first three groups are reversed). Scenarios that must patch the file
# *before* the client joins need this without a launch.
function Get-OfflinePlayerFileName {
	param([string]$Name)
	$md5 = [System.Security.Cryptography.MD5]::Create()
	try {
		$bytes = $md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("OfflinePlayer:$Name"))
	} finally {
		$md5.Dispose()
	}
	$bytes[6] = [byte](($bytes[6] -band 0x0F) -bor 0x30)
	$bytes[8] = [byte](($bytes[8] -band 0x3F) -bor 0x80)
	$hex = -join ($bytes | ForEach-Object { $_.ToString('x2') })
	return '{0}-{1}-{2}-{3}-{4}.dat' -f $hex.Substring(0, 8), $hex.Substring(8, 4), $hex.Substring(12, 4), $hex.Substring(16, 4), $hex.Substring(20, 12)
}

# The offline UUID itself (not the reversed playerdata file name): ops.json
# keys players by this UUID, so a scenario that grants a test player operator
# rights has to write the same value the server computes at login.
function Get-OfflineUuid {
	param([string]$Name)
	$md5 = [System.Security.Cryptography.MD5]::Create()
	try {
		$bytes = $md5.ComputeHash([Text.Encoding]::UTF8.GetBytes("OfflinePlayer:$Name"))
	} finally {
		$md5.Dispose()
	}
	$bytes[6] = [byte](($bytes[6] -band 0x0F) -bor 0x30)
	$bytes[8] = [byte](($bytes[8] -band 0x3F) -bor 0x80)
	$hex = -join ($bytes | ForEach-Object { $_.ToString('x2') })
	return '{0}-{1}-{2}-{3}-{4}' -f $hex.Substring(0, 8), $hex.Substring(8, 4), $hex.Substring(12, 4), $hex.Substring(16, 4), $hex.Substring(20, 12)
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
	$lines = @($text -split "`n" | Where-Object { $_.Trim() })
	# The proxy appends while the scenario runs (S13 reads the trace of a
	# client that is still connected), so the last line may be cut mid-JSON.
	# Drop it: one packet less is fine, a truncated array throws and its
	# message embeds the whole base64 payload (S13 r3: 40 MB of text).
	while ($lines.Count -gt 0 -and -not $lines[-1].TrimEnd().EndsWith('}')) {
		$lines = @($lines | Select-Object -First ($lines.Count - 1))
	}
	if ($lines.Count -eq 0) { return @() }
	# A cut can also land on a nested '}' of a half-written line, so the
	# trailing-line drop is retried a few times before failing.
	for ($attempt = 0; $attempt -lt 4; $attempt++) {
		try {
			return @(('[' + ($lines -join ',') + ']') | ConvertFrom-Json)
		} catch {
			$message = $_.Exception.Message
			if ($attempt -eq 3 -or $lines.Count -eq 0) {
				# A live append can also leave a line that ends with '}' but is
				# still torn (S15 V2 r1 read it at 38k lines). One bad record
				# must not abort a scenario, so keep the lines that parse.
				# This is the rare path: whole-file parsing is the fast one.
				$valid = @()
				foreach ($line in $lines) {
					try { $valid += ($line | ConvertFrom-Json) } catch { }
				}
				if ($valid.Count -gt 0) { return $valid }
				throw "trace '$Path' is not valid JSONL ($($lines.Count) lines): $($message.Substring(0, [Math]::Min(200, $message.Length)))"
			}
			$lines = @($lines | Select-Object -First ($lines.Count - 1))
		}
	}
	return @()
}

# Streams a large proxy trace line by line and returns only the rows whose
# packet name is in `$Names` (exact match). Unlike Read-Trace it never builds a
# whole-file JSON array, so 100+ MB traces (two clients streaming entity
# packets) cannot exhaust memory or stall post-processing. A torn trailing line
# is skipped.
function Read-TraceMatches {
	param([string]$Path, [string[]]$Names)
	$lookup = @{}
	foreach ($n in $Names) { $lookup[[string]$n] = $true }
	$out = New-Object System.Collections.ArrayList
	foreach ($line in [IO.File]::ReadLines($Path)) {
		$want = $false
		foreach ($n in $Names) { if ($line.Contains(('"' + $n + '"'))) { $want = $true; break } }
		if (-not $want) { continue }
		try { $row = $line | ConvertFrom-Json } catch { continue }
		if ($lookup.ContainsKey([string]$row.name)) { $out.Add($row) | Out-Null }
	}
	return $out
}

# Cheap live check: does the trace already contain a disconnect row? Used
# while a client is still connected; a full JSON parse of a growing 40 MB
# trace every second is both slow and racy.
function Test-TraceHasDisconnect {
	param([string]$Path)
	if (-not (Test-Path -LiteralPath $Path)) { return $false }
	$hit = Select-String -Path $Path -Pattern '"name":"minecraft:(login_)?disconnect"' -Quiet -ErrorAction SilentlyContinue
	return [bool]$hit
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

# Decodes a packed block position (8 bytes, big-endian): x = v >> 38 (26-bit
# signed), z = (v >> 12) & 0x3ffffff (26-bit signed), y = v & 0xfff (12-bit
# signed). SOURCE: the packet field layout in crates/protocol.
function ConvertFrom-PackedPosition {
	param([byte[]]$Bytes, [int]$Offset)
	if ($Offset + 8 -gt $Bytes.Length) { throw 'truncated packed position in trace payload' }
	[UInt64]$value = 0
	for ($i = 0; $i -lt 8; $i++) { $value = ($value -shl 8) -bor [UInt64]$Bytes[$Offset + $i] }
	[long]$x = [long](($value -shr 38) -band 0x3ffffff)
	if ($x -ge 0x2000000) { $x -= 0x4000000 }
	[long]$z = [long](($value -shr 12) -band 0x3ffffff)
	if ($z -ge 0x2000000) { $z -= 0x4000000 }
	[long]$y = [long]($value -band 0xfff)
	if ($y -ge 0x800) { $y -= 0x1000 }
	return [PSCustomObject]@{ x = [int]$x; y = [int]$y; z = [int]$z }
}

# Decodes one VarInt from a packet payload.
function ConvertFrom-TraceVarInt {
	param([byte[]]$Bytes, [int]$Offset)
	$value = 0
	for ($part = 0; $part -lt 5; $part++) {
		$index = $Offset + $part
		if ($index -ge $Bytes.Length) { throw 'truncated VarInt in trace payload' }
		$byte = [int]$Bytes[$index]
		$value = $value -bor (($byte -band 0x7f) -shl (7 * $part))
		if (($byte -band 0x80) -eq 0) { return [PSCustomObject]@{ value = $value; next = $index + 1 } }
	}
	throw 'VarInt exceeds five bytes in trace payload'
}

# .NET Guid stores the first three UUID fields little-endian internally;
# ToByteArray returns the network byte sequence expected in Minecraft packets.
function ConvertTo-NetworkUuidBytes {
	param([Guid]$Uuid)
	return ,$Uuid.ToByteArray()
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
		Where-Object {
			$_.CreationTime -gt $Since -and
			(Select-String -LiteralPath $_.FullName -Pattern '"name":"minecraft:login_finished"' -Quiet)
		} | Sort-Object CreationTime)
}

# First file from the list whose text matches the regex. Used to attribute
# trace files to roles: the digger's file carries serverbound player_action,
# the observer's carries the clientbound destruction/effects. A client also
# receives the clientbound packets, so the caller must exclude the file it
# already took for the other role.
function Select-TraceByPattern {
	param([object[]]$Files, [string]$Pattern, [string[]]$Exclude = @())
	foreach ($file in @($Files)) {
		if ($Exclude -contains $file.FullName) { continue }
		if (Select-String -LiteralPath $file.FullName -Pattern $Pattern -Quiet) { return $file }
	}
	return $null
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

# Stops a test client by the gameDir marker in its command line and waits for
# process exit so the next launch can safely replace locked native DLLs.
function Stop-ClientByName {
	param([string]$Nick, [int]$TimeoutSec = 20)
	$processes = @(Get-CimInstance Win32_Process -Filter "Name='java.exe'" |
		Where-Object { $_.CommandLine -match "clients\\$Nick" })
	foreach ($process in $processes) {
		Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
		$deadline = (Get-Date).AddSeconds($TimeoutSec)
		while ((Get-Date) -lt $deadline -and (Get-Process -Id $process.ProcessId -ErrorAction SilentlyContinue)) {
			Start-Sleep -Milliseconds 200
		}
		if (Get-Process -Id $process.ProcessId -ErrorAction SilentlyContinue) {
			throw "client '$Nick' process $($process.ProcessId) did not stop within ${TimeoutSec}s"
		}
	}
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

function New-BlockCmd {
	param([string]$Id, [int]$X, [int]$Y, [int]$Z)
	return '{"id":"' + $Id + '","cmd":"block","args":{"x":' + $X + ',"y":' + $Y + ',"z":' + $Z + '}}'
}

function New-DigPacketCmd {
	param([string]$Id, [int]$X, [int]$Y, [int]$Z, [int]$Ticks = 1)
	return '{"id":"' + $Id + '","cmd":"digpacket","args":{"x":' + $X + ',"y":' + $Y + ',"z":' + $Z + ',"k":' + $Ticks + '}}'
}

function New-KeyCmd {
	param([string]$Id, [string]$Key, [string]$Action = 'tap', [int]$HoldMs = 0)
	$json = '{"id":"' + $Id + '","cmd":"key","args":{"key":"' + $Key + '","action":"' + $Action + '"'
	# hold_ms only matters for a tap and is omitted otherwise, so existing
	# callers keep the exact request they always sent.
	if ($HoldMs -gt 0) { $json += ',"hold_ms":' + $HoldMs }
	return $json + '}}'
}

function New-SessionCmd {
	param([string]$Id, [string]$Cmd)
	return '{"id":"' + $Id + '","cmd":"' + $Cmd + '"}'
}

# Шаг 25: a plain chat line through the mod's `chat` command
# (ClientPacketListener.sendChat, the chat-screen path for a line without a
# leading slash). The text is JSON-escaped because it comes from a scenario.
function New-ChatCmd {
	param([string]$Id, [string]$Text)
	$escaped = $Text.Replace('\', '\\').Replace('"', '\"')
	return '{"id":"' + $Id + '","cmd":"chat","args":{"text":"' + $escaped + '"}}'
}

# Шаг 25: one chat command through the mod's `command` command
# (ClientPacketListener.sendCommand). A leading slash is optional.
function New-CommandCmd {
	param([string]$Id, [string]$Text)
	$escaped = $Text.Replace('\', '\\').Replace('"', '\"')
	return '{"id":"' + $Id + '","cmd":"command","args":{"text":"' + $escaped + '"}}'
}

# Decoded payload as UTF-8: commands, chat bodies, NBT component strings and
# translation keys are plain ASCII, so a substring check needs no field parser
# (the exact field layout comes from the byte tables in the trace).
function Get-PayloadUtf8 {
	param([string]$B64)
	return [Text.Encoding]::UTF8.GetString((ConvertFrom-TraceB64 $B64))
}

# Шаг 24.0a: the creative-menu item-add path. `slot` is the raw container slot
# (the player inventory menu numbers the hotbar 36..44; omitted -> selected).
function New-CreativeCmd {
	param([string]$Id, [string]$Item, [int]$Slot = -1, [int]$Count = 0)
	$json = '{"id":"' + $Id + '","cmd":"creative","args":{"item":"' + $Item + '"'
	if ($Slot -ge 0) { $json += ',"slot":' + $Slot }
	if ($Count -gt 0) { $json += ',"count":' + $Count }
	return $json + '}}'
}

# Шаг 24.0e: one mouse-wheel tick through MouseHandler.onScroll.
function New-ScrollCmd {
	param([string]$Id, [double]$Dy)
	$inv = [System.Globalization.CultureInfo]::InvariantCulture
	return '{"id":"' + $Id + '","cmd":"scroll","args":{"dy":' + ([string]::Format($inv, '{0}', $Dy)) + '}}'
}

# Client-side teleport for our own server (no RCON, no /tp): the mod sets the
# local player position and the client's own movement path reports it.
# Rotation is optional; the invariant-culture strings keep the decimals plain.
function New-TpCmd {
	param([string]$Id, $X, $Y, $Z, $Yaw = $null, $Pitch = $null)
	$inv = [System.Globalization.CultureInfo]::InvariantCulture
	$json = '{"id":"' + $Id + '","cmd":"tp","args":{"x":' + ([string]::Format($inv, '{0}', $X)) +
		',"y":' + ([string]::Format($inv, '{0}', $Y)) +
		',"z":' + ([string]::Format($inv, '{0}', $Z))
	if ($null -ne $Yaw) { $json += ',"yaw":' + ([string]::Format($inv, '{0}', $Yaw)) }
	if ($null -ne $Pitch) { $json += ',"pitch":' + ([string]::Format($inv, '{0}', $Pitch)) }
	return $json + '}}'
}

# --- result recording ------------------------------------------------------

# Records one check into the scenario result object.
function Add-Check {
	param($Scenario, [string]$Name, [bool]$Pass, [string]$Fact)
	$Scenario.checks.Add([PSCustomObject]@{ name = $Name; pass = $Pass; fact = $Fact }) | Out-Null
	if (-not $Pass) { $Scenario.failed = $true }
	Write-Host ("  [{0}] {1} - {2}" -f $(if ($Pass) { 'PASS' } else { 'FAIL' }), $Name, $Fact)
}

# --- block-position trace decoding -----------------------------------------

# Reads one mcproxy JSONL trace and decodes the block position of the packet
# kinds named in `Names` (names and ids from reports/packets.json). Every row
# is returned as an object carrying the packet's own fields; a kind whose
# layout is not decoded here is returned with only `ms/dir/name` so a count is
# still available. Field layouts (Шаг 13b/Шаг 18): block_update = pos + state
# varint; block_changed_ack = sequence varint; block_destruction = entityId
# varint + pos + stage byte; level_event = event int32 + pos + data int32.
function Get-TraceBlockRows {
	param([string]$Path, [string[]]$Names)
	$wanted = @{}
	foreach ($name in $Names) { $wanted[[string]$name] = $true }
	$rows = New-Object System.Collections.ArrayList
	$lines = @(Get-Content -LiteralPath $Path)
	foreach ($line in $lines) {
		if ([string]::IsNullOrWhiteSpace($line)) { continue }
		$row = $null
		try { $row = $line | ConvertFrom-Json } catch { continue }
		if ($null -eq $row -or -not $wanted.ContainsKey([string]$row.name)) { continue }
		$p = ConvertFrom-TraceB64 ([string]$row.b64)
		$out = [ordered]@{
			ms   = [double]$row.read_ms
			dir  = [string]$row.dir
			name = [string]$row.name
		}
		switch ([string]$row.name) {
			'minecraft:block_update' {
				$pos = ConvertFrom-PackedPosition $p 0
				$state = ConvertFrom-TraceVarInt $p 8
				$out.x = $pos.x; $out.y = $pos.y; $out.z = $pos.z; $out.state = [int]$state.value
			}
			'minecraft:block_changed_ack' {
				$seq = ConvertFrom-TraceVarInt $p 0
				$out.seq = [int]$seq.value
			}
			'minecraft:block_destruction' {
				$eid = ConvertFrom-TraceVarInt $p 0
				$pos = ConvertFrom-PackedPosition $p $eid.next
				$out.eid = [int]$eid.value
				$out.x = $pos.x; $out.y = $pos.y; $out.z = $pos.z
				$out.stage = [int]$p[$eid.next + 8]
			}
			'minecraft:player_action' {
				$action = ConvertFrom-TraceVarInt $p 0
				$pos = ConvertFrom-PackedPosition $p $action.next
				$out.action = [int]$action.value
				$out.x = $pos.x; $out.y = $pos.y; $out.z = $pos.z
				$out.face = [int]$p[$action.next + 8]
				$seq = ConvertFrom-TraceVarInt $p ($action.next + 9)
				$out.seq = [int]$seq.value
			}
			'minecraft:level_event' {
				# [int] casts matter: PowerShell shifts a [byte] in byte width.
				$event = ([int]$p[0] -shl 24) -bor ([int]$p[1] -shl 16) -bor ([int]$p[2] -shl 8) -bor [int]$p[3]
				$pos = ConvertFrom-PackedPosition $p 4
				$data = ([int]$p[12] -shl 24) -bor ([int]$p[13] -shl 16) -bor ([int]$p[14] -shl 8) -bor [int]$p[15]
				$out.event = $event
				$out.x = $pos.x; $out.y = $pos.y; $out.z = $pos.z; $out.data = $data
			}
			'default' { }
		}
		$rows.Add([PSCustomObject]$out) | Out-Null
	}
	return $rows
}

# --- playerdata NBT (test side) --------------------------------------------
# The player file is a gzipped named compound (the vanilla form). These
# helpers read it structurally so the scenario can show the fields the step
# promises (Pos, Rotation, Health, foodLevel, DataVersion, playerGameType)
# without adding a binary dependency to the test runner.

# Big-endian fixed-width value copied into a little-endian .NET buffer.
function ConvertFrom-BigEndian {
	param([byte[]]$Data, [int]$Offset, [int]$Length)
	$tmp = New-Object byte[] $Length
	for ($i = 0; $i -lt $Length; $i++) { $tmp[$i] = $Data[$Offset + $Length - 1 - $i] }
	return , $tmp
}

# Reads one tag payload by NBT type; unknown types are a hard error (the file
# must be fully understood or the check is not a check).
function Read-NbtPayload {
	param([byte[]]$Data, [int]$Type, [ref]$Pos, [int]$Depth = 0)
	if ($Depth -gt 64) { throw 'NBT nesting deeper than 64' }
	switch ($Type) {
		1 { $v = [sbyte]$Data[$Pos.Value]; $Pos.Value += 1; return $v }
		2 { $v = [BitConverter]::ToInt16((ConvertFrom-BigEndian $Data $Pos.Value 2), 0); $Pos.Value += 2; return $v }
		3 { $v = [BitConverter]::ToInt32((ConvertFrom-BigEndian $Data $Pos.Value 4), 0); $Pos.Value += 4; return $v }
		4 { $v = [BitConverter]::ToInt64((ConvertFrom-BigEndian $Data $Pos.Value 8), 0); $Pos.Value += 8; return $v }
		5 { $v = [BitConverter]::ToSingle((ConvertFrom-BigEndian $Data $Pos.Value 4), 0); $Pos.Value += 4; return $v }
		6 { $v = [BitConverter]::ToDouble((ConvertFrom-BigEndian $Data $Pos.Value 8), 0); $Pos.Value += 8; return $v }
		7 {
			$len = [BitConverter]::ToInt32((ConvertFrom-BigEndian $Data $Pos.Value 4), 0); $Pos.Value += 4
			$v = New-Object byte[] $len
			[Array]::Copy($Data, $Pos.Value, $v, 0, $len); $Pos.Value += $len
			return , $v
		}
		8 {
			$len = [BitConverter]::ToUInt16((ConvertFrom-BigEndian $Data $Pos.Value 2), 0); $Pos.Value += 2
			$v = [Text.Encoding]::UTF8.GetString($Data, $Pos.Value, $len); $Pos.Value += $len
			return $v
		}
		9 {
			$elemType = [int]$Data[$Pos.Value]; $Pos.Value += 1
			$count = [BitConverter]::ToInt32((ConvertFrom-BigEndian $Data $Pos.Value 4), 0); $Pos.Value += 4
			$list = New-Object System.Collections.Generic.List[object]
			for ($i = 0; $i -lt $count; $i++) {
				if ($elemType -eq 10) { $list.Add((Read-NbtCompound $Data $Pos)) | Out-Null }
				else { $list.Add((Read-NbtPayload $Data $elemType $Pos ($Depth + 1))) | Out-Null }
			}
			return , $list.ToArray()
		}
		10 { return , (Read-NbtCompound $Data $Pos $Depth) }
		11 {
			$count = [BitConverter]::ToInt32((ConvertFrom-BigEndian $Data $Pos.Value 4), 0); $Pos.Value += 4
			$v = New-Object int[] $count
			for ($i = 0; $i -lt $count; $i++) { $v[$i] = [BitConverter]::ToInt32((ConvertFrom-BigEndian $Data $Pos.Value 4), 0); $Pos.Value += 4 }
			return , $v
		}
		12 {
			$count = [BitConverter]::ToInt32((ConvertFrom-BigEndian $Data $Pos.Value 4), 0); $Pos.Value += 4
			$v = New-Object long[] $count
			for ($i = 0; $i -lt $count; $i++) { $v[$i] = [BitConverter]::ToInt64((ConvertFrom-BigEndian $Data $Pos.Value 8), 0); $Pos.Value += 8 }
			return , $v
		}
		default { throw "unsupported NBT tag type $Type at byte $($Pos.Value)" }
	}
}

# Reads a compound body (the root is a named compound too). Returns a
# hashtable of name -> value; $Offsets receives the absolute payload offset of
# every scalar tag so a check can patch one value in place.
function Read-NbtCompound {
	param([byte[]]$Data, [ref]$Pos, [int]$Depth = 0, [hashtable]$Offsets = $null)
	if ($Depth -gt 64) { throw 'NBT nesting deeper than 64' }
	$result = @{}
	while ($true) {
		$type = [int]$Data[$Pos.Value]; $Pos.Value += 1
		if ($type -eq 0) { break }
		$nameLen = [BitConverter]::ToUInt16((ConvertFrom-BigEndian $Data $Pos.Value 2), 0); $Pos.Value += 2
		$name = [Text.Encoding]::UTF8.GetString($Data, $Pos.Value, $nameLen); $Pos.Value += $nameLen
		$valuePos = $Pos.Value
		$value = Read-NbtPayload -Data $Data -Type $type -Pos $Pos -Depth ($Depth + 1)
		$result[$name] = $value
		if ($null -ne $Offsets -and $type -ne 9 -and $type -ne 10) { $Offsets[$name] = $valuePos }
	}
	return $result
}

# Decompresses one gzipped NBT document into its bytes.
function Read-GzipBytes {
	param([string]$Path)
	$fs = [IO.File]::OpenRead($Path)
	try {
		$gz = New-Object IO.Compression.GzipStream($fs, [IO.Compression.CompressionMode]::Decompress)
		$ms = New-Object IO.MemoryStream
		$gz.CopyTo($ms)
		$bytes = $ms.ToArray()
		$gz.Close()
		return , $bytes
	} finally { $fs.Close() }
}

# Writes bytes as one gzip member (replacing the file).
function Write-GzipBytes {
	param([byte[]]$Bytes, [string]$Path)
	$fs = [IO.File]::Create($Path)
	try {
		$gz = New-Object IO.Compression.GzipStream($fs, [IO.Compression.CompressionMode]::Compress)
		$gz.Write($Bytes, 0, $Bytes.Length)
		$gz.Close()
	} finally { $fs.Close() }
}

# Parses one playerdata file into { root, offsets }.
function Read-PlayerNbt {
	param([string]$Path)
	$data = Read-GzipBytes $Path
	$pos = 0
	$type = [int]$data[$pos]; $pos += 1
	if ($type -ne 10) { throw "playerdata root is not a compound (type $type)" }
	$nameLen = [BitConverter]::ToUInt16((ConvertFrom-BigEndian $data $pos 2), 0); $pos += 2
	$name = [Text.Encoding]::UTF8.GetString($data, $pos, $nameLen); $pos += $nameLen
	$offsets = @{}
	$root = Read-NbtCompound -Data $data -Pos ([ref]$pos) -Offsets $offsets
	return [PSCustomObject]@{ root = $root; offsets = $offsets; bytes = $data }
}

# The fields the S13 check prints. Missing tags are reported, never invented.
function Get-PlayerNbtFacts {
	param([string]$Path)
	$doc = Read-PlayerNbt $Path
	$r = $doc.root
	$pos = $null
	if ($r.ContainsKey('Pos') -and @($r['Pos']).Count -ge 3) { $pos = @($r['Pos'])[0..2] }
	$rot = $null
	if ($r.ContainsKey('Rotation') -and @($r['Rotation']).Count -ge 2) { $rot = @($r['Rotation'])[0..1] }
	return [PSCustomObject]@{
		pos            = $pos
		rotation       = $rot
		health         = $r['Health']
		foodLevel      = $r['foodLevel']
		saturation     = $r['foodSaturationLevel']
		gameType       = $r['playerGameType']
		dimension      = $r['Dimension']
		dataVersion    = $r['DataVersion']
		onGround       = $r['OnGround']
		doc            = $doc
	}
}

# Sets one int tag in place and rewrites the gzip member. Used to prove the
# configured game mode wins over the file when force_gamemode is on.
function Set-PlayerNbtInt {
	param([string]$Path, [string]$Tag, [int]$Value)
	$doc = Read-PlayerNbt $Path
	if (-not $doc.offsets.ContainsKey($Tag)) { throw "tag '$Tag' not found in $Path" }
	$offset = [int]$doc.offsets[$Tag]
	$bytes = $doc.bytes
	$big = [BitConverter]::GetBytes($Value)
	[Array]::Reverse($big)
	[Array]::Copy($big, 0, $bytes, $offset, 4)
	Write-GzipBytes $bytes $Path
}

# Minimal structural fingerprint of a directory tree (relative path, length,
# SHA-256) so two moments can be compared without keeping the tree in memory.
function Get-TreeFingerprint {
	param([string]$Root)
	if (-not (Test-Path -LiteralPath $Root)) { return @() }
	$rootFull = (Resolve-Path -LiteralPath $Root).Path
	return @(Get-ChildItem -LiteralPath $rootFull -File -Recurse -Force | ForEach-Object {
		$rel = $_.FullName.Substring($rootFull.Length).TrimStart('\')
		'{0}|{1}|{2}' -f $rel, $_.Length, (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
	} | Sort-Object)
}

# --- playerdata trace reads ------------------------------------------------

# Decodes a clientbound player_position payload: VarInt teleport id, three
# f64, three f64 velocity, two f32 angles, i32 flags.
function Get-TracePlayerPosition {
	param($Rows)
	foreach ($row in $Rows) {
		if ($row.dir -ne 'clientbound' -or $row.name -ne 'minecraft:player_position') { continue }
		$p = ConvertFrom-TraceB64 $row.b64
		$id = ConvertFrom-TraceVarInt $p 0
		if ($p.Length -lt ($id.next + 59)) { continue }
		$off = $id.next
		$x = [BitConverter]::ToDouble((ConvertFrom-BigEndian $p $off 8), 0)
		$y = [BitConverter]::ToDouble((ConvertFrom-BigEndian $p ($off + 8) 8), 0)
		$z = [BitConverter]::ToDouble((ConvertFrom-BigEndian $p ($off + 16) 8), 0)
		$yaw = [BitConverter]::ToSingle((ConvertFrom-BigEndian $p ($off + 48) 4), 0)
		$pitch = [BitConverter]::ToSingle((ConvertFrom-BigEndian $p ($off + 52) 4), 0)
		return [PSCustomObject]@{ seq = [int]$row.seq; t = [double]$row.read_ms; x = $x; y = $y; z = $z; yaw = $yaw; pitch = $pitch }
	}
	return $null
}

# Game mode announced by Login (play): read from the test client's observe
# (the packet's offset depends on the dimension name strings, so no byte
# offset is assumed here).

# Position of A's add_entity in B's trace: VarInt entity id, 16-byte UUID,
# VarInt type, then three f64.
function Get-TraceAddEntityPos {
	param($Rows, [byte[]]$UuidBytes)
	foreach ($row in $Rows) {
		if ($row.dir -ne 'clientbound' -or $row.name -ne 'minecraft:add_entity') { continue }
		$p = ConvertFrom-TraceB64 $row.b64
		$id = ConvertFrom-TraceVarInt $p 0
		if ($p.Length -lt ($id.next + 16 + 1 + 24)) { continue }
		$match = $true
		for ($i = 0; $i -lt 16; $i++) { if ($p[$id.next + $i] -ne $UuidBytes[$i]) { $match = $false; break } }
		if (-not $match) { continue }
		$type = ConvertFrom-TraceVarInt $p ($id.next + 16)
		$off = $type.next
		return [PSCustomObject]@{
			seq = [int]$row.seq
			t = [double]$row.read_ms
			x = [BitConverter]::ToDouble((ConvertFrom-BigEndian $p $off 8), 0)
			y = [BitConverter]::ToDouble((ConvertFrom-BigEndian $p ($off + 8) 8), 0)
			z = [BitConverter]::ToDouble((ConvertFrom-BigEndian $p ($off + 16) 8), 0)
		}
	}
	return $null
}
