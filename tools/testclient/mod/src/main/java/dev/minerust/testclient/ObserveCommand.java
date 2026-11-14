package dev.minerust.testclient;

import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;

import java.util.concurrent.CompletableFuture;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

/**
 * {@code observe}: a read-only snapshot of the client state, used as a fact
 * source for the attack-input diagnostics (stage B3, owner decision: variant
 * 1 - facts first, then choose the attack path).
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
				return CompletableFuture.completedFuture(snapshot());
			} catch (Exception error) {
				CompletableFuture<JsonElement> failed = new CompletableFuture<>();
				failed.completeExceptionally(error);
				return failed;
			}
		});
	}

	private JsonElement snapshot() throws Exception {
		JsonObject out = new JsonObject();
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
