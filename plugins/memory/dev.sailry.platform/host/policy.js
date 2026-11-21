// Curated policy adapted from Sailry's original Memory package.
// ZCode 872ad960 informed its curated categories. The fixed policy text is the manifest's
// `instructions`; per-turn output is only matched bodies and a save cue.
export function bytes(text) {
  let size = 0;
  for (const character of text) {
    const code = character.codePointAt(0);
    size += code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
  }
  return size;
}
export const content = text => ({role:"user",parts:[{text}]});
const cost = text => bytes(JSON.stringify(content(text))) + 128;

export const relevantLimit = 3;
// Lasting-preference, correction, decision and remember cues in Chinese and English. The Chinese
// terms are escaped: don't, no more, don't use, later, from now on, next time, remember (twice),
// always, forever, no longer, preference, like, habit, decide, switch to, forbid.
const cues = [
  /\u4e0d\u8981|\u522b\u518d|\u522b\u7528|\u4ee5\u540e|\u4eca\u540e|\u4e0b\u6b21|\u8bb0\u4f4f|\u8bb0\u5f97|\u603b\u662f|\u6c38\u8fdc|\u4e0d\u518d|\u504f\u597d|\u559c\u6b22|\u4e60\u60ef|\u51b3\u5b9a|\u6539\u7528|\u7981\u6b62/u,
  /\b(?:always|never|prefer(?:s|red|ence|ences)?|remember|don['\u2019]?t|do not|from now on|going forward|next time|instead of|stop (?:using|doing)|decided|i (?:like|want you to))\b/i,
];
export const cued = message => typeof message === 'string' && cues.some(pattern=>pattern.test(message));
export const cue = 'If this states a lasting preference or correction, save it with save_memory.';

// Each relevant body is one context part. The leading parts that fit the limit are kept, up to three.
export function relevantParts(relevant,limit) {
  const parts = [];
  let size = 0;
  for (const {summary,body} of relevant) {
    if (parts.length === relevantLimit) break;
    const part = `Relevant memory: ${JSON.stringify({id:summary.id,title:summary.title,kind:summary.kind,global:summary.project === null,body})}`;
    const added = bytes(part);
    if (size+added > limit) continue;
    parts.push(part); size += added;
  }
  return parts;
}

// Turn parts: matched bodies, then the save cue, within half the context budget together.
// A turn without either adds no context.
export function turnParts({relevant=[],message,settings,write}) {
  const save = write && cued(message);
  const parts = relevantParts(relevant,Math.floor(settings.context_bytes / 2) - (save ? bytes(cue) : 0));
  if (save) parts.push(cue);
  return parts;
}

export function values(entries, state) {
  return entries.filter(entry => {
    const size = cost(JSON.stringify(entry));
    if (size > state.remaining) return false;
    state.remaining -= size;
    return true;
  });
}

export function retrieve(entries, state) {
  const results = [];
  for (const entry of entries) {
    const characters = Array.from(entry.body);
    const encode = end => JSON.stringify({body:characters.slice(0,end).join(""),body_truncated:end < characters.length,summary:entry.summary});
    let text = encode(characters.length);
    if (cost(text) > state.remaining) {
      let low = 0, high = characters.length + 1;
      while (low < high) {
        const middle = Math.floor((low + high) / 2);
        if (cost(encode(middle)) <= state.remaining) low = middle + 1;
        else high = middle;
      }
      if (low <= 1) continue;
      text = encode(low - 1);
    }
    const size = cost(text);
    if (size <= state.remaining) {
      state.remaining -= size;
      results.push(content(text));
    }
  }
  return results;
}

export function reads(results) {
  const visible = [];
  for (const result of results) {
    const response=result.response?.context ?? result.response;
    const entries=result.name==='search_memory' ? response?.entries
      : result.name==='save_memory' && response?.status==='similar' && response.saved===false ? response.existing
      : ['save_memory','consolidate_memories'].includes(result.name) && response?.status==='changed' && response.saved===false ? response.latest : null;
    if (!Array.isArray(entries)) continue;
    for (const entry of entries) {
      for (const part of entry?.parts ?? []) {
        if (typeof part.text !== "string") continue;
        try {
          const value = JSON.parse(part.text);
          if (value.body_truncated === false && typeof value.body === "string" && value.body.length > 0 && typeof value.summary?.id === "string" && Number.isSafeInteger(value.summary.revision)) {
            visible.push([value.summary.id,value.summary.revision]);
          }
        } catch { /* Non-memory text carries no replacement authority. */ }
      }
    }
  }
  return visible;
}

export function requireRead(state, id, revision) {
  if (!(state.reads ?? []).some(([entry,current]) => entry === id && current === revision)) {
    fail("call search_memory with this ID to read its complete current body, then retry with its revision; titles and truncated bodies are not enough");
  }
}
export function fail(message, code = "invalid_request") {
  const error = new Error(message);
  error.code = code;
  throw error;
}
export function argumentsFor(value, allowed, required) {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !allowed.includes(key)) || required.some(key => !(key in value))) fail("invalid memory arguments");
  for (const key of ["title","body","query"]) if (key in value && typeof value[key] !== "string") fail("invalid memory arguments");
  for (const key of ["global","archived"]) if (key in value && typeof value[key] !== "boolean") fail("invalid memory arguments");
  if ("kind" in value && !["user","feedback","project","reference"].includes(value.kind)) fail("invalid memory arguments");
  if ("expected_revision" in value && (!Number.isSafeInteger(value.expected_revision) || value.expected_revision < 0)) fail("invalid memory arguments");
  if (value.id != null && (typeof value.id !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value.id))) fail("invalid memory arguments");
  return value;
}
