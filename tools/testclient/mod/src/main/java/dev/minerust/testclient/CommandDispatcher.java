package dev.minerust.testclient;

import java.util.Map;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

/**
 * Routes JSON control commands onto the Minecraft client thread.
 *
 * <p>The control socket thread must never touch game state. It builds a
 * {@link Task}, enqueues it here and blocks on its future; the client tick
 * pump ({@link #tick()}) executes the handler on the game thread and
 * completes the future. One request is in flight per control connection,
 * so commands execute strictly in order.
 */
final class CommandDispatcher {
	/** How long the socket thread waits for the client thread before giving up. */
	static final long HANDLER_TIMEOUT_MS = 5000;

	/** A command handler. Runs on the client thread only. */
	@FunctionalInterface
	interface Handler {
		JsonElement handle(JsonObject args) throws Exception;
	}

	/** One queued command plus its future result. */
	static final class Task {
		/** Parsed request object. */
		final JsonObject request;
		/** Completed by the client thread with the handler result. */
		final CompletableFuture<JsonElement> result = new CompletableFuture<>();

		Task(JsonObject request) {
			this.request = request;
		}
	}

	private final BlockingQueue<Task> tasks = new LinkedBlockingQueue<>();
	private final Map<String, Handler> handlers = new ConcurrentHashMap<>();

	CommandDispatcher() {
		handlers.put("ping", args -> {
			JsonObject out = new JsonObject();
			out.addProperty("pong", true);
			out.addProperty("stage", "B2");
			return out;
		});
	}

	/**
	 * Enqueues a request and waits for the client thread to answer.
	 *
	 * @param request the parsed command object ({@code cmd}, optional {@code id})
	 * @return the handler result element
	 * @throws Exception on handler failure or when the client thread is stuck
	 */
	JsonElement run(JsonObject request) throws Exception {
		Task task = new Task(request);
		tasks.add(task);
		try {
			return task.result.get(HANDLER_TIMEOUT_MS, TimeUnit.MILLISECONDS);
		} catch (ExecutionException error) {
			Throwable cause = error.getCause() != null ? error.getCause() : error;
			if (cause instanceof Exception real) {
				throw real;
			}
			throw error;
		} catch (TimeoutException timeout) {
			throw new TimeoutException("client thread did not answer in " + HANDLER_TIMEOUT_MS + " ms");
		}
	}

	/** Client thread: executes every queued command. Called once per client tick. */
	void tick() {
		Task task;
		while ((task = tasks.poll()) != null) {
			try {
				task.result.complete(execute(task.request));
			} catch (Throwable error) {
				task.result.completeExceptionally(error);
			}
		}
	}

	private JsonElement execute(JsonObject request) throws Exception {
		String cmd = request.has("cmd") && request.get("cmd").isJsonPrimitive()
				? request.get("cmd").getAsString()
				: null;
		Handler handler = cmd == null ? null : handlers.get(cmd);
		if (handler == null) {
			throw new IllegalArgumentException(cmd == null ? "missing \"cmd\"" : "unknown cmd: " + cmd);
		}
		JsonObject args = request.has("args") && request.get("args").isJsonObject()
				? request.getAsJsonObject("args")
				: new JsonObject();
		return handler.handle(args);
	}
}
