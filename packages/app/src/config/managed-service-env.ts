import { isValidSub2APIEndpoint } from "@/screens/settings/sub2api-auth-bridge";

/** Product builds must set `EXPO_PUBLIC_MANAGED_SERVICE_URL` via their brand wrapper. */
const DEFAULT_MANAGED_SERVICE_URL = "";

/**
 * Managed cloud service base URL: `EXPO_PUBLIC_MANAGED_SERVICE_URL` at bundle time.
 *
 * Staging example: `EXPO_PUBLIC_MANAGED_SERVICE_URL=https://staging.example.com npm run dev:desktop`
 */
export function getManagedServiceUrlFromEnv(): string {
  const raw = process.env.EXPO_PUBLIC_MANAGED_SERVICE_URL;
  const fromEnv = typeof raw === "string" ? raw.trim() : "";
  return fromEnv || DEFAULT_MANAGED_SERVICE_URL;
}

/**
 * `EXPO_PUBLIC_MANAGED_SERVICE_URL` was set at bundle time (non-empty).
 */
export function hasExplicitManagedServiceUrlEnv(): boolean {
  const raw = process.env.EXPO_PUBLIC_MANAGED_SERVICE_URL;
  return typeof raw === "string" && raw.trim().length > 0;
}

/**
 * Product builds never show a service-URL field in the UI; set `EXPO_PUBLIC_MANAGED_SERVICE_URL`
 * at build time instead.
 */
export function shouldShowManagedServiceUrlEditor(): boolean {
  return false;
}

export function isManagedServiceUrlEnvValid(): boolean {
  return isValidSub2APIEndpoint(getManagedServiceUrlFromEnv());
}
