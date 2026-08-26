import {
  ArrowUp,
  ChevronDown,
  ChevronRight,
  CircleCheck,
  Folder,
  History,
  Pencil,
  Terminal,
} from 'lucide-react'
import type { ReactNode } from 'react'

function Chip({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1 rounded-[5px] border border-white/8 px-1.5 py-0.5 font-mono text-[9.5px] leading-none text-zinc-400">
      {children}
    </span>
  )
}

function SessionRow({
  title,
  active,
  running,
}: {
  title: string
  active?: boolean
  running?: boolean
}) {
  return (
    <div
      className={`flex items-center gap-1.5 rounded-md px-2 py-[5px] ${
        active ? 'bg-white/8 text-zinc-200' : 'text-zinc-500'
      }`}
    >
      <span className="truncate">{title}</span>
      {running && (
        <span className="ml-auto size-[5px] shrink-0 rounded-full bg-emerald-400" />
      )}
    </div>
  )
}

// A stylized, hand-drawn impression of the Waku window — always graphite,
// independent of the page theme, because that's what the app looks like.
export function AppWindow() {
  return (
    <div className="overflow-hidden rounded-xl border border-black/10 bg-[#17181c] text-[11.5px] leading-normal text-zinc-300 shadow-[0_32px_90px_-28px_rgba(0,0,0,0.5)] ring-1 ring-white/10 select-none dark:border-white/10">
      {/* Title bar */}
      <div className="relative flex h-9 items-center gap-2 border-b border-white/6 bg-[#1d1e23] px-3.5">
        <div className="flex items-center gap-[7px]">
          <span className="size-[11px] rounded-full bg-[#ff5f57] shadow-[inset_0_0_0_0.5px_rgba(0,0,0,0.25)]" />
          <span className="size-[11px] rounded-full bg-[#febc2e] shadow-[inset_0_0_0_0.5px_rgba(0,0,0,0.25)]" />
          <span className="size-[11px] rounded-full bg-[#28c840] shadow-[inset_0_0_0_0.5px_rgba(0,0,0,0.25)]" />
        </div>
        <span className="absolute left-1/2 hidden max-w-[55%] -translate-x-1/2 truncate text-[11px] text-zinc-500 sm:block">
          waku — fix dropped frames in the transcript list
        </span>
        <span className="ml-auto hidden items-center gap-1.5 font-mono text-[9.5px] text-zinc-500 sm:flex">
          <span
            className="provider-mark size-3 opacity-70"
            style={{
              maskImage: 'url(/providers/claude.svg)',
              WebkitMaskImage: 'url(/providers/claude.svg)',
            }}
          />
          Claude Code
        </span>
      </div>

      <div className="flex h-[430px]">
        {/* Sidebar */}
        <div className="hidden w-48 shrink-0 flex-col gap-0.5 border-r border-white/6 bg-[#191a1e] p-2 md:flex">
          <div className="px-2 pt-1 pb-1.5 text-[9px] tracking-[0.12em] text-zinc-600 uppercase">
            Projects
          </div>
          <div className="flex items-center gap-1.5 px-2 py-[5px] text-zinc-300">
            <ChevronDown className="size-3 text-zinc-600" />
            <Folder className="size-3 text-zinc-500" />
            <span>waku</span>
          </div>
          <div className="flex flex-col gap-px pl-4">
            <SessionRow title="fix dropped frames" active running />
            <SessionRow title="sparkle updater pipeline" />
            <SessionRow title="model traits menu polish" />
          </div>
          <div className="mt-1 flex items-center gap-1.5 px-2 py-[5px] text-zinc-500">
            <ChevronRight className="size-3 text-zinc-600" />
            <Folder className="size-3 text-zinc-600" />
            <span>dotfiles</span>
          </div>
        </div>

        {/* Transcript */}
        <div className="flex min-w-0 flex-1 flex-col bg-[#1b1c21]">
          <div className="flex-1 space-y-3 overflow-hidden p-4">
            <div className="ml-auto w-fit max-w-[85%] rounded-lg rounded-br-sm bg-white/7 px-3 py-2 text-zinc-200">
              The transcript list drops frames on long sessions. Find it and
              fix it.
            </div>

            <p className="max-w-[92%] leading-relaxed text-zinc-400">
              The row builder asks git for status on every visible row, every
              frame. Moving it to a background pass and caching the result on
              the entity:
            </p>

            <div className="overflow-hidden rounded-md border border-white/7 bg-black/20">
              <div className="flex items-center gap-2 px-2.5 py-1.5">
                <Pencil className="size-3 text-zinc-500" />
                <span className="font-mono text-[10.5px] text-zinc-300">
                  Edit
                </span>
                <span className="truncate font-mono text-[10.5px] text-zinc-500">
                  src/transcript/list.rs
                </span>
                <span className="ml-auto shrink-0 font-mono text-[10px]">
                  <span className="text-emerald-400/90">+18</span>{' '}
                  <span className="text-red-400/80">−6</span>
                </span>
              </div>
              <div className="border-t border-white/6 px-3 py-2 font-mono text-[10.5px] leading-[1.7]">
                <div className="text-red-300/70">
                  <span className="mr-2 select-none">-</span>
                  {'let status = repo.git_status(&row.path);'}
                </div>
                <div className="text-emerald-300/80">
                  <span className="mr-2 select-none">+</span>
                  {'let status = self.status_cache.get(&row.path);'}
                </div>
                <div className="text-emerald-300/80">
                  <span className="mr-2 select-none">+</span>
                  cx.background_executor().spawn(refresh(paths));
                </div>
              </div>
            </div>

            <div className="overflow-hidden rounded-md border border-white/7 bg-black/20">
              <div className="flex items-center gap-2 px-2.5 py-1.5">
                <Terminal className="size-3 text-zinc-500" />
                <span className="font-mono text-[10.5px] text-zinc-300">
                  Bash
                </span>
                <span className="truncate font-mono text-[10.5px] text-zinc-500">
                  cargo test -p waku
                </span>
                <CircleCheck className="ml-auto size-3 shrink-0 text-emerald-400/90" />
              </div>
              <div className="border-t border-white/6 px-3 py-1.5 font-mono text-[10.5px] text-zinc-500">
                test result: ok. 94 passed; 0 failed
              </div>
            </div>

            <div className="flex items-center gap-1.5 pt-0.5 text-[10.5px] text-zinc-600">
              <History className="size-3" />
              Checkpoint saved — rewind restores code and conversation
            </div>
          </div>

          {/* Composer */}
          <div className="m-3 mt-0 rounded-lg border border-white/8 bg-[#1f2026] px-3 pt-2.5 pb-2">
            <div className="text-zinc-600">
              Reply — ⏎ queues while the agent works, ⌘⏎ steers mid-turn
            </div>
            <div className="mt-2.5 flex items-center gap-1.5">
              <Chip>
                <span
                  className="provider-mark size-2.5"
                  style={{
                    maskImage: 'url(/providers/claude.svg)',
                    WebkitMaskImage: 'url(/providers/claude.svg)',
                  }}
                />
                Fable 5
              </Chip>
              <Chip>Build</Chip>
              <Chip>Full access</Chip>
              <span className="ml-auto flex size-5 items-center justify-center rounded-md bg-zinc-200 text-zinc-900">
                <ArrowUp className="size-3" />
              </span>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
