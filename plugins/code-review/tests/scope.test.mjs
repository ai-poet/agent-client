import assert from 'node:assert/strict';
import test from 'node:test';
import {file,fixture,receipt} from './support.mjs';

const skipped = ['.env','.npmrc','.netrc','.pypirc','.ssh/id_rsa','credentials/token.json','secrets/api.json',
  'cert/server.key','node_modules/lib/main.js','target/debug/app.rs','generated/types.ts','src/schema.pb.go',
  'assets/app.min.js','Cargo.lock'];

test('initial page limit preserves complete frozen coverage', async () => {
  const source = [file('src/first.rs'),file('src/second.rs'),file('src/third.rs')];
  const fixtureData = await fixture(source);
  const scope = await fixtureData.capture({limit:1});
  assert.equal(scope.counts.files,3);
  assert.equal(scope.files.length,1);
  assert.equal(scope.next,1);
  assert.equal(fixtureData.calls.filter(call => call.kind === 'read_git_change').length,3);
  assert.equal((await fixtureData.invoke('complete',receipt(scope))).isError,true);
  const complete = {...scope,files:[...scope.files],groups:[],skipped:[],findings:[]};
  let next = scope.next;
  while (next !== null) {
    const page = await fixtureData.invoke('scope',{review_id:scope.review_id,offset:next,limit:1});
    assert.deepEqual(page.snapshot,scope.snapshot);
    for (const kind of ['files','groups','skipped','findings']) complete[kind].push(...page[kind]);
    next = page.next;
  }
  assert.deepEqual(complete.files.map(value => value.path),source.map(value => value.path));
  assert.ok(complete.files.every(value => value.revision && value.metadata_revision));
  assert.equal((await fixtureData.invoke('complete',receipt(complete))).status,'complete');
});

test('creation rejects invalid pagination before reading resources', async () => {
  for (const input of [{limit:0},{limit:Number.MAX_SAFE_INTEGER+1},{limit:null},{offset:0,limit:1}]) {
    const fixtureData = await fixture();
    const result = await fixtureData.invoke('scope',input);
    assert.equal(result.isError,true);
    assert.equal([...fixtureData.values.values()].filter(value => value.present).length,0);
    assert.equal(fixtureData.calls.length,0);
  }
});

test('inventory finishes before eligible reads', async () => {
  const source = Array.from({length:15},(_,index) => file(`src/file-${index}.rs`));
  const fixtureData = await fixture([...source,...skipped.map(path => file(path)),
    ...Array.from({length:550},(_,index) => file(`node_modules/pkg-${index}/index.js`))]);
  const scope = await fixtureData.capture();
  assert.equal(scope.counts.files,15);
  assert.equal(scope.counts.skipped,skipped.length+550);
  assert.deepEqual(scope.groups.map(group => group.files.length),[10,5]);
  const gitCalls=fixtureData.calls.filter(call => call.kind !== 'plugin_transaction');
  const first = gitCalls.findIndex(call => call.kind === 'read_git_change');
  assert.equal(first,3);
  assert.ok(gitCalls.slice(0,first).every(call => call.kind === 'list_git_changes'));
  const reads = fixtureData.calls.filter(call => call.kind === 'read_git_change').map(call => call.data.path);
  assert.deepEqual(new Set(reads),new Set(source.map(file => file.path)));
  const visible = [...scope.skipped];
  let next = scope.next;
  while (next !== null) {
    const page = await fixtureData.invoke('scope',{review_id:scope.review_id,offset:next,limit:100});
    visible.push(...page.skipped);
    next = page.next;
  }
  assert.equal(visible.length,scope.counts.skipped);
  assert.ok(visible.some(item => item.path === '.npmrc' && item.reason === 'credentials'));
});

test('patch reads distinguish inventory and file revisions', async () => {
  const fixtureData = await fixture([file('inventory/stock.py'),file('tests/test_stock.py')]);
  const scope = await fixtureData.capture();
  const git = await fixtureData.module('git');
  const selected = scope.files[0];
  assert.notEqual(scope.snapshot.metadata_revision,selected.metadata_revision);
  await assert.rejects(git.change(fixtureData.captured.worktree,scope.snapshot,
    {...selected,metadata_revision:scope.snapshot.metadata_revision}), {code:'revision_conflict'});
  for (const file of scope.files) {
    const page = await git.change(fixtureData.captured.worktree,scope.snapshot,file);
    assert.deepEqual(JSON.parse(JSON.stringify(page.snapshot)),scope.snapshot);
    assert.equal(page.metadata_revision,file.metadata_revision);
    assert.equal(page.revision,file.revision);
  }
  const reads = fixtureData.calls.filter(call => call.kind === 'read_git_change').slice(-scope.files.length);
  assert.deepEqual(reads.map(call => call.data),scope.files.map(file => ({
    worktree:fixtureData.captured.worktree,snapshot:scope.snapshot,path:file.path,
    expected_metadata_revision:file.metadata_revision,expected_revision:file.revision,offset:0,limit:500,
  })));
});

test('tests stay eligible and related files share groups', async () => {
  const fixtureData = await fixture(['src/math.c','include/math.h','tests/math_test.c',
    'other/src/main.rs','other/tests/main_test.rs','locales/en.json','locales/zh.json',
    'pkg-one/src/main.js','pkg-two/src/main.js','resources/en/messages.properties','resources/fr/messages.properties'].map(path => file(path)));
  const scope = await fixtureData.capture();
  assert.equal(scope.counts.files,11);
  const group = path => scope.files.find(file => file.path === path).group;
  assert.equal(group('src/math.c'),group('tests/math_test.c'));
  assert.equal(group('src/math.c'),group('include/math.h'));
  assert.equal(group('other/src/main.rs'),group('other/tests/main_test.rs'));
  assert.equal(group('locales/en.json'),group('locales/zh.json'));
  assert.notEqual(group('pkg-one/src/main.js'),group('pkg-two/src/main.js'));
  assert.equal(group('resources/en/messages.properties'),group('resources/fr/messages.properties'));
});

test('rules use order case and Objective-C prefixes', async () => {
  const fixtureData = await fixture([file('Cargo.TOML'),file('.github/workflows/build.yml'),file('src/unknown.xyz'),
    file('src/object.M',{first_line:'  #import <Foundation/Foundation.h>'}),file('src/value.m',{first_line:'function value = example()'})]);
  const scope = await fixtureData.capture();
  const rule = path => scope.files.find(file => file.path === path).rule;
  assert.equal(rule('Cargo.TOML').file,'cargo_toml.md');
  assert.equal(rule('.github/workflows/build.yml').file,'github_workflows.md');
  assert.equal(rule('src/unknown.xyz').file,'default.md');
  assert.equal(rule('src/object.M').file,'objc.md');
  assert.equal(rule('src/value.m').file,'matlab.md');
  for (const file of scope.files) assert.match(file.rule.sha256,/^[0-9a-f]{64}$/);
});

test('binary and unsupported files return skip receipts', async () => {
  const fixtureData = await fixture([file('src/image.bin',{binary:true}),file('src/socket',{skip_reason:'unsupported_mode'}),file('tests/main.rs')]);
  const scope = await fixtureData.capture();
  assert.equal(scope.counts.files,1);
  assert.deepEqual(scope.skipped.map(file => file.reason),['binary file','unsupported_mode']);
  assert.equal((await fixtureData.invoke('complete',receipt(scope))).status,'complete');
});

test('incomplete inventory never persists', async () => {
  const fixtureData = await fixture([file('src/main.rs')],{incomplete:true});
  const result = await fixtureData.invoke('scope',{});
  assert.equal(result.isError,true);
  assert.equal([...fixtureData.values.values()].filter(value => value.present).length,0);
  assert.equal(fixtureData.calls.filter(call => call.kind === 'read_git_change').length,0);
});

test('large scopes retain all files across bounded output pages', async () => {
  const source = Array.from({length:257},(_,index) => file(`src/${index}.rs`));
  const current = await fixture(source);
  const first = await current.capture({limit:1000});
  assert.equal(first.counts.files,source.length);
  assert.equal(first.files.length,100);
  const scope = {...first,files:[...first.files],groups:[...first.groups]};
  for (let next=first.next; next!==null;) {
    const page=await current.invoke('scope',{review_id:first.review_id,offset:next,limit:1000});
    assert.deepEqual(page.snapshot,first.snapshot);
    assert.ok(page.files.length+page.groups.length+page.skipped.length+page.findings.length<=100);
    scope.files.push(...page.files); scope.groups.push(...page.groups); next=page.next;
  }
  assert.deepEqual(new Set(scope.files.map(value=>value.path)),new Set(source.map(value=>value.path)));
  assert.equal((await current.invoke('complete',receipt(scope))).status,'complete');
});

test('creation failures never publish a scope', async () => {
  let declined=false;
  const fixtureData = await fixture(undefined,{command(command) {
    if (command.kind === 'plugin_transaction' && !declined) {
      declined=true;
      return {Err:{code:'permission_denied',message:'Declined'}};
    }
  }});
  const error = await fixtureData.invoke('scope',{});
  assert.equal(error.error.code,'permission_denied');
  assert.equal([...fixtureData.values.values()].filter(value => value.present).length,0);
});

test('transient Git reads retain capture progress', async () => {
  for (const kind of ['list_git_changes','read_git_change','read_git_context']) {
    let blocked=true;
    const current=await fixture([file('src/first.rs'),file('src/second.rs')],{
      command(command) {
        if (command.kind === kind && blocked) {
          blocked=false;
          return {Err:{code:'busy',message:'Git inspection queue is busy'}};
        }
      },
    });
    const failure=await current.invoke('scope',{});
    assert.equal(failure.error.code,'busy');
    const review_id=failure.remaining.review_id;
    const saved=current.values.get(`review/${review_id}/capture`);
    assert.equal(saved.present,true);
    assert.equal(saved.value.status,'capturing');
    const scope=await current.capture({review_id});
    assert.equal(scope.status,'open');
    assert.equal(scope.counts.files,2);
  }
});

test('continuation does not reread completed files', async () => {
  let blocked=true;
  const current=await fixture([file('src/first.rs'),file('src/second.rs')],{
    command(command) {
      if (command.kind === 'read_git_change' && command.data.path === 'src/second.rs' && blocked) {
        blocked=false;
        return {Err:{code:'expired',message:'Git read deadline exceeded'}};
      }
    },
  });
  const failure=await current.invoke('scope',{});
  assert.equal(failure.error.code,'expired');
  const scope=await current.capture({review_id:failure.remaining.review_id});
  assert.equal(scope.counts.files,2);
  assert.equal(current.calls.filter(call=>call.kind === 'read_git_change'
    && call.data.path === 'src/first.rs').length,1);
});

test('source test conventions retain project ownership', async () => {
  const fixtureData = await fixture(['java/src/main/java/example/Service.java',
    'java/src/test/java/example/ServiceTest.java','dotnet/src/Service.cs','dotnet/tests/ServiceTests.cs'].map(path => file(path)));
  const scope = await fixtureData.capture();
  const group = path => scope.files.find(file => file.path === path).group;
  assert.equal(group('java/src/main/java/example/Service.java'),group('java/src/test/java/example/ServiceTest.java'));
  assert.equal(group('dotnet/src/Service.cs'),group('dotnet/tests/ServiceTests.cs'));
  assert.notEqual(group('dotnet/src/Service.cs'),group('java/src/main/java/example/Service.java'));
});

test('oversized Unicode groups never persist', async () => {
  const source = Array.from({length:100},(_,index) => file(`app/locales/${index}-${'漢'.repeat(700)}.json`));
  const fixtureData = await fixture(source);
  const scope = await fixtureData.capture();
  assert.equal(scope.isError,true);
  assert.equal([...fixtureData.values.values()].filter(value => value.present).length,0);
});


test('encoded scope pages continue without losing files', async () => {
  const source = Array.from({length:100},(_,index) => file(`part-${index}/${'漢'.repeat(700)}.rs`));
  const fixtureData = await fixture(source);
  const scope = await fixtureData.capture();
  assert.equal(scope.counts.files,100);
  assert.ok(Buffer.byteLength(JSON.stringify(scope)) <= 128*1024);
  assert.ok(scope.next !== null);
  const paths = scope.files.map(file => file.path);
  let next = scope.next;
  while (next !== null) {
    const page = await fixtureData.invoke('scope',{review_id:scope.review_id,offset:next});
    assert.ok(Buffer.byteLength(JSON.stringify(page)) <= 128*1024);
    paths.push(...page.files.map(file => file.path));
    next = page.next;
  }
  assert.deepEqual(new Set(paths),new Set(source.map(file => file.path)));
});
