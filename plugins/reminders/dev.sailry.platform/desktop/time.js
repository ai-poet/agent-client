// Pure local-time helpers for reminder cards and the details panel.
export const MINUTE = 60000, HOUR = 3600000, DAY = 86400000;

const pad = value => String(value).padStart(2, "0");
function startOfDay(ms) { const date = new Date(ms); date.setHours(0, 0, 0, 0); return date.getTime(); }
export function weekday(ms) { const day = new Date(ms).getDay(); return day === 0 ? 7 : day; }

export function clock(ms, seconds = false) {
  const date = new Date(ms);
  return `${pad(date.getHours())}:${pad(date.getMinutes())}${seconds ? `:${pad(date.getSeconds())}` : ""}`;
}
export function dayLabel(ms, now, text) {
  const days = Math.round((startOfDay(ms) - startOfDay(now)) / DAY);
  if (days === 0) return text.today;
  if (days === -1) return text.yesterday;
  if (days === 1) return text.tomorrow;
  return date(ms, text);
}
export function date(ms, text) {
  const value = new Date(ms);
  return `${text.monthDay(value.getMonth() + 1, value.getDate())} ${text[`weekday${weekday(ms)}`]}`;
}
export function when(ms, now, text) { return `${dayLabel(ms, now, text)} ${clock(ms)}`; }
// Elapsed or remaining time between now and a due time, rounded to whole minutes.
export function relative(ms, now, text) {
  const past = ms <= now, minutes = Math.max(1, Math.round(Math.abs(ms - now) / MINUTE));
  const [inMinutes, inHours, inDays] = past ? [text.agoMinutes, text.agoHours, text.agoDays] : [text.inMinutes, text.inHours, text.inDays];
  if (minutes < 60) return inMinutes(minutes);
  if (minutes < 24 * 60) return inHours(Math.floor(minutes / 60), minutes % 60);
  return inDays(Math.floor(minutes / (24 * 60)));
}
