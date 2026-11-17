package dev.minerust.testclient;

import java.util.concurrent.CompletableFuture;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.protocol.game.ServerboundPlayerActionPacket;
import net.minecraft.world.phys.BlockHitResult;

/**
 * {@code digpacket} — a **diagnostic-only** manual dig experiment (Шаг 17b).
 *
 * <p>Sends the client's own serverbound {@code player_action} start at the
 * block under the crosshair and, after {@code k} client ticks, the finish
 * (action 3) for the same block, face and sequence window. Only public types
 * are used: the packet record and {@link ClientPacketListener#send}.
 *
 * <p>Why not the game mode calls: the first two versions used
 * {@code MultiPlayerGameMode.stopDestroyBlock()} and then {@code destroyBlock},
 * and both send action 2 (abort) - the trace showed it, and an abort can never
 * be accepted by the server. The client sends action 3 only from its own
 * local-completion path, which happens no earlier than the full dig time; that
 * is exactly the value under test, so the finish has to be built from packets.
 *
 * <p>It is **not** a scenario input path — scenarios keep using {@code key}
 * (the KeyMapping path) and this command is never referenced by
 * `run-scenario.ps1`.
 *
 * <p>Args: {@code k} — client ticks between start and finish (default 3; at
 * least 1). The result reports the block, the face, the sequence numbers and
 * the k that was used, plus (Шаг 18.0.2) the client's own context at packet
 * time: {@code gameMode}, {@code onGround}, {@code heldItem}/{@code heldCount}
 * and {@code blockDestroySpeed} ({@code Player.getDestroySpeed(state)}), so a
 * run can tell whether the server's accept/reject boundary follows any of
 * them. All values come from public API only.
 */
final class DigPacketCommand {
	/** Sequence counter for the crafted packets; the server only echoes it. */
	private int sequence = 1;

	private final Minecraft minecraft;
	private final TickQueue ticks;

	DigPacketCommand(Minecraft minecraft, TickQueue ticks) {
		this.minecraft = minecraft;
		this.ticks = ticks;
	}

	void register(CommandDispatcher dispatcher) {
		dispatcher.register("digpacket", this::dig);
	}

	private CompletableFuture<JsonElement> dig(JsonObject args) {
		LocalPlayerCheck.requireInGame(minecraft);
		if (!(minecraft.hitResult instanceof BlockHitResult hit)) {
			throw new IllegalStateException(
					"digpacket: no block under the crosshair (hit=" + minecraft.hitResult + ")");
		}
		int k = (int) Json.optLong(args, "k", 3);
		if (k < 1) {
			throw new IllegalArgumentException("digpacket: k must be at least 1 client tick");
		}
		// The `start` flag exists for one control: a finish with no start must
		// be ignored (or rejected) by the server, which proves that the dig
		// timer, not a bare packet, decides the break.
		boolean withStart = !args.has("start") || args.get("start").getAsBoolean();
		ClientPacketListener connection = minecraft.getConnection();
		if (connection == null) {
			throw new IllegalStateException("digpacket: not connected");
		}
		BlockPos pos = hit.getBlockPos().immutable();
		Direction face = hit.getDirection();
		int startSequence = this.sequence++;
		int finishSequence = this.sequence++;
		if (withStart) {
			connection.send(new ServerboundPlayerActionPacket(
					ServerboundPlayerActionPacket.Action.START_DESTROY_BLOCK, pos, face, startSequence));
		}
		ticks.schedule(k, () -> {
			ClientPacketListener late = minecraft.getConnection();
			if (late != null) {
				late.send(new ServerboundPlayerActionPacket(
						ServerboundPlayerActionPacket.Action.STOP_DESTROY_BLOCK, pos, face, finishSequence));
			}
		});

		JsonObject out = new JsonObject();
		out.addProperty("x", pos.getX());
		out.addProperty("y", pos.getY());
		out.addProperty("z", pos.getZ());
		out.addProperty("face", face.getName());
		out.addProperty("k", k);
		out.addProperty("start", withStart);
		out.addProperty("startSequence", startSequence);
		out.addProperty("finishSequence", finishSequence);
		// Шаг 18.0.2: the client's own context at packet time, so a later run
		// can tell whether the accept/reject boundary follows the game mode,
		// the held item or the actual dig speed (all read through public
		// API, no Minecraft internals). `blockDestroySpeed` is
		// Player.getDestroySpeed(state) — the same value the hardness report
		// exports — which the game already divides by 5 while airborne or in
		// water.
		out.addProperty("gameMode", minecraft.gameMode == null ? "unknown"
				: minecraft.gameMode.getPlayerMode().getSerializedName());
		out.addProperty("onGround", minecraft.player == null || minecraft.player.onGround());
		out.addProperty("heldItem", minecraft.player == null ? "unknown"
				: net.minecraft.core.registries.BuiltInRegistries.ITEM
						.getKey(minecraft.player.getMainHandItem().getItem()).toString());
		out.addProperty("heldCount",
				minecraft.player == null ? 0 : minecraft.player.getMainHandItem().getCount());
		float destroySpeed = minecraft.player == null || minecraft.level == null ? 0.0F
				: minecraft.player.getDestroySpeed(minecraft.level.getBlockState(pos));
		out.addProperty("blockDestroySpeed", destroySpeed);
		return CompletableFuture.completedFuture(out);
	}
}