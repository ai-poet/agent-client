import { describe, expect, test } from 'bun:test'
import type { AgentSession, ProviderModel } from 'client'
import { createSession } from './daemon-api'
import {
  isExternalCliError,
  localizedErrorMessage,
  parseNativeFavoriteModels,
  preferredNativeModel,
  redirectDraftToNative,
  runtimeBinaryForProbe,
} from './provider-policy'

describe('built-in provider policy', () => {
  test('moves an external draft to native with the preferred model', () => {
    const draft = createSession('project', 'codex', false)
    const redirected = redirectDraftToNative(draft, 'anthropic::claude-sonnet-5-5')
    expect(redirected).toMatchObject({
      id: draft.id,
      provider: 'native',
      model: 'anthropic::claude-sonnet-5-5',
      provider_cursor: null,
      reasoning_effort: null,
    })
    expect(redirectDraftToNative(redirected, null)).toBe(redirected)
  })

  test('never rewrites started external history', () => {
    const started = {
      ...createSession('project', 'claude', false),
      messages: [{ id: 'message' } as AgentSession['messages'][number]],
    }
    expect(redirectDraftToNative(started, 'native-model')).toBe(started)
    expect(started.provider).toBe('claude')
  })

  test('uses the remembered native model, then the catalog default', () => {
    const models = [
      { id: 'first', is_default: false },
      { id: 'default', is_default: true },
    ] as ProviderModel[]
    expect(preferredNativeModel('remembered', models)).toBe('remembered')
    expect(preferredNativeModel(null, models)).toBe('default')
    expect(preferredNativeModel('', [])).toBeNull()
  })

  test('drops external and malformed favorites', () => {
    expect(parseNativeFavoriteModels([
      'codex:gpt-5.6-sol',
      'native:anthropic::claude-sonnet-5-5',
      'native:anthropic::claude-sonnet-5-5',
      42,
    ])).toEqual(['native:anthropic::claude-sonnet-5-5'])
    expect(parseNativeFavoriteModels(null)).toEqual([])
  })

  test('allows native runtime startup without a binary path only when installed', () => {
    expect(runtimeBinaryForProbe({ provider: 'native', installed: true, path: null })).toBe('')
    expect(runtimeBinaryForProbe({ provider: 'native', installed: true, path: '/native' }))
      .toBe('/native')
    expect(runtimeBinaryForProbe({ provider: 'native', installed: false, path: null })).toBeNull()
  })

  test('still requires an installed executable path for external runtimes', () => {
    expect(runtimeBinaryForProbe({ provider: 'codex', installed: true, path: null })).toBeNull()
    expect(runtimeBinaryForProbe({ provider: 'codex', installed: true, path: '' })).toBeNull()
    expect(runtimeBinaryForProbe({ provider: 'codex', installed: false, path: '/codex' }))
      .toBeNull()
    expect(runtimeBinaryForProbe({ provider: 'codex', installed: true, path: '/codex' }))
      .toBe('/codex')
  })

  test('maps direct and wrapped daemon refusals to the friendly locale key', () => {
    for (const message of [
      'external agent CLIs are not supported',
      'external CLI sessions cannot be restored by this build',
      'external CLI session forks are not supported',
      'external provider plan usage is not supported',
    ]) {
      expect(isExternalCliError(message)).toBe(true)
      expect(localizedErrorMessage(new Error(`Could not fork: ${message}`), (key) => key))
        .toBe('errors.external_cli_unavailable')
    }
    expect(localizedErrorMessage(new Error('daemon disconnected'), (key) => key))
      .toBe('daemon disconnected')
  })
})
