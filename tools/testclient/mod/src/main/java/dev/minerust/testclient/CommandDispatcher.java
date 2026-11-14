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
 *
 * <p>A handler returns a future because some commands finish later than
 * the tick that starts them ({@code wait} completes after real client
 * ticks, screenshot after the frame capture callback).
 */
final class CommandDispatcher {
	/** Default how long the socket thread waits for the client thread. */
	static final long HANDLER_TIMEOUT_MS = 5000;
	/** Hard cap for the automatic wait-command timeout lift. */
	static final long REQUEST_TIMEOUT_MAX_MS = 30000;

	/** A command handler. Runs on the client thread only; may finish later. */
	@FunctionalInterface
	interface Handler {
		CompletableFuture<JsonElement> handle(JsonObject args) throws Exception;
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
		JsonObject pong = new JsonObject();
		pong.addProperty("pong", true);
		pong.addProperty("stage", "B4");
		handlers.put("ping", args -> CompletableFuture.completedFuture(pong));
	}

	/**
	 * Registers a command handler. Called during mod initialization, before
	 * the control socket accepts anything.
	 *
	 * @param name    command name ({@code "cmd"} value in requests)
	 * @param handler runs on the client thread when the command executes
	 */
	void register(String name, Handler handler) {
		Handler previous = handlers.put(name, handler);
		if (previous != null) {
			throw new IllegalStateException("command registered twice: " + name);
		}
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
		long timeout = HANDLER_TIMEOUT_MS;
		// A wait command answers after its own duration: lift the socket
		// wait above it automatically so the caller does not need
		// timeout_ms.
		if ("wait".equals(request.has("cmd") && request.get("cmd").isJsonPrimitive()
				? request.get("cmd").getAsString() : null)
				&& request.has("args") && request.get("args").isJsonObject()) {
			JsonObject waitArgs = request.getAsJsonObject("args");
			if (waitArgs.has("ms") && waitArgs.get("ms").isJsonPrimitive()
					&& waitArgs.get("ms").getAsJsonPrimitive().isNumber()) {
				timeout = Math.max(timeout, Math.min(25000, waitArgs.get("ms").getAsLong()) + 3000);
			}
		}
		try {
			return task.result.get(timeout, TimeUnit.MILLISECONDS);
		} catch (ExecutionException error) {
			Throwable cause = error.getCause() != null ? error.getCause() : error;
			if (cause instanceof Exception real) {
				throw real;
			}
			throw error;
		} catch (TimeoutException stuck) {
			throw new TimeoutException("client thread did not answer in " + timeout + " ms");
		}
	}

	/** Client thread: executes every queued command. Called once per client tick. */
	void tick() {
		Task task;
		while ((task = tasks.poll()) != null) {
			Task current = task;
			try {
				execute(current.request).whenComplete((value, error) -> {
					if (error != null) {
						current.result.completeExceptionally(error);
					} else {
						current.result.complete(value);
					}
				});
			} catch (Throwable error) {
				current.result.completeExceptionally(error);
			}
		}
	}

	private CompletableFuture<JsonElement> execute(JsonObject request) throws Exception {
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
