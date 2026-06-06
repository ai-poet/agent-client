import { afterEach, describe, expect, it } from "vitest";
import {
  getManagedServiceUrlFromEnv,
  hasExplicitManagedServiceUrlEnv,
  isManagedServiceUrlEnvValid,
} from "./managed-service-env";

describe("managed-service-env", () => {
  const original = process.env.EXPO_PUBLIC_MANAGED_SERVICE_URL;

  afterEach(() => {
    if (original === undefined) {
      delete process.env.EXPO_PUBLIC_MANAGED_SERVICE_URL;
    } else {
      process.env.EXPO_PUBLIC_MANAGED_SERVICE_URL = original;
    }
  });

  it("has no shipped managed service endpoint without an explicit build env", () => {
    delete process.env.EXPO_PUBLIC_MANAGED_SERVICE_URL;

    expect(getManagedServiceUrlFromEnv()).toBe("");
    expect(hasExplicitManagedServiceUrlEnv()).toBe(false);
    expect(isManagedServiceUrlEnvValid()).toBe(false);
  });

  it("allows explicit managed service endpoint overrides", () => {
    process.env.EXPO_PUBLIC_MANAGED_SERVICE_URL = " https://staging.cheaprouter.org ";

    expect(getManagedServiceUrlFromEnv()).toBe("https://staging.cheaprouter.org");
    expect(hasExplicitManagedServiceUrlEnv()).toBe(true);
  });
});
