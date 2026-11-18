const dependencies = new Set(['node_modules','vendor','third_party','third-party','.venv','venv','pods','.gradle']);
const builds = new Set(['target','dist','build','.next','.nuxt','coverage','.dart_tool','out','bin','obj']);
const generated = new Set(['generated','__generated__']);
const protectedDirs = new Set(['.ssh','.gnupg','.aws','.azure','.kube','.docker','secrets','credentials']);
const credentialFiles = new Set(['.npmrc','.netrc','.pypirc','.git-credentials','auth.json']);
const locks = new Set(['cargo.lock','package-lock.json','npm-shrinkwrap.json','yarn.lock','pnpm-lock.yaml',
  'bun.lock','bun.lockb','composer.lock','poetry.lock','uv.lock','pipfile.lock','gemfile.lock',
  'pubspec.lock','podfile.lock','packages.lock.json']);

export function excluded(path) {
  const parts = path.toLowerCase().split('/'), name = parts.at(-1);
  if (parts.some(part => protectedDirs.has(part)) || credentialFiles.has(name) || /^\.env(?:\.|$)/.test(name)
      || /^(?:credentials|secrets)(?:\.|$)/.test(name)
      || /^id_(?:rsa|dsa|ecdsa|ed25519)(?:\.|$)/.test(name)
      || /^service[-_]account.*\.json$/.test(name)
      || /\.(?:pem|key|p12|pfx|keystore)$/.test(name)) return 'credentials';
  if (parts.some(part => dependencies.has(part))) return 'dependencies';
  if (parts.some(part => builds.has(part))) return 'build output';
  if (locks.has(name)) return 'lockfile';
  if (/\.min\.(?:js|css)$/.test(name) || /\.map$/.test(name)) return 'minified output';
  if (parts.some(part => generated.has(part))
      || /(?:\.g\.dart|\.freezed\.dart|\.generated\.[^.]+|\.gen\.[^.]+|\.pb\.(?:go|cc|h)|_pb2(?:_grpc)?\.py)$/.test(name)) {
    return 'generated output';
  }
  return null;
}
