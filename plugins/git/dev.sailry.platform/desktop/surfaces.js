// The package owns captured documents; Kit owns their retained diff/editor state.
export class Surfaces {
  constructor(createDiff,createEditor) {
    this.createDiff = createDiff; this.createEditor = createEditor;
    this.states = new Map();
  }
  sync(documents) {
    const active = new Set();
    for (const document of documents.values()) {
      if (document.request.kind === 'output') {
        const id = `git-output-${document.id}`, text = document.text ?? '';
        active.add(id);
        const current = this.states.get(id);
        if (!current) this.states.set(id,{kind:'source',text,state:this.createEditor(text)});
        else if (current.text !== text) this.states.set(id,{kind:'source',text,state:this.createEditor(text)});
        continue;
      }
      const single = ['file','stash'].includes(document.request.kind);
      for (const file of document.files ?? []) {
        if (file.binary) continue;
        const id = `git-diff-${document.id}${single ? '' : `/${file.path}`}`;
        const text = patch(file);
        active.add(id);
        const current = this.states.get(id);
        if (!current) this.states.set(id,{kind:'diff',text,state:this.createDiff(text)});
        else if (current.text !== text) this.states.set(id,{kind:'diff',text,state:this.createDiff(text)});
      }
    }
    for (const id of this.states.keys()) if (!active.has(id)) this.states.delete(id);
  }
  get(id) { return this.states.get(id)?.state; }
  clear() { this.states.clear(); }
}

// Node can return a bounded final hunk. Its absent rows remain absent; advertised
// lengths are shortened only for explicitly truncated captures.
export function patch(file) {
  let text = file.text ?? '';
  if (file.truncated) {
    const lines = text.split('\n');
    if (lines.at(-1) === '') lines.pop();
    let index = lines.length - 1;
    while (index >= 0 && !lines[index].startsWith('@@ ')) index--;
    const header = index >= 0 ? /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/.exec(lines[index]) : null;
    if (header) {
      const tail = lines.slice(index + 1);
      const old = tail.filter(line => line.startsWith(' ') || line.startsWith('-')).length;
      const next = tail.filter(line => line.startsWith(' ') || line.startsWith('+')).length;
      if (tail.every(line => /^[ +\-\\]/.test(line)) && old <= Number(header[2] ?? 1) && next <= Number(header[4] ?? 1)
        && (old < Number(header[2] ?? 1) || next < Number(header[4] ?? 1))) {
        lines[index] = `@@ -${header[1]},${old} +${header[3]},${next} @@${header[5]}`;
        text = lines.join('\n');
      }
    }
  }
  if (!text || /^diff --git |^--- /m.test(text)) return text;
  return `--- ${JSON.stringify(`a/${file.path}`)}\n+++ ${JSON.stringify(`b/${file.path}`)}\n${text}`;
}
