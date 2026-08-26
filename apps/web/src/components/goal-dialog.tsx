import type { AgentSession, ThreadGoal, ThreadGoalStatus } from '@waku/client'
import { useEffect, useState, type RefObject } from 'react'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { Textarea } from '@/components/ui/textarea'
import { WakuIcon, type WakuIconName } from '@/components/waku-icon'
import { useI18n } from '@/lib/i18n'
import { usePrimaryShortcut } from '@/lib/platform'
import { cn } from '@/lib/utils'

/** Mirror of Codex's `/goal edit` semantics: saving keeps a resumable status
 * but restarts a finished one. */
function editedGoalStatus(status: ThreadGoalStatus): ThreadGoalStatus {
  return status === 'complete' || status === 'budgetLimited' ? 'active' : status
}

export function goalStatusLabel(
  status: ThreadGoalStatus,
  t: (key: string) => string,
): string {
  switch (status) {
    case 'active': return t('goal.status_active')
    case 'paused': return t('goal.status_paused')
    case 'blocked': return t('goal.status_stalled')
    case 'usageLimited': return t('goal.status_usage_limited')
    case 'budgetLimited': return t('goal.status_budget_limited')
    case 'complete': return t('goal.status_complete')
  }
}

/** Status tint classes, always paired with the label text — never color alone. */
export function goalStatusClass(status: ThreadGoalStatus): string {
  switch (status) {
    case 'active': return 'text-ring'
    case 'paused': return 'text-[var(--text-secondary)]'
    case 'blocked':
    case 'usageLimited':
    case 'budgetLimited': return 'text-[var(--warning)]'
    case 'complete': return 'text-[var(--success)]'
  }
}

/** Compact elapsed time matching Codex's goal display: `45s`, `12m`,
 * `1h 30m`, `2d 3h 15m`. */
export function formatGoalElapsed(rawSeconds: number): string {
  const seconds = Math.max(0, Math.floor(rawSeconds))
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m`
  const hours = Math.floor(minutes / 60)
  const remainingMinutes = minutes % 60
  if (hours >= 24) {
    return `${Math.floor(hours / 24)}d ${hours % 24}h ${remainingMinutes}m`
  }
  return remainingMinutes === 0 ? `${hours}h` : `${hours}h ${remainingMinutes}m`
}

export function formatGoalTokens(tokens: number): string {
  const value = Math.max(0, tokens)
  if (value >= 999_500) return `${(value / 1_000_000).toFixed(1)}M`
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`
  return String(value)
}

/** The parenthetical consumption readout: budget-bounded goals report tokens,
 * unbudgeted ones elapsed pursuit time — the Codex CLI's own treatment. */
export function goalUsageReadout(goal: ThreadGoal, liveElapsedSeconds = 0): string | null {
  if (typeof goal.tokenBudget === 'number') {
    return `${formatGoalTokens(goal.tokensUsed)} / ${formatGoalTokens(goal.tokenBudget)}`
  }
  const seconds = goal.timeUsedSeconds + liveElapsedSeconds
  return seconds > 0 ? formatGoalElapsed(seconds) : null
}

export function GoalDialog({
  open,
  session,
  prefill,
  replace,
  returnFocus,
  onSubmit,
  onSetStatus,
  onClear,
  onOpenChange,
}: {
  open: boolean
  session: AgentSession | null
  /** Objective text to start the editor with; `null` prefills the current goal. */
  prefill: string | null
  /** Saving replaces the existing goal and restarts its accounting. */
  replace: boolean
  returnFocus?: RefObject<HTMLElement | null>
  onSubmit: (objective: string, status: ThreadGoalStatus, replace: boolean) => void
  onSetStatus: (status: ThreadGoalStatus) => void
  onClear: () => void
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useI18n()
  const [objective, setObjective] = useState('')
  const saveShortcut = usePrimaryShortcut('⌘↩', 'Ctrl+Enter')
  const goal = session?.thread_goal ?? null

  useEffect(() => {
    if (!open) return
    setObjective(prefill ?? goal?.objective ?? '')
    // The dialog seeds from the goal state at open; live accounting updates
    // must not clobber the draft while the user types.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const trimmed = objective.trim()
  const saveLabel = goal
    ? t(replace ? 'goal.replace' : 'goal.save')
    : t('goal.set')
  const statusAction = goal && !replace
    ? goal.status === 'active'
      ? { status: 'paused' as const, label: t('goal.pause'), icon: 'stop' as WakuIconName }
      : goal.status === 'paused' || goal.status === 'blocked' || goal.status === 'usageLimited'
        ? { status: 'active' as const, label: t('goal.resume'), icon: 'arrowUp' as WakuIconName }
        : null
    : null
  const usage = goal ? goalUsageReadout(goal) : null

  function save() {
    if (!trimmed) return
    const status = goal && !replace ? editedGoalStatus(goal.status) : 'active'
    onSubmit(trimmed, status, replace && Boolean(goal))
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="max-w-[420px] overflow-hidden rounded-[18px] bg-[var(--raised)] p-0"
        finalFocus={returnFocus}
      >
        <DialogTitle className="flex h-12 items-center gap-2.5 px-4 text-sm font-normal">
          <WakuIcon className="size-[15px]" name="target" />
          <span>{t('goal.title')}</span>
          {goal && (
            <span className={cn(
              'rounded-full bg-accent px-2 py-0.5 text-[12px]',
              goalStatusClass(goal.status),
            )}>
              {goalStatusLabel(goal.status, t)}
            </span>
          )}
          {usage && (
            <span className="min-w-0 truncate text-[12px] text-[var(--text-secondary)]">
              {usage}
            </span>
          )}
        </DialogTitle>
        <div className="px-4 pb-2.5">
          <Textarea
            autoFocus
            className="min-h-24 resize-none text-sm"
            placeholder={t('goal.objective_placeholder')}
            value={objective}
            onChange={(event) => setObjective(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
                event.preventDefault()
                save()
              }
            }}
          />
        </div>
        {replace && goal && (
          <p className="px-5 pb-2 text-[11.5px] leading-4 text-[var(--warning)]">
            {t('goal.replace_notice')}
          </p>
        )}
        <div className="mx-2 border-t" />
        <div className="flex flex-col gap-0.5 p-2">
          <GoalActionRow
            enabled={Boolean(trimmed)}
            icon="check"
            label={saveLabel}
            shortcut={saveShortcut}
            onClick={save}
          />
          {statusAction && (
            <GoalActionRow
              enabled
              icon={statusAction.icon}
              label={statusAction.label}
              onClick={() => {
                onSetStatus(statusAction.status)
                onOpenChange(false)
              }}
            />
          )}
          {goal && !replace && (
            <GoalActionRow
              destructive
              enabled
              icon="trash"
              label={t('goal.clear')}
              onClick={() => {
                onClear()
                onOpenChange(false)
              }}
            />
          )}
        </div>
      </DialogContent>
    </Dialog>
  )
}

function GoalActionRow({
  icon,
  label,
  shortcut,
  enabled,
  destructive,
  onClick,
}: {
  icon: WakuIconName
  label: string
  shortcut?: string
  enabled: boolean
  destructive?: boolean
  onClick: () => void
}) {
  return (
    <button
      className={cn(
        'flex min-h-9 w-full items-center gap-2.5 rounded-lg px-3 text-[13.5px] outline-none hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-40',
        destructive && 'text-destructive',
      )}
      disabled={!enabled}
      type="button"
      onClick={onClick}
    >
      <WakuIcon
        className={cn('size-3.5', destructive ? 'text-destructive' : 'text-[var(--text-secondary)]')}
        name={icon}
      />
      <span className="min-w-0 flex-1 truncate text-left">{label}</span>
      {shortcut && <span className="text-[11px] text-[var(--text-ghost)]">{shortcut}</span>}
    </button>
  )
}
