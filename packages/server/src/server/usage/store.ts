import { appendFile, mkdir, readFile, readdir, rm } from "node:fs/promises";
import path from "node:path";

import type { Logger } from "pino";

import { UsageRecordSchema, type UsageRecord } from "./types.js";

export const USAGE_RETENTION_DAYS = 90;
const MONTH_FILE_PATTERN = /^(\d{4})-(\d{2})\.jsonl$/;

/**
 * Append-only usage log, sharded by month so a date-range query is a filename filter plus
 * a linear scan. There is no database in the daemon; this mirrors the file-backed pattern
 * used by agent and schedule storage.
 */
export class UsageStore {
  private readonly directory: string;
  private readonly logger: Logger;
  private writeChain: Promise<void> = Promise.resolve();

  constructor(options: { paseoHome: string; logger: Logger }) {
    this.directory = path.join(options.paseoHome, "usage");
    this.logger = options.logger;
  }

  /**
   * Appends one record. Writes are serialized so concurrent turns cannot interleave
   * partial lines within a file.
   */
  async append(record: UsageRecord): Promise<void> {
    const write = this.writeChain.then(async () => {
      await mkdir(this.directory, { recursive: true });
      const file = path.join(this.directory, `${monthKey(record.ts)}.jsonl`);
      await appendFile(file, `${JSON.stringify(record)}\n`, "utf8");
    });
    // Keep the chain alive even if this write fails, so one bad record cannot wedge the log.
    this.writeChain = write.catch(() => undefined);
    await write;
  }

  /** Reads every record whose timestamp falls within [from, to]. */
  async query(options: { from: Date; to: Date }): Promise<UsageRecord[]> {
    const months = monthKeysBetween(options.from, options.to);
    const records: UsageRecord[] = [];

    for (const month of months) {
      const file = path.join(this.directory, `${month}.jsonl`);
      let contents: string;
      try {
        contents = await readFile(file, "utf8");
      } catch {
        continue;
      }
      for (const line of contents.split("\n")) {
        if (!line.trim()) {
          continue;
        }
        const record = parseRecord(line);
        if (!record) {
          continue;
        }
        const at = new Date(record.ts).getTime();
        if (Number.isNaN(at) || at < options.from.getTime() || at > options.to.getTime()) {
          continue;
        }
        records.push(record);
      }
    }

    return records;
  }

  /** Drops month files that fall entirely outside the retention window. */
  async prune(now: Date = new Date()): Promise<void> {
    let files: string[];
    try {
      files = await readdir(this.directory);
    } catch {
      return;
    }

    const cutoff = new Date(now.getTime() - USAGE_RETENTION_DAYS * 24 * 60 * 60 * 1000);
    const cutoffMonth = `${cutoff.getUTCFullYear()}-${pad(cutoff.getUTCMonth() + 1)}`;

    for (const file of files) {
      const match = MONTH_FILE_PATTERN.exec(file);
      if (!match) {
        continue;
      }
      const month = `${match[1]}-${match[2]}`;
      // A month is only removable once its last day is older than the cutoff, so compare
      // the whole month key rather than a single date inside it.
      if (month < cutoffMonth) {
        try {
          await rm(path.join(this.directory, file));
        } catch (error) {
          this.logger.debug({ err: error, file }, "Failed to prune usage log");
        }
      }
    }
  }
}

function parseRecord(line: string): UsageRecord | null {
  try {
    const parsed = UsageRecordSchema.safeParse(JSON.parse(line));
    return parsed.success ? parsed.data : null;
  } catch {
    return null;
  }
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

export function monthKey(iso: string): string {
  const date = new Date(iso);
  const usable = Number.isNaN(date.getTime()) ? new Date() : date;
  return `${usable.getUTCFullYear()}-${pad(usable.getUTCMonth() + 1)}`;
}

export function monthKeysBetween(from: Date, to: Date): string[] {
  const keys: string[] = [];
  const cursor = new Date(Date.UTC(from.getUTCFullYear(), from.getUTCMonth(), 1));
  const last = Date.UTC(to.getUTCFullYear(), to.getUTCMonth(), 1);

  while (cursor.getTime() <= last) {
    keys.push(`${cursor.getUTCFullYear()}-${pad(cursor.getUTCMonth() + 1)}`);
    cursor.setUTCMonth(cursor.getUTCMonth() + 1);
  }

  return keys;
}
