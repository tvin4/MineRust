package dev.minerust.testclient;

import java.util.ArrayList;
import java.util.Iterator;
import java.util.List;

/**
 * Client-thread tick scheduler: actions run on the game thread after a
 * given number of client ticks (50 ms each). Shared by the input tap
 * completions and the {@code wait} command; pumped once per client tick
 * from the mod entrypoint.
 */
final class TickQueue {
	/** One delayed action. */
	private record Scheduled(int dueTick, Runnable action) {
	}

	/** One-based client tick counter, incremented once per tick. */
	private int tickCount;
	private final List<Scheduled> scheduled = new ArrayList<>();

	/** Schedules {@code action} to run on the client thread after {@code delayTicks}. */
	void schedule(int delayTicks, Runnable action) {
		scheduled.add(new Scheduled(tickCount + Math.max(1, delayTicks), action));
	}

	/** Client-thread pump: executes due actions. Called once per client tick. */
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
}
