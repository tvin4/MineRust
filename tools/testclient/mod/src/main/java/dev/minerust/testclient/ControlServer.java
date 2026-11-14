package dev.minerust.testclient;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.google.gson.JsonSyntaxException;

/**
 * Newline-delimited JSON control socket, bound to 127.0.0.1 only.
 *
 * <p>One command per line, one response per line. Request:
 * {@code {"id": 1, "cmd": "ping", "args": {}}} (the {@code id} is echoed).
 * Response: {@code {"id": 1, "ok": true, "result": {...}}} or
 * {@code {"id": 1, "ok": false, "error": "..."}}. The socket thread only
 * reads, parses and writes: command execution happens on the client thread
 * through {@link CommandDispatcher}.
 */
final class ControlServer {
	private final int port;
	private final CommandDispatcher dispatcher;

	ControlServer(int port, CommandDispatcher dispatcher) {
		this.port = port;
		this.dispatcher = dispatcher;
	}

	/** Starts the accept loop on a daemon thread. */
	void start() {
		Thread thread = new Thread(this::acceptLoop, "testclient-control");
		thread.setDaemon(true);
		thread.start();
	}

	private void acceptLoop() {
		try (ServerSocket server = new ServerSocket()) {
			server.bind(new InetSocketAddress(InetAddress.getByName("127.0.0.1"), port));
			TestClientMod.LOGGER.info("control socket listening on 127.0.0.1:{}", port);
			while (true) {
				Socket socket = server.accept();
				Thread connection = new Thread(() -> serve(socket), "testclient-control-conn");
				connection.setDaemon(true);
				connection.start();
			}
		} catch (IOException error) {
			TestClientMod.LOGGER.error("control socket failed: {}", error.toString());
		}
	}

	private void serve(Socket socket) {
		try (Socket open = socket;
				BufferedReader in = new BufferedReader(
						new InputStreamReader(open.getInputStream(), StandardCharsets.UTF_8));
				Writer out = new OutputStreamWriter(open.getOutputStream(), StandardCharsets.UTF_8)) {
			String line;
			while ((line = in.readLine()) != null) {
				if (line.isBlank()) {
					continue;
				}
				out.write(handleLine(line));
				out.write('\n');
				out.flush();
			}
		} catch (IOException error) {
			TestClientMod.LOGGER.debug("control connection closed: {}", error.toString());
		}
	}

	private String handleLine(String line) {
		JsonObject response = new JsonObject();
		JsonObject request = null;
		try {
			JsonElement parsed = JsonParser.parseString(line);
			if (!parsed.isJsonObject()) {
				throw new JsonSyntaxException("command must be a JSON object");
			}
			request = parsed.getAsJsonObject();
			JsonElement result = dispatcher.run(request);
			if (request.has("id")) {
				response.add("id", request.get("id"));
			}
			response.addProperty("ok", true);
			response.add("result", result);
		} catch (Exception error) {
			if (request != null && request.has("id")) {
				response.add("id", request.get("id"));
			}
			response.addProperty("ok", false);
			response.addProperty("error", message(error));
		}
		return response.toString();
	}

	private static String message(Exception error) {
		Throwable cause = error.getCause() != null ? error.getCause() : error;
		return cause.toString();
	}
}
