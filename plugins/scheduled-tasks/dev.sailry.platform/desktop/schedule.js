// Pure schedule and history helpers for the task details panel.
// The script runtime has no Intl time zone support, so weekly occurrences derive the
// task time zone offset from the Node-computed next run instead of an IANA lookup.
export const MINUTE = 60000, HOUR = 3600000, DAY = 86400000;

const pad = value => String(value).padStart(2, "0");
const iso = day => day === 0 ? 7 : day;
function startOfDay(ms) { const date = new Date(ms); date.setHours(0, 0, 0, 0); return date.getTime(); }

export function clock(ms, seconds = false) {
  const date = new Date(ms);
  if (!Number.isFinite(date.getTime())) return "—";
  return `${pad(date.getHours())}:${pad(date.getMinutes())}${seconds ? `:${pad(date.getSeconds())}` : ""}`;
}
export function wallClock(hour, minute) { return `${pad(hour)}:${pad(minute)}`; }
// Minutes east of UTC for the task zone at the given weekly occurrence.
export function weeklyOffset(data, next) {
  const date = new Date(next);
  const minutes = ((data.hour * 60 + data.minute - date.getUTCHours() * 60 - date.getUTCMinutes()) % 1440 + 1440) % 1440;
  return minutes > 840 ? minutes - 1440 : minutes;
}
export function zoneWeekday(ms, offset) { return iso(new Date(ms + offset * MINUTE).getUTCDay()); }
export function localWeekday(ms) { return iso(new Date(ms).getDay()); }

export function upcoming(item, now, count = 5) {
  if (!item.enabled) return [];
  const {kind, data} = item.timing, next = item.next_ms ?? null;
  if (kind === "once") return data.at_ms > now ? [data.at_ms] : [];
  if (kind === "every") {
    const first = next ?? (data.anchor_ms >= now ? data.anchor_ms
      : data.anchor_ms + Math.ceil((now - data.anchor_ms) / data.interval_ms) * data.interval_ms);
    return Array.from({length:count}, (_, index) => first + index * data.interval_ms);
  }
  if (next === null) return [];
  const base = zoneWeekday(next, weeklyOffset(data, next)), times = [];
  for (let day = 0; times.length < count && day < 7 * count + 7; day++) {
    if (data.weekdays.includes((base - 1 + day) % 7 + 1)) times.push(next + day * DAY);
  }
  return times;
}
// Fractions of today's local day for interval runs; null when ticks would be unreadable.
export function ribbon(data, now, limit = 96) {
  const start = startOfDay(now), end = start + DAY;
  let at = data.anchor_ms >= start ? data.anchor_ms
    : data.anchor_ms + Math.ceil((start - data.anchor_ms) / data.interval_ms) * data.interval_ms;
  const ticks = [];
  for (; at < end; at += data.interval_ms) {
    if (ticks.length === limit) return {ticks:null, now:(now - start) / DAY};
    ticks.push((at - start) / DAY);
  }
  return {ticks, now:(now - start) / DAY};
}
export function streak(runs) {
  let count = 0;
  for (const run of runs) { if (run.job.status !== "failed") break; count++; }
  return count;
}
export function tally(runs) {
  const counts = {completed:0, failed:0, cancelled:0, running:0, queued:0, unknown:0};
  for (const run of runs) counts[run.job.status] = (counts[run.job.status] ?? 0) + 1;
  return counts;
}

export function dayLabel(ms, now, text) {
  if (!Number.isFinite(new Date(ms).getTime())) return "—";
  const days = Math.round((startOfDay(ms) - startOfDay(now)) / DAY);
  if (days === 0) return text.today;
  if (days === -1) return text.yesterday;
  if (days === 1) return text.tomorrow;
  const date = new Date(ms);
  return `${text.monthDay(date.getMonth() + 1, date.getDate())} ${text[`weekday${localWeekday(ms)}`]}`;
}
export function when(ms, now, text) {
  if (!Number.isFinite(new Date(ms).getTime())) return "—";
  return `${dayLabel(ms, now, text)} ${clock(ms)}`;
}
export function relative(ms, now, text) {
  if (!Number.isFinite(new Date(ms).getTime())) return "—";
  const days = Math.round((startOfDay(ms) - startOfDay(now)) / DAY);
  if (days >= 1) return text.inDays(days);
  const minutes = Math.max(1, Math.ceil((ms - now) / MINUTE));
  return minutes < 60 ? text.inMinutes(minutes) : text.inHours(Math.floor(minutes / 60), minutes % 60);
}
export function interval(ms, text) {
  for (const [unit, size] of [["everyDays", DAY], ["everyHours", HOUR], ["everyMinutes", MINUTE]]) {
    if (ms % size === 0) return text[unit](ms / size);
  }
  return text.everySeconds(ms / 1000);
}
export function summary(item, now, text) {
  const {kind, data} = item.timing;
  if (kind === "every") return interval(data.interval_ms, text);
  if (kind === "once") return `${text.once} ${when(data.at_ms, now, text)}`;
  const days = data.weekdays.length === 7 ? text.daily
    : data.weekdays.join() === "1,2,3,4,5" ? text.workdays
    : data.weekdays.map(day => text[`weekday${day}`]).join(" ");
  return `${days} ${wallClock(data.hour, data.minute)}`;
}
