package dev.minerust.testclient;

import java.util.concurrent.CompletableFuture;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;

/**
 * {@code tp}: moves the local player to a position, the way a server teleport
 * does ({@code Entity.setPos} plus the rotation setters).
 *
 * <p>This is the client-side teleport the S10/S11 runs use against our own
 * server, which has no RCON and no {@code /tp}: the client's own movement path
 * sends the new position to the server, and our server applies client
 * positions directly. Against vanilla the same scenarios keep using the
 * server console, because a vanilla server rejects an unannounced jump.
 *
 * <p>Args: {@code x}, {@code y}, {@code z} required; {@code yaw} and
 * {@code pitch} optional (default: keep the current rotation).
 */
final class TeleportCommand {
	private final Minecraft minecraft;

	TeleportCommand(Minecraft minecraft) {
		this.minecraft = minecraft;
	}

	/** Registers the {@code tp} command on the dispatcher. */
	void register(CommandDispatcher dispatcher) {
		dispatcher.register("tp", args -> CompletableFuture.completedFuture(teleport(args)));
	}

	private JsonElement teleport(JsonObject args) {
		LocalPlayerCheck.requireInGame(minecraft);
		double x = Json.requireDouble(args, "x", "tp needs x/y/z");
		double y = Json.requireDouble(args, "y", "tp needs x/y/z");
		double z = Json.requireDouble(args, "z", "tp needs x/y/z");
		float yaw = args.has("yaw") ? (float) args.get("yaw").getAsDouble() : minecraft.player.getYRot();
		float pitch = args.has("pitch") ? (float) args.get("pitch").getAsDouble() : minecraft.player.getXRot();
		minecraft.player.setPos(x, y, z);
		minecraft.player.setYRot(yaw);
		minecraft.player.setXRot(pitch);

		JsonObject out = new JsonObject();
		out.addProperty("x", x);
		out.addProperty("y", y);
		out.addProperty("z", z);
		out.addProperty("yaw", yaw);
		out.addProperty("pitch", pitch);
		return out;
	}
}