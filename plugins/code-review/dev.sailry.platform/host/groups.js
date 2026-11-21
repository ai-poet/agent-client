function directory(path) { return path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : ''; }
function counterpart(path) {
  const raw = path.split('/');
  const name = raw.pop().replace(/\.[^.]+$/, '').replace(/(?:Test|Tests)$/, '')
    .replace(/(?:\.test|\.spec|_test|_tests|_spec)$/i, '').replace(/^test_/i, '').toLowerCase();
  const parts = raw.map(part => part.toLowerCase()).filter((part,index,all) =>
    !['src','source','include','includes','headers','test','tests','__tests__','spec','specs'].includes(part)
    && !(part === 'main' && ['src','source'].includes(all[index-1])));
  return [...parts,name].join('/');
}
function locale(path) {
  const parts = path.toLowerCase().split('/');
  const position = parts.findIndex(part => ['locales','locale','i18n','l10n'].includes(part));
  if (position < 0) return null;
  return parts.slice(0, position + 1).join('/');
}
function resource(path) {
  const parts = path.toLowerCase().split('/');
  const position = parts.findIndex(part => ['resources','res','assets','translations','intl'].includes(part));
  if (position < 0 || !/\.(?:properties|json|ya?ml|arb|po|pot|strings|resx|xml)$/.test(parts.at(-1))) return null;
  const language = /^[a-z]{2}(?:[-_][a-z]{2,4}){0,2}$/;
  return parts.filter((part,index) => index <= position || !language.test(part)).join('/');
}

// Directory peers join a group only while it stays within this many files, so
// large directories remain delegable in parallel. Counterparts and locale sets
// are always kept together, even when they exceed the cap.
export const peers = 10;

export function groups(files) {
  const parent = files.map((_, index) => index);
  const count = files.map(() => 1);
  const find = index => parent[index] === index ? index : (parent[index] = find(parent[index]));
  const union = (left, right) => {
    const target = find(left), source = find(right);
    if (target === source) return;
    parent[source] = target;
    count[target] += count[source];
  };
  const ownership = new Map();
  for (let index = 0; index < files.length; index++) {
    const file = files[index], keys = [`counterpart:${counterpart(file.path)}`];
    const translations = locale(file.path);
    if (translations) keys.push(`locale:${translations}`);
    const localized = resource(file.path);
    if (localized) keys.push(`resource:${localized}`);
    for (const key of keys) {
      if (ownership.has(key)) union(ownership.get(key), index);
      else ownership.set(key, index);
    }
  }
  const open = new Map();
  for (let index = 0; index < files.length; index++) {
    const key = directory(files[index].path);
    const owner = open.get(key);
    if (owner !== undefined && (find(owner) === find(index) || count[find(owner)] + count[find(index)] <= peers)) {
      union(owner, index);
    } else {
      open.set(key, index);
    }
  }
  const components = new Map();
  files.forEach((file,index) => {
    const owner = find(index);
    if (!components.has(owner)) components.set(owner, []);
    components.get(owner).push(file.path);
  });
  return [...components.values()].map((paths,index) => ({id:`group-${index+1}`, files:paths}));
}
