// graphify OpenCode integration.
//
// OpenCode runs two plugin loaders, and a single default export can satisfy
// both (the documented "Support V1 and V2" layout):
//   * the server runtime loads a V2 plugin: a default-exported object with
//     `id` and `setup(ctx)` (or `effect`);
//   * the CLI runtime loads a V1 plugin: a default-exported object with
//     `server()` that returns a hooks map.
// Exporting both shapes from one object keeps either loader happy. A bare
// named export (the previous version of this file) loads in neither and makes
// OpenCode log "Plugin must export a default definition with an id and an
// effect or setup function".
//
// The integration reminds the agent to consult the knowledge graph once per
// session, before the first shell command, when graphify-out/graph.json exists.
//
// IMPORTANT: keep REMINDER free of backticks and $(...). It is placed inside a
// double-quoted echo; backticks or $(...) there trigger shell command
// substitution, which corrupts output and can run the suggested command.
//
// SOURCE: opencode.ai/docs/build/plugins/server (V2 tool hook + V1/V2 layout).
// On OpenCode 1.17.13 the V2 plugin context exposes no tool/shell hook (only
// agent/aisdk/catalog/command/integration/plugin/reference/skill), so the V2
// path cannot inject the reminder there; it becomes active on newer releases.
import { existsSync } from "node:fs";
import { join } from "node:path";

const REMINDER =
  "[graphify] knowledge graph at graphify-out/. For focused questions, run graphify query with your question (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context.";

// Prepend the reminder to a shell command once, if the graph exists.
// The V1 hook mutates output.args.command; the V2 hook mutates event.input.
function prependReminder(command, directory) {
  if (!existsSync(join(directory, "graphify-out", "graph.json"))) return command;
  // ';' not '&&' — Windows PowerShell 5.1 rejects '&&' as a statement separator,
  // which would break the first shell command of the session (#1646).
  return `echo "${REMINDER}" ; ${command}`;
}

export default {
  id: "graphify",
  async setup(ctx) {
    const hook = ctx?.tool?.hook;
    if (typeof hook !== "function") return; // no tool hook in this OpenCode version

    const directory = ctx?.location?.directory ?? process.cwd();
    let reminded = false;
    await hook("execute.before", (event) => {
      if (reminded) return;
      if (event?.tool !== "bash") return;
      if (typeof event?.input?.command !== "string") return;
      const next = prependReminder(event.input.command, directory);
      if (next === event.input.command) return;
      event.input.command = next;
      reminded = true;
    });
  },
  async server() {
    let reminded = false;
    return {
      "tool.execute.before": async (input, output) => {
        if (reminded) return;
        if (input?.tool !== "bash") return;
        if (typeof output?.args?.command !== "string") return;
        const next = prependReminder(output.args.command, process.cwd());
        if (next === output.args.command) return;
        output.args.command = next;
        reminded = true;
      },
    };
  },
};
