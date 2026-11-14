package dev.minerust.testclient;

import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.minecraft.client.Minecraft;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Client entrypoint of the MineRust test-client mod (step 9, stage B).
 *
 * <p>Stage B2 starts the JSON control socket (127.0.0.1 only, one command
 * per line) and pumps its commands on the client tick. The input-path
 * commands (look/key, which must produce serverbound player_input/punch,
 * protocol ids 43/46 per reports/packets.json) arrive in stage B3.
 */
public class TestClientMod implements ClientModInitializer {
	/** Mod logger, same logging pipeline as the game itself. */
	public static final Logger LOGGER = LoggerFactory.getLogger("testclient");

	/** Default control port; override with {@code -Dtestclient.port=<port>}. */
	static final int DEFAULT_PORT = 25571;

	@Override
	public void onInitializeClient() {
		CommandDispatcher dispatcher = new CommandDispatcher();
		Minecraft minecraft = Minecraft.getInstance();
		InputCommands input = new InputCommands(minecraft);
		input.register(dispatcher);
		new ObserveCommand(minecraft).register(dispatcher);
		int port = Integer.getInteger("testclient.port", DEFAULT_PORT);
		new ControlServer(port, dispatcher).start();
		ClientTickEvents.END_CLIENT_TICK.register(client -> {
			input.tick();
			dispatcher.tick();
		});
		LOGGER.info("testclient mod loaded (stage B3: input-path commands)");
	}
}
