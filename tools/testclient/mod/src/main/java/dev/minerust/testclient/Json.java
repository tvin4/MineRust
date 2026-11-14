package dev.minerust.testclient;

import com.google.gson.JsonObject;

/** Small JSON argument helpers with uniform error messages. */
final class Json {
	private Json() {
	}

	/** Reads a required string member or throws a readable error. */
	static String requireString(JsonObject args, String name, String hint) {
		if (!args.has(name) || !args.get(name).isJsonPrimitive()) {
			throw new IllegalArgumentException("missing \"" + name + "\" (" + hint + ")");
		}
		return args.get(name).getAsString();
	}

	/** Reads a required number member or throws a readable error. */
	static double requireDouble(JsonObject args, String name, String hint) {
		if (!args.has(name) || !args.get(name).isJsonPrimitive() || !args.get(name).getAsJsonPrimitive().isNumber()) {
			throw new IllegalArgumentException("missing number \"" + name + "\" (" + hint + ")");
		}
		return args.get(name).getAsDouble();
	}
}
