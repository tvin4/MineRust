# tools/testclient — RCON client for the vanilla test server console.
#
# The scenario orchestrator needs server-console actions (save-all flush,
# /tp, /gamemode). The server reads its console from a file redirect only
# once, so RCON (enabled in server.properties, 127.0.0.1 only) is the
# reliable path. Protocol: Source RCON (little-endian length, request id,
# type 3 = auth, 2 = command, payload, two zero bytes) - public spec.
#
# Usage:
#   & tools\testclient\Invoke-Rcon.ps1 -Command 'list'
#   & tools\testclient\Invoke-Rcon.ps1 -Command 'save-all flush' [-Port 25575] [-Password 'test']
param(
	[Parameter(Mandatory = $true)][string]$Command,
	[string]$ServerHost = '127.0.0.1',
	[int]$Port = 25575,
	[string]$Password = 'test'
)

$ErrorActionPreference = 'Stop'

function Send-Packet($stream, [int]$id, [int]$type, [string]$payload) {
	$body = [byte[]](@([BitConverter]::GetBytes($id)) + [BitConverter]::GetBytes($type) +
		[Text.Encoding]::UTF8.GetBytes($payload) + [byte]0 + [byte]0)
	$packet = [byte[]]([BitConverter]::GetBytes($body.Length) + $body)
	$stream.Write($packet, 0, $packet.Length)
	$stream.Flush()
}

function Read-Packet($stream) {
	$lenBytes = New-Object byte[] 4
	$read = $stream.Read($lenBytes, 0, 4)
	if ($read -lt 4) { throw 'connection closed' }
	$len = [BitConverter]::ToInt32($lenBytes, 0)
	$buf = New-Object byte[] $len
	$offset = 0
	while ($offset -lt $len) {
		$read = $stream.Read($buf, $offset, $len - $offset)
		if ($read -le 0) { throw 'connection closed' }
		$offset += $read
	}
	return [PSCustomObject]@{
		id      = [BitConverter]::ToInt32($buf, 0)
		type    = [BitConverter]::ToInt32($buf, 4)
		payload = [Text.Encoding]::UTF8.GetString($buf, 8, [Math]::Max(0, $buf.Length - 10))
	}
}

$client = New-Object System.Net.Sockets.TcpClient
$client.Connect($ServerHost, $Port)
try {
	$stream = $client.GetStream()
	$auth = Send-Packet $stream 1 3 $Password
	$authReply = Read-Packet $stream
	if ($authReply.id -eq -1) { throw 'rcon auth failed' }
	Send-Packet $stream 2 2 $Command
	# Command responses are type 0 (SERVERDATA_RESPONSE_VALUE). The server
	# may fragment long answers; read until an empty payload or a short
	# idle gap.
	$result = ''
	while ($true) {
		$reply = Read-Packet $stream
		if ($reply.payload -eq '') { break }
		$result += $reply.payload
		if ($stream.DataAvailable -eq $false) {
			Start-Sleep -Milliseconds 60
			if ($stream.DataAvailable -eq $false) { break }
		}
	}
	return $result
} finally {
	$client.Close()
}
