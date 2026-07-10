import { describe, expect, test } from "vitest";

import { CodexAppServerAgentClient } from "./codex-app-server-agent.js";
import { createTestLogger } from "../../../test-utils/test-logger.js";

describe("CodexAppServerAgentClient spawn error handling", () => {
  const logger = createTestLogger();

  test("listModels uses the hardcoded catalog when the codex binary does not exist", async () => {
    const client = new CodexAppServerAgentClient(logger, {
      command: {
        mode: "replace",
        argv: ["/nonexistent/codex-binary-that-does-not-exist"],
      },
    });

    await expect(client.listModels({ cwd: "/tmp/codex-models", force: false })).resolves.toEqual(
      expect.arrayContaining([expect.objectContaining({ id: "gpt-5.4", isDefault: true })]),
    );
  });

  test("listPersistedAgents rejects gracefully when the codex binary does not exist", async () => {
    const client = new CodexAppServerAgentClient(logger, {
      command: {
        mode: "replace",
        argv: ["/nonexistent/codex-binary-that-does-not-exist"],
      },
    });

    const uncaughtErrors: unknown[] = [];
    const onUncaught = (err: unknown) => {
      uncaughtErrors.push(err);
    };
    process.on("uncaughtException", onUncaught);

    try {
      await expect(client.listPersistedAgents()).rejects.toThrow();
      await new Promise((resolve) => setTimeout(resolve, 100));
      expect(uncaughtErrors).toHaveLength(0);
    } finally {
      process.off("uncaughtException", onUncaught);
    }
  });
});
