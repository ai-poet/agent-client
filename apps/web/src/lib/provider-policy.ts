import type { AgentSession, ProviderKind, ProviderModel } from 'client'
import { sessionHasStarted } from './sidebar-presentation'

const EXTERNAL_CLI_ERRORS = [
  'external agent CLIs are not supported',
  'external CLI sessions cannot be restored by this build',
  'external CLI session forks are not supported',
  'external provider plan usage is not supported',
] as const

export function isExternalProvider(provider: ProviderKind): boolean {
  return provider !== 'native'
}

export function isExternalCliError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return EXTERNAL_CLI_ERRORS.some((refusal) => message.includes(refusal))
}

export function localizedErrorMessage(
  error: unknown,
  t: (key: string) => string,
): string {
  return isExternalCliError(error)
    ? t('errors.external_cli_unavailable')
    : error instanceof Error ? error.message : String(error)
}

/** Only built-in favorites survive the legacy external-CLI migration. */
export function parseNativeFavoriteModels(value: unknown): string[] {
  if (!Array.isArray(value)) return []
  return [...new Set(value.filter((key): key is string =>
    typeof key === 'string' && key.startsWith('native:'),
  ))]
}

/** The last built-in model wins, then the catalog default, then its first row. */
export function preferredNativeModel(
  lastModel: string | null,
  models: ProviderModel[],
): string | null {
  const remembered = lastModel?.trim()
  if (remembered) return remembered
  return models.find((model) => model.is_default)?.id ?? models[0]?.id ?? null
}

/** Migrate only a draft. Started external history is immutable and readable. */
export function redirectDraftToNative(
  session: AgentSession,
  model: string | null,
): AgentSession {
  if (!isExternalProvider(session.provider) || sessionHasStarted(session)) {
    return session
  }
  return {
    ...session,
    provider: 'native',
    model,
    reasoning_effort: null,
    service_tier: null,
    context_window: null,
    provider_cursor: null,
    agent_preset: null,
  }
}
