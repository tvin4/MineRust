package dev.minerust.testclient;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.CompletableFuture;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.shapes.VoxelShape;

/**
 * {@code shapes}: writes the bottom of every block state's collision shape to a
 * JSON file, relative to the base of its block cell.
 *
 * <p>This is the data source for the server's free-headroom query (Шаг 15 п.5):
 * the headroom above a player depends on where the collision boxes of the block
 * above start, and `reports/blocks.json` carries state ids and properties but no
 * shapes. The values are read through the public
 * {@link BlockState#getCollisionShape} / {@link VoxelShape#min} API against the
 * connected world, in an air cell above the player, so shapes that depend on
 * neighbours are measured in the neutral (isolated) context.
 *
 * <p>States with an empty collision shape are omitted: they never obstruct a
 * head. The file stays outside the repository (E:/MineRust-data/reports).
 * Command: {@code {"cmd":"shapes","args":{"path":"<file>"}}}; the result carries
 * the counts and the path.
 */
final class ShapeReportCommand {
	private final Minecraft minecraft;

	ShapeReportCommand(Minecraft minecraft) {
		this.minecraft = minecraft;
	}

	void register(CommandDispatcher dispatcher) {
		dispatcher.register("shapes", this::write);
	}

	private CompletableFuture<JsonElement> write(JsonObject args) throws Exception {
		String path = args.has("path") && args.get("path").isJsonPrimitive()
				? args.get("path").getAsString()
				: "";
		if (path.isBlank()) {
			throw new IllegalArgumentException("shapes: missing \"path\"");
		}
		if (minecraft.player == null || minecraft.level == null) {
			throw new IllegalStateException("shapes: the client is not in a world");
		}
		// A cell high above the player: air around it, inside the loaded chunk
		// and below the build limit, so neighbour-dependent shapes see air.
		BlockPos probe = BlockPos.containing(
				minecraft.player.getX(), minecraft.player.getY() + 20.0, minecraft.player.getZ());

		JsonObject shapes = new JsonObject();
		int states = 0;
		for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
			if (state == null) {
				continue;
			}
			states++;
			VoxelShape shape = state.getCollisionShape(minecraft.level, probe);
			if (shape.isEmpty()) {
				continue;
			}
			shapes.addProperty(Integer.toString(Block.getId(state)), shape.min(Direction.Axis.Y));
		}

		JsonObject report = new JsonObject();
		report.addProperty("source", "BlockState.getCollisionShape(level,pos).min(Y), relative to the block cell; "
				+ "measured in an air cell above the player; empty shapes omitted");
		report.addProperty("states", states);
		report.add("shapes", shapes);
		Files.writeString(Path.of(path), report.toString(), StandardCharsets.UTF_8);

		JsonObject result = new JsonObject();
		result.addProperty("path", path);
		result.addProperty("states", states);
		result.addProperty("nonEmpty", shapes.size());
		return CompletableFuture.completedFuture(result);
	}
}