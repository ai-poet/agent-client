import { describe, expect, it, vi } from "vitest";
import {
  defaultModelsForTarget,
  fetchGatewayModelIds,
  filterGatewayModelsForTarget,
  gatewayModelsUrl,
  parseGatewayModelIds,
  targetNeedsModelList,
} from "./gateway-models";

describe("defaultModelsForTarget", () => {
  it("gives Grok something writable when the catalog cannot be read", () => {
    expect(defaultModelsForTarget("grok")).toEqual(["grok-4.6"]);
  });

  it("has no defaults for targets that do not pin a model list", () => {
    expect(defaultModelsForTarget("pi")).toEqual([]);
    expect(defaultModelsForTarget("claude")).toEqual([]);
  });
});

describe("targetNeedsModelList", () => {
  it("only asks for a catalog for targets whose config embeds a model list", () => {
    expect(targetNeedsModelList("grok")).toBe(true);
    expect(targetNeedsModelList("pi")).toBe(true);
    expect(targetNeedsModelList("claude")).toBe(false);
    expect(targetNeedsModelList("codex")).toBe(false);
  });
});

describe("filterGatewayModelsForTarget", () => {
  const catalog = ["grok-4.5", "GROK-3-mini", "gpt-5.4", "claude-opus-5"];

  it("narrows Grok to the xAI family, case-insensitively", () => {
    expect(filterGatewayModelsForTarget("grok", catalog)).toEqual(["grok-4.5", "GROK-3-mini"]);
  });

  it("leaves Pi unrestricted", () => {
    expect(filterGatewayModelsForTarget("pi", catalog)).toEqual(catalog);
  });
});

describe("gatewayModelsUrl", () => {
  it("appends /v1/models to a bare origin", () => {
    expect(gatewayModelsUrl("https://api.example.com")).toBe("https://api.example.com/v1/models");
  });

  it("does not double up when the row already carries /v1", () => {
    expect(gatewayModelsUrl("https://api.example.com/v1")).toBe(
      "https://api.example.com/v1/models",
    );
  });

  it("tolerates trailing slashes", () => {
    expect(gatewayModelsUrl("https://api.example.com/v1/")).toBe(
      "https://api.example.com/v1/models",
    );
  });
});

describe("parseGatewayModelIds", () => {
  it("reads ids and drops duplicates", () => {
    expect(
      parseGatewayModelIds({ data: [{ id: "a" }, { id: "a" }, { id: " b " }] }),
    ).toEqual(["a", "b"]);
  });

  it("skips malformed entries instead of throwing", () => {
    expect(parseGatewayModelIds({ data: [null, 42, { id: 7 }, { id: "ok" }] })).toEqual(["ok"]);
  });

  it("returns nothing for a body that is not a catalog", () => {
    expect(parseGatewayModelIds({})).toEqual([]);
    expect(parseGatewayModelIds(null)).toEqual([]);
    expect(parseGatewayModelIds({ data: "nope" })).toEqual([]);
  });
});

describe("fetchGatewayModelIds", () => {
  it("sends the key as a bearer token and returns the ids", async () => {
    const fetchImpl = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ data: [{ id: "grok-4.5" }] }),
    });

    await expect(
      fetchGatewayModelIds({
        endpoint: "https://api.example.com",
        apiKey: "sk-test",
        fetchImpl: fetchImpl as unknown as typeof globalThis.fetch,
      }),
    ).resolves.toEqual(["grok-4.5"]);

    expect(fetchImpl).toHaveBeenCalledWith("https://api.example.com/v1/models", {
      headers: { Authorization: "Bearer sk-test" },
    });
  });

  it("throws on a failed request rather than reporting an empty catalog", async () => {
    const fetchImpl = vi.fn().mockResolvedValue({ ok: false, status: 401, json: async () => ({}) });

    await expect(
      fetchGatewayModelIds({
        endpoint: "https://api.example.com",
        apiKey: "bad",
        fetchImpl: fetchImpl as unknown as typeof globalThis.fetch,
      }),
    ).rejects.toThrow("HTTP 401");
  });
});
