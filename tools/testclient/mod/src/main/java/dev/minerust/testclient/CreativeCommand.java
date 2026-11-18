package dev.minerust.testclient;

import java.util.concurrent.CompletableFuture;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import net.minecraft.client.Minecraft;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;

/**
 * {@code creative}: puts an item into the creative inventory through the same
 * public client method a creative-menu slot click ends in
 * ({@code MultiPlayerGameMode.handleCreativeModeItemAdd}), so the client emits
 * its genuine serverbound {@code set_creative_mode_slot} (Шаг 24.0a; id 57 in
 * reports/packets.json). The packet type
 * {@code ServerboundSetCreativeModeSlotPacket} carries only {@code slotNum}
 * (short) and the {@code ItemStack}; there is no container id on the wire.
 *
 * <p>This is not a hand-built packet. The creative screen decides which item
 * was clicked from mouse pixel coordinates, and a bare
 * {@code handleCreativeModeItemAdd} sends the packet but does not touch the
 * local inventory: the screen also updates it. The command therefore mirrors
 * the exact pair of calls the vanilla client performs when it plants a saved
 * stack into a hotbar slot (the creative hotbar load path,
 * {@code CreativeModeInventoryScreen}: {@code Inventory.setItem} followed by
 * {@code handleCreativeModeItemAdd}), so both the local inventory and the
 * server-side copy match what a real menu click produces.
 *
 * <p>Args: {@code item} - item id (a missing namespace becomes
 * {@code minecraft:}); {@code slot} - destination container slot (default the
 * selected hotbar slot, 36..44 in the inventory menu numbering); {@code count}
 * - stack size (default 64, 1..64). The player must have infinite materials
 * (creative/spectator). The result echoes what was sent.
 */
final class CreativeCommand {
	private final Minecraft minecraft;

	CreativeCommand(Minecraft minecraft) {
		this.minecraft = minecraft;
	}

	/** Registers the {@code creative} command. */
	void register(CommandDispatcher dispatcher) {
		dispatcher.register("creative", args -> CompletableFuture.completedFuture(add(args)));
	}

	private JsonElement add(JsonObject args) {
		LocalPlayerCheck.requireInGame(minecraft);
		if (!minecraft.player.hasInfiniteMaterials()) {
			throw new IllegalStateException("creative: the player is not in creative mode");
		}
		String name = Json.requireString(args, "item", "creative needs the item id");
		if (!name.contains(":")) {
			name = "minecraft:" + name;
		}
		Identifier id = Identifier.parse(name);
		if (!BuiltInRegistries.ITEM.containsKey(id)) {
			throw new IllegalArgumentException("creative: unknown item " + name);
		}
		Item item = BuiltInRegistries.ITEM.getValue(id);
		int count = (int) Json.optLong(args, "count", 64);
		if (count < 1 || count > 64) {
			throw new IllegalArgumentException("creative: count must be 1..64");
		}
		int slot = args.has("slot") && args.get("slot").isJsonPrimitive()
				? args.get("slot").getAsInt()
				: 36 + minecraft.player.getInventory().getSelectedSlot();
		ItemStack stack = new ItemStack(item, count);
		int inventoryIndex = inventoryIndex(slot);
		if (inventoryIndex >= 0) {
			// Same order as the vanilla creative hotbar-load loop: update the
			// local inventory, then let the game build and send the packet.
			minecraft.player.getInventory().setItem(inventoryIndex, stack);
		}
		minecraft.gameMode.handleCreativeModeItemAdd(stack, slot);

		JsonObject out = new JsonObject();
		out.addProperty("item", name);
		out.addProperty("slot", slot);
		out.addProperty("inventoryIndex", inventoryIndex);
		out.addProperty("count", count);
		out.addProperty("selectedSlot", minecraft.player.getInventory().getSelectedSlot());
		return out;
	}

	/**
	 * Maps a player-inventory menu slot id to the {@code Inventory} index the
	 * local inventory uses: menu 36..44 (hotbar) -&gt; 0..8, menu 9..35 (main)
	 * -&gt; 9..35, menu 5..8 (armor, head..feet) -&gt; 39..36, menu 45 (offhand)
	 * -&gt; 40. Returns -1 for a slot the local inventory does not hold.
	 */
	private static int inventoryIndex(int menuSlot) {
		if (menuSlot >= 36 && menuSlot <= 44) {
			return menuSlot - 36;
		}
		if (menuSlot >= 9 && menuSlot <= 35) {
			return menuSlot;
		}
		if (menuSlot >= 5 && menuSlot <= 8) {
			return 39 - (menuSlot - 5);
		}
		return menuSlot == 45 ? 40 : -1;
	}
}
