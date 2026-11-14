package dev.minerust.testclient;

import net.fabricmc.api.ClientModInitializer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Client entrypoint of the MineRust test-client mod (step 9, stage B).
 *
 * <p>Stage B1 is the skeleton: it only proves that the mod builds with
 * Loom and loads into the vanilla 26.3 client. The JSON control socket
 * (127.0.0.1 only) arrives in stage B2, the input-path commands (look/key,
 * must produce serverbound player_input/punch packets) in stage B3.
 */
public class TestClientMod implements ClientModInitializer {
	/** Mod logger, same logging pipeline as the game itself. */
	public static final Logger LOGGER = LoggerFactory.getLogger("testclient");

	@Override
	public void onInitializeClient() {
		LOGGER.info("testclient mod loaded (stage B1 skeleton)");
	}
}
