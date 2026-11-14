package dev.minerust.testclient;

import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;

import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.input.MouseButtonInfo;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.component.SwingAnimation;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

/**
 * Input-path commands: {@code look} and {@code key}.
 *
 * <p>Everything goes through the client's own input path so the vanilla
 * networking produces the genuine serverbound packets (player_input,
 * protocol id 43, and punch, id 46 - names and ids from
 * E:\MineRust-data\reports\packets.json). No packet is ever built or sent
 * from here.
 *
 * <p>Keys use {@link KeyMapping#setDown(boolean)} - the exact state the
 * keyboard handler maintains - so {@code ClientInput.tick()} rebuilds
 * {@code keyPresses} and the client sends player_input on the change.
 * {@code look} uses {@code Entity.turn(double, double)} - the mouse path;
 * arguments are raw mouse deltas, actual angles are read back via observe.
 * {@code swing} calls {@code LivingEntity.swing} - the same client-side
 * hand-swing path a real attack uses.
 */
final class InputCommands {
	/** Delayed actions: key taps complete over several client ticks. */
	private record Scheduled(int dueTick, Runnable action) {
	}

	/** Client tick rate (50 ms per tick); used to convert hold_ms to ticks. */
	private static final int MS_PER_TICK = 50;

	// GLFW action constants (public GLFW documentation, glfw3.h).
	private static final int GLFW_RELEASE = 0;
	private static final int GLFW_PRESS = 1;

	private final Minecraft minecraft;
	/** One-based tick counter, incremented once per client tick. */
	private int tickCount;
	private final List<Scheduled> scheduled = new ArrayList<>();

	InputCommands(Minecraft minecraft) {
		this.minecraft = minecraft;
	}

	/** Client-thread pump: executes due tap completions. Called every tick. */
	void tick() {
		tickCount++;
		Iterator<Scheduled> it = scheduled.iterator();
		while (it.hasNext()) {
			Scheduled task = it.next();
			if (task.dueTick() <= tickCount) {
				it.remove();
				task.action().run();
			}
		}
	}

	/** Registers the input commands on the dispatcher. */
	void register(CommandDispatcher dispatcher) {
		dispatcher.register("look", this::look);
		dispatcher.register("key", this::key);
	}

	/**
	 * look: turns the camera like the mouse does.
	 *
	 * <p>Args: {@code dx}/{@code dy} - mouse deltas (positive dx turns
	 * right, positive dy turns down). The client applies its own mouse
	 * sensitivity; read the resulting angles via {@code observe}.
	 */
	private JsonElement look(JsonObject args) throws Exception {
		LocalPlayerCheck.requireInGame(minecraft);
		double dx = Json.requireDouble(args, "dx", "look needs dx/dy mouse deltas");
		double dy = Json.requireDouble(args, "dy", "look needs dx/dy mouse deltas");
		minecraft.player.turn(dx, dy);
		JsonObject out = new JsonObject();
		out.addProperty("dx", dx);
		out.addProperty("dy", dy);
		return out;
	}

	/**
	 * key: presses, releases or taps one of the client's key bindings.
	 *
	 * <p>Args: {@code key} - up/down/left/right/jump/shift/sprint/attack/
	 * use/swing; {@code action} - down, up or tap (default tap);
	 * {@code hold_ms} - tap duration (default 100 ms). attack/use replay the
	 * GLFW mouse button callback ({@code MouseHandler.onButton}) and, unlike
	 * held keys, can also be held with down/up/tap so the client's own mouse
	 * handling (attack, block breaking, item use) runs exactly as with a
	 * real click, including sustained digging.
	 */
	private JsonElement key(JsonObject args) throws Exception {
		LocalPlayerCheck.requireInGame(minecraft);
		String key = Json.requireString(args, "key", "key needs a key name");
		String action = args.has("action") ? args.get("action").getAsString() : "tap";
		long holdMs = args.has("hold_ms") ? args.get("hold_ms").getAsLong() : 100;

		JsonObject out = new JsonObject();
		out.addProperty("key", key);
		out.addProperty("action", action);

		switch (key) {
			case "up", "down", "left", "right", "jump", "shift", "sprint" -> heldKey(key, action, holdMs);
			case "attack" -> heldMouse(minecraft.options.keyAttack, action, holdMs, path(args));
			case "use" -> heldMouse(minecraft.options.keyUse, action, holdMs, path(args));
			case "swing" -> {
				requireMomentary(action, key);
				swing(args);
			}
			default -> throw new IllegalArgumentException("unknown key: " + key
					+ " (up/down/left/right/jump/shift/sprint/attack/use/swing)");
		}
		return out;
	}

	private void heldKey(String key, String action, long holdMs) {
		if ("tap".equals(action)) {
			setDown(key, true);
			scheduleTapUp(key, holdMs);
			return;
		}
		if (!"down".equals(action) && !"up".equals(action)) {
			throw new IllegalArgumentException("unknown action: " + action + " (down/up/tap)");
		}
		setDown(key, "down".equals(action));
	}

	private void scheduleTapUp(String key, long holdMs) {
		int delay = Math.max(1, (int) (holdMs / MS_PER_TICK));
		scheduled.add(new Scheduled(tickCount + delay, () -> setDown(key, false)));
	}

	private void setDown(String key, boolean down) {
		Options options = minecraft.options;
		KeyMapping mapping = switch (key) {
			case "up" -> options.keyUp;
			case "down" -> options.keyDown;
			case "left" -> options.keyLeft;
			case "right" -> options.keyRight;
			case "jump" -> options.keyJump;
			case "shift" -> options.keyShift;
			case "sprint" -> options.keySprint;
			default -> throw new IllegalArgumentException("unknown held key: " + key);
		};
		mapping.setDown(down);
	}

	/**
	 * Presses, releases or taps the attack/use binding through the client's
	 * key-binding path (owner variant 2). Default. {@code path: "mouse"}
	 * switches to the GLFW mouse callback path (MouseHandler.onButton) -
	 * kept for comparison: with all observe gates open (window active, mouse
	 * grabbed, no screen, crosshair on a block) it produced no serverbound
	 * packets at all.
	 */
	private void heldMouse(KeyMapping mapping, String action, long holdMs, String path) {
		if ("mouse".equals(path)) {
			heldMouseButton(action, holdMs, mapping == minecraft.options.keyAttack ? 0 : 1);
			return;
		}
		if ("tap".equals(action)) {
			mappingDown(mapping, true);
			int delay = Math.max(1, (int) (Math.max(holdMs, 100) / MS_PER_TICK));
			scheduled.add(new Scheduled(tickCount + delay, () -> mappingDown(mapping, false)));
			return;
		}
		if (!"down".equals(action) && !"up".equals(action)) {
			throw new IllegalArgumentException("unknown action: " + action + " (down/up/tap)");
		}
		mappingDown(mapping, "down".equals(action));
	}

	/**
	 * Key-binding path: setDown holds the binding the same way the keyboard
	 * handler does, click() replays the GLFW key click for the momentary
	 * attack trigger. Both are public KeyMapping APIs.
	 */
	private void mappingDown(KeyMapping mapping, boolean down) {
		mapping.setDown(down);
		if (down) {
			KeyMapping.click(mapping.getDefaultKey());
		}
	}

	/** Old path: replays one GLFW mouse button tap (press now, release later). */
	private void heldMouseButton(String action, long holdMs, int button) {
		if ("tap".equals(action)) {
			mouseButton(button, GLFW_PRESS);
			int delay = Math.max(1, (int) (100 / MS_PER_TICK));
			scheduled.add(new Scheduled(tickCount + delay, () -> mouseButton(button, GLFW_RELEASE)));
			return;
		}
		if (!"down".equals(action) && !"up".equals(action)) {
			throw new IllegalArgumentException("unknown action: " + action + " (down/up/tap)");
		}
		mouseButton(button, "down".equals(action) ? GLFW_PRESS : GLFW_RELEASE);
	}

	/** Optional {@code path} argument: "mapping" (default) or "mouse". */
	private String path(JsonObject args) {
		return args.has("path") ? args.get("path").getAsString() : "mapping";
	}

	private void mouseButton(int button, int action) {
		minecraft.mouseHandler.onButton(
				minecraft.getWindow().handle(),
				new MouseButtonInfo(button, 0),
				action);
	}

	/**
	 * Swing: the client-side hand-swing path. Args: {@code broadcast} -
	 * the third swing argument (default true: the signature alone does not
	 * say what it means; SOURCE: unverified, acceptance is the wire trace
	 * showing serverbound punch, id 46).
	 */
	private void swing(JsonObject args) {
		boolean broadcast = !args.has("broadcast") || args.get("broadcast").getAsBoolean();
		minecraft.player.swing(InteractionHand.MAIN_HAND, SwingAnimation.DEFAULT, broadcast);
	}

	private static void requireMomentary(String action, String key) {
		if (!"tap".equals(action)) {
			throw new IllegalArgumentException(key + " is momentary: only action=tap");
		}
	}
}
