/** Output budgets count UTF-8 bytes of the serialized model result. */
export function invalid(message) {
  return {result:{isError:true,error:{code:'invalid_request',message}}};
}

function bytes(text) {
  let count = 0;
  for (const character of text) {
    const point = character.codePointAt(0);
    count += point <= 0x7f ? 1 : point <= 0x7ff ? 2 : point <= 0xffff ? 3 : 4;
  }
  return count;
}

export function size(value) { return bytes(JSON.stringify(value)); }

export function bound(value, fallback, maximum, minimum=1) {
  if (value === undefined) return fallback;
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new Error('Invalid code tool limit');
  }
  return value;
}

export function omitted(data) {
  data.partial = true;
  data.truncated = true;
}

/** Drops trailing list items until the result fits its budget. */
export function fit(data, key, budget, offset) {
  while (size(data) > budget && data[key].length) {
    data[key].pop();
    omitted(data);
    if (offset !== undefined) data.next_offset = offset + Math.max(1,data[key].length);
  }
  if (size(data) > budget) throw new Error('Code result metadata exceeds the output budget');
  return data;
}
