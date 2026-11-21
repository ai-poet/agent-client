// Composer branch chip policy. A new conversation chooses where it starts; the
// bound checkout is never switched. A running conversation can fork elsewhere.

/** Contribution menus hold 256 rows; each branch list keeps well inside that. */
export const MENU_BRANCHES = 100;

function byteLength(value) {
  let bytes = 0;
  for (const character of value) {
    const code = character.codePointAt(0);
    bytes += code <= 0x7f ? 1 : code <= 0x7ff ? 2 : code <= 0xffff ? 3 : 4;
  }
  return bytes;
}

/** Shortens a dynamic name to the 128-byte contribution label limit. */
export function fit(value) {
  if (byteLength(value) <= 128) return value;
  let kept = '';
  for (const character of value) {
    if (byteLength(kept + character) > 125) break;
    kept += character;
  }
  return `${kept}…`;
}

// Choice ids carry the branch name and stay below the 512-byte id limit.
const usable = name => typeof name === 'string' && name.trim() !== '' && byteLength(name) <= 480
  && !/[\u0000-\u001f\u007f-\u009f]/u.test(name);

/** Branch name -> registered worktree id for checkouts other than the bound one. */
export function checkouts(catalog, entries, bound) {
  const found = new Map();
  for (const tree of catalog?.worktrees ?? []) {
    if (tree.id === bound) continue;
    const entry = (entries ?? []).find(entry => entry.path === tree.path && entry.available !== false);
    if (entry?.branch) found.set(entry.branch, tree.id);
  }
  return found;
}

export function label(location, text) {
  if (location.base) return fit(location.base.from);
  return location.branch ? fit(location.branch) : text.git_detached;
}

// The default branch leads each list; the rest keep their repository order.
const PRIMARY = ['main', 'master'];
const primaryFirst = list => [...list].sort((a, b) =>
  (PRIMARY.includes(b.name) ? 1 : 0) - (PRIMARY.includes(a.name) ? 1 : 0));

function bounded(list, keep) {
  const shown = list.slice(0, MENU_BRANCHES);
  for (const branch of list.slice(MENU_BRANCHES)) if (keep(branch)) shown.push(branch);
  return shown;
}

function row(prefix, branch, group, description) {
  return {id:`${prefix}:${branch.name}`, label:{label:fit(branch.name)}, group:{label:group},
    ...(description ? {description:{label:description}} : {})};
}

/** Menu rows and checked id for the composer branch contribution. */
export function branchMenu(location, branches, found, text, forkable) {
  const entries = (branches ?? []).filter(branch => usable(branch.name));
  const local = primaryFirst(entries.filter(branch => !branch.remote)), remote = entries.filter(branch => branch.remote);
  if (location.session) {
    const choices = location.branch
      ? [{id:'copy', label:{label:text.git_copy_branch}, icon:'copy', group:{label:text.git_branch_actions}}] : [];
    if (forkable && entries.length) choices.push({id:'fork', label:{label:text.git_fork_from_branch}, icon:'network',
      group:{label:text.git_branch_actions}, children:[
        ...bounded(local, () => false).map(branch => row('fork-local', branch, text.git_local_branches)),
        ...bounded(remote, () => false).map(branch => row('fork-remote', branch, text.git_remote_branches)),
      ]});
    return {choices, value:null};
  }
  const current = location.branch, from = location.base?.from;
  if (!branches && current && usable(current)) local.unshift({name:current, remote:false});
  const choices = bounded(local, branch => branch.name === current || branch.name === from).map(branch =>
    row('branch', branch, text.git_local_branches,
      branch.name !== current && found.has(branch.name) ? text.git_branch_worktree : null));
  if (remote.length) choices.push({id:'remote', label:{label:text.git_remote_branches_menu}, group:{label:text.git_remote_branches},
    children:bounded(remote, branch => branch.name === from).map(branch => row('remote', branch, text.git_remote_branches))});
  choices.push({id:'start', label:{label:text.git_start_new_branch}, icon:'plus', group:{label:text.git_branch_actions}});
  const selected = from === undefined ? `branch:${current}`
    : entries.some(branch => !branch.remote && branch.name === from) ? `branch:${from}` : `remote:${from}`;
  const listed = choices.some(choice => choice.id === selected || choice.children?.some(child => child.id === selected));
  return {choices, value:listed ? selected : null};
}

/** What a chosen row does; null when the row no longer matches the repository. */
export function resolve(id, location, branches, found, newId) {
  if (id === 'start' && !location.session) return {kind:'start'};
  if (id === 'copy' && location.session && location.branch) return {kind:'copy', text:location.branch};
  const split = id.indexOf(':'), prefix = id.slice(0, split), name = id.slice(split + 1);
  const branch = (branches ?? []).find(entry => entry.name === name
    && entry.remote === ['remote','fork-remote'].includes(prefix));
  if (['fork-local','fork-remote'].includes(prefix))
    return location.session && branch?.commit ? {kind:'fork', base:{from:branch.name, commit:branch.commit}} : null;
  if (location.session || !['branch','remote'].includes(prefix)) return null;
  if (prefix === 'branch' && name === location.branch) return {kind:'stay'};
  if (prefix === 'branch' && found.has(name)) return {kind:'worktree', worktree:found.get(name)};
  if (!branch?.commit) return null;
  return {kind:'base', base:{from:branch.name, branch:`sailry/${newId().slice(0,8)}`, commit:branch.commit}};
}
