package dev.minerust.testclient;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;

import java.util.concurrent.CompletableFuture;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

/**
 * {@code observe}: a read-only snapshot of the client state, used as a fact
 * source for the attack-input diagnostics (stage B3, owner decision: variant
 * 1 - facts first, then choose the attack path) and for the visibility
 * scenarios (remote-player form, stage B5).
 *
 * <p>Only public APIs verified by javap signatures against the Loom named
 * jar (26.3); no Minecraft internals are copied. One note: 26.3 has no
 * public {@code Minecraft.screen} accessor, so "screen is open" is reported
 * through its observable equivalent {@code MouseHandler.isMouseGrabbed()}
 * (SOURCE: signature-level observation, unverified semantics).
 */
final class ObserveCommand {
	private final Minecraft minecraft;

	ObserveCommand(Minecraft minecraft) {
		this.minecraft = minecraft;
	}

	/** Registers the {@code observe} command on the dispatcher. */
	void register(CommandDispatcher dispatcher) {
		dispatcher.register("observe", args -> {
			try {
				return CompletableFuture.completedFuture(snapshot(args));
			} catch (Exception error) {
				CompletableFuture<JsonElement> failed = new CompletableFuture<>();
				failed.completeExceptionally(error);
				return failed;
			}
		});
	}

	/**
	 * Remote-player snapshot: what this client (the observer) sees about
	 * another player. The position/rotation come from the observer's own
	 * ClientLevel (server-updated entity state); the game mode from the
	 * player-info entry (tab-list data). This is the "client B looks at
	 * player A" fact the visibility scenarios check.
	 */
	private JsonElement remoteSnapshot(String name) {
		JsonObject out = new JsonObject();
		out.addProperty("player", name);
		if (minecraft.level == null) {
			out.addProperty("found", false);
			out.addProperty("reason", "no level");
			return out;
		}
		AbstractClientPlayer target = null;
		for (AbstractClientPlayer candidate : minecraft.level.players()) {
			if (candidate.getScoreboardName().equalsIgnoreCase(name)) {
				target = candidate;
				break;
			}
		}
		if (target == null) {
			out.addProperty("found", false);
			return out;
		}
		out.addProperty("found", true);
		JsonObject pos = new JsonObject();
		pos.addProperty("x", target.getX());
		pos.addProperty("y", target.getY());
		pos.addProperty("z", target.getZ());
		out.add("pos", pos);
		JsonObject rot = new JsonObject();
		rot.addProperty("yaw", target.getYRot());
		rot.addProperty("pitch", target.getXRot());
		out.add("rot", rot);
		PlayerInfo info = minecraft.getConnection() == null ? null
				: minecraft.getConnection().getPlayerInfoIgnoreCase(name);
		out.addProperty("gameMode", info == null ? "unknown" : info.getGameMode().getSerializedName());
		return out;
	}

	private JsonElement snapshot(JsonObject args) throws Exception {
		JsonObject out = new JsonObject();
		if (args != null && args.has("player")) {
			return remoteSnapshot(args.get("player").getAsString());
		}
		LocalPlayer player = minecraft.player;
		if (player == null) {
			out.addProperty("inGame", false);
			return out;
		}
		out.addProperty("inGame", true);

		JsonObject pos = new JsonObject();
		pos.addProperty("x", player.getX());
		pos.addProperty("y", player.getY());
		pos.addProperty("z", player.getZ());
		out.add("pos", pos);

		JsonObject rot = new JsonObject();
		rot.addProperty("yaw", player.getYRot());
		rot.addProperty("pitch", player.getXRot());
		out.add("rot", rot);

		out.addProperty("gameMode", minecraft.gameMode.getPlayerMode().getSerializedName());
		out.addProperty("version", minecraft.getLaunchedVersion());
		out.addProperty("maxMemory", Runtime.getRuntime().maxMemory());

		// Attack-input gates (all public signatures, 26.3).
		out.addProperty("windowActive", minecraft.isWindowActive());
		out.addProperty("mouseGrabbed", minecraft.mouseHandler.isMouseGrabbed());
		out.addProperty("paused", minecraft.isPaused());
		out.addProperty("keyAttackDown", minecraft.options.keyAttack.isDown());
		out.addProperty("keyUseDown", minecraft.options.keyUse.isDown());

		HitResult hit = minecraft.hitResult;
		if (hit == null) {
			out.addProperty("hitType", "none");
		} else {
			out.addProperty("hitType", hit.getType().name());
			if (hit instanceof BlockHitResult blockHit) {
				JsonObject block = new JsonObject();
				block.addProperty("x", blockHit.getBlockPos().getX());
				block.addProperty("y", blockHit.getBlockPos().getY());
				block.addProperty("z", blockHit.getBlockPos().getZ());
				block.addProperty("face", blockHit.getDirection().name());
				out.add("hitBlock", block);
			}
			out.addProperty("crosshairEntity", minecraft.crosshairPickEntity == null ? "none"
					: minecraft.crosshairPickEntity.getClass().getSimpleName());
		}
		return out;
	}
}
