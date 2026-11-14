package dev.minerust.testclient;

import net.minecraft.client.Minecraft;

/**
 * Guard: commands that touch the game world only run while a player is
 * connected. Runs on the client thread (CommandDispatcher contract).
 */
final class LocalPlayerCheck {
	private LocalPlayerCheck() {
	}

	/** Throws when the client is not in a world (no player object). */
	static void requireInGame(Minecraft minecraft) {
		if (minecraft.player == null) {
			throw new IllegalStateException("client is not in game");
		}
	}
}
