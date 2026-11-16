package dev.minerust.testclient;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.CompletableFuture;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;

/**
 * {@code hardness}: writes every block state's destroy speed (its hardness) and
 * the bare-hand dig speed to a JSON file.
 *
 * <p>This is the data source for the server-side mining timer (Шаг 16a):
 * `reports/blocks.json` carries state ids and properties but no hardness, so the
 * time to break a block cannot be derived from the reports. The values come from
 * the public game API only:
 *
 * <ul>
 *   <li>{@code BlockState.getDestroySpeed(BlockGetter, BlockPos)} — the state's
 *       hardness; a negative value means the state is unbreakable.</li>
 *   <li>{@code Player.getDestroySpeed(BlockState)} — the dig speed of the
 *       player holding its current items; the report run holds nothing, so this
 *       is the bare-hand speed.</li>
 * </ul>
 *
 * <p>Both are read against the connected world in an air cell above the player,
 * the same neutral context the {@code shapes} command uses, so
 * neighbour-dependent states are measured in isolation. No Minecraft internals
 * are read and nothing is assumed about the dig-time formula: the server derives
 * that from real dig traces (Шаг 16a.2).
 *
 * <p>The file stays outside the repository (E:/MineRust-data/reports).
 * Command: {@code {"cmd":"hardness","args":{"path":"<file>"}}}; the result
 * carries the counts and the path.
 */
final class HardnessReportCommand {
	private final Minecraft minecraft;

	HardnessReportCommand(Minecraft minecraft) {
		this.minecraft = minecraft;
	}

	void register(CommandDispatcher dispatcher) {
		dispatcher.register("hardness", this::write);
	}

	private CompletableFuture<JsonElement> write(JsonObject args) throws Exception {
		String path = args.has("path") && args.get("path").isJsonPrimitive()
				? args.get("path").getAsString()
				: "";
		if (path.isBlank()) {
			throw new IllegalArgumentException("hardness: missing \"path\"");
		}
		if (minecraft.player == null || minecraft.level == null) {
			throw new IllegalStateException("hardness: the client is not in a world");
		}
		// A cell high above the player: air around it, inside the loaded chunk
		// and below the build limit, so neighbour-dependent states see air.
		BlockPos probe = BlockPos.containing(
				minecraft.player.getX(), minecraft.player.getY() + 20.0, minecraft.player.getZ());

		JsonObject hardness = new JsonObject();
		JsonObject handSpeed = new JsonObject();
		JsonArray unbreakable = new JsonArray();
		int states = 0;
		for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
			if (state == null) {
				continue;
			}
			states++;
			String id = Integer.toString(Block.getId(state));
			float block = state.getDestroySpeed(minecraft.level, probe);
			hardness.addProperty(id, block);
			handSpeed.addProperty(id, minecraft.player.getDestroySpeed(state));
			if (block < 0.0F) {
				unbreakable.add(Integer.parseInt(id));
			}
		}

		JsonObject report = new JsonObject();
		report.addProperty("source", "BlockState.getDestroySpeed(level,pos) and Player.getDestroySpeed(state) "
				+ "with an empty hand; measured in an air cell above the player; hardness < 0 is unbreakable");
		report.addProperty("states", states);
		report.add("hardness", hardness);
		report.add("handSpeed", handSpeed);
		report.add("unbreakable", unbreakable);
		Files.writeString(Path.of(path), report.toString(), StandardCharsets.UTF_8);

		JsonObject result = new JsonObject();
		result.addProperty("path", path);
		result.addProperty("states", states);
		result.addProperty("unbreakable", unbreakable.size());
		return CompletableFuture.completedFuture(result);
	}
}
