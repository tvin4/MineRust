package dev.minerust.testclient;

import java.io.File;
import java.util.HashMap;
import java.util.concurrent.CompletableFuture;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.ConnectScreen;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.resolver.ServerAddress;
import net.minecraft.client.multiplayer.TransferState;
import net.minecraft.client.Screenshot;
import net.minecraft.network.chat.Component;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.mojang.blaze3d.platform.NativeImage;

/**
 * Session commands (stage B4): {@code connect}, {@code disconnect},
 * {@code wait}, {@code command} and {@code screenshot}.
 *
 * <p>All of them go through public client APIs only:
 * {@link ConnectScreen#startConnecting} (the same entry the multiplayer
 * screen uses), {@code Minecraft.disconnectWithSavingScreen()},
 * {@code ClientPacketListener.sendCommand} (the chat-command path),
 * {@code Screenshot.takeScreenshot} (the F2 path) and a tick-scheduled
 * completion for {@code wait} (the client thread must never block).
 */
final class SessionCommands {
	/** Client tick rate (50 ms); used for the wait conversion. */
	private static final int MS_PER_TICK = 50;
	/** Upper bound for one wait (socket timeout is 30 s). */
	private static final long WAIT_MAX_MS = 20000;

	private final Minecraft minecraft;
	private final TickQueue ticks;
	/** Sequence number for default screenshot file names. */
	private int screenshotIndex;

	SessionCommands(Minecraft minecraft, TickQueue ticks) {
		this.minecraft = minecraft;
		this.ticks = ticks;
	}

	/** Registers the B4 commands on the dispatcher. */
	void register(CommandDispatcher dispatcher) {
		dispatcher.register("connect", args -> CompletableFuture.completedFuture(connect(args)));
		dispatcher.register("disconnect", args -> {
			// disconnectFromWorld is the multiplayer "Disconnect" button path;
			// disconnectWithSavingScreen (attempt 1) nulled the player but
			// left the server connection open until keepalive timeout.
			minecraft.disconnectFromWorld(Component.nullToEmpty("testclient disconnect"));
			JsonObject out = new JsonObject();
			out.addProperty("disconnecting", true);
			return CompletableFuture.completedFuture(out);
		});
		dispatcher.register("wait", this::wait);
		dispatcher.register("command", args -> CompletableFuture.completedFuture(command(args)));
		dispatcher.register("screenshot", this::screenshot);
	}

	/**
	 * connect: joins a server through the multiplayer screen path.
	 *
	 * <p>Args: {@code host} - address without port; {@code port} - optional
	 * (default 25565). The fifth startConnecting argument is false and the
	 * TransferState is empty (SOURCE: signatures only, unverified).
	 */
	private JsonElement connect(JsonObject args) {
		String host = Json.requireString(args, "host", "connect needs host (and optional port)");
		int port = (int) Json.optLong(args, "port", 25565);
		ServerAddress address = args.has("port")
				? new ServerAddress(host, port)
				: ServerAddress.parseString(host);
		ServerData data = new ServerData("testclient", address.toString(), ServerData.Type.OTHER);
		// 26.3 has no public Minecraft.screen accessor; null parent screen
		// (SOURCE: signatures only, unverified - ConnectScreen sets itself).
		ConnectScreen.startConnecting(
				null,
				minecraft,
				address,
				data,
				false,
				new TransferState(new HashMap<>(), new HashMap<>(), false));
		JsonObject out = new JsonObject();
		out.addProperty("host", address.getHost());
		out.addProperty("port", address.getPort());
		return out;
	}

	/**
	 * wait: completes the response after {@code ms} of real client ticks,
	 * so the world keeps ticking while the caller waits.
	 *
	 * <p>Args: {@code ms} - duration (default 1000, max 20000).
	 */
	private CompletableFuture<JsonElement> wait(JsonObject args) {
		long ms = Math.min(WAIT_MAX_MS, Json.optLong(args, "ms", 1000));
		CompletableFuture<JsonElement> future = new CompletableFuture<>();
		int delay = Math.max(1, (int) (ms / MS_PER_TICK));
		ticks.schedule(delay, () -> {
			JsonObject out = new JsonObject();
			out.addProperty("ms", ms);
			future.complete(out);
		});
		return future;
	}

	/**
	 * command: runs a chat command through the client's own chat path
	 * ({@code ClientPacketListener.sendCommand}). A leading {@code /} is
	 * optional. The server's reply arrives later as chat, not here.
	 */
	private JsonElement command(JsonObject args) throws Exception {
		LocalPlayerCheck.requireInGame(minecraft);
		String text = Json.requireString(args, "text", "command needs text");
		if (text.startsWith("/")) {
			text = text.substring(1);
		}
		minecraft.player.connection.sendCommand(text);
		JsonObject out = new JsonObject();
		out.addProperty("sent", text);
		return out;
	}

	/**
	 * screenshot: captures the main render target through the F2 screenshot
	 * path and writes it to {@code <gameDir>/screenshots/<name>.png}
	 * (default name {@code testclient-<n>}).
	 *
	 * <p>Args: {@code name} - optional file name without extension.
	 */
	private CompletableFuture<JsonElement> screenshot(JsonObject args) {
		String name = args.has("name") ? args.get("name").getAsString()
				: "testclient-" + (++screenshotIndex);
		File screenshots = new File(minecraft.gameDirectory, Screenshot.SCREENSHOT_DIR);
		File target = new File(screenshots, name + ".png");
		CompletableFuture<JsonElement> future = new CompletableFuture<>();
		Screenshot.takeScreenshot(minecraft.gameRenderer.mainRenderTarget(), image -> {
			try {
				writeAndComplete(image, target, future);
			} catch (Exception error) {
				future.completeExceptionally(error);
			}
		});
		return future;
	}

	/** Writes the captured frame and completes the response with the path. */
	private void writeAndComplete(NativeImage image, File target, CompletableFuture<JsonElement> future)
			throws Exception {
		try (NativeImage frame = image) {
			target.getParentFile().mkdirs();
			frame.writeToFile(target);
		}
		JsonObject out = new JsonObject();
		out.addProperty("file", target.getPath());
		out.addProperty("exists", target.isFile());
		future.complete(out);
	}
}
