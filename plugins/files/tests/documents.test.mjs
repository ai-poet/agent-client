import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';

const source = (await readFile(new URL('../dev.sailry.platform/desktop/documents.js',import.meta.url),'utf8'))
  .replace(/^import[\s\S]*?;\n/gm,'').replace(/^export /gm,'');
const plain = value => JSON.parse(JSON.stringify(value));
const document = (id, fields = {}) => ({id,path:`${id}.txt`,revision:'original',dirty:false,
  saving:false,uncertain:false,truncated:false,mode:'source',markdown:false,can_copy:true,
  can_edit:true,can_save:false,can_cut_paste:true,readonly:false,error:null,...fields});

function fixture(initial, afterClose = async () => {}, confirm = async () => false) {
  let state = initial;
  const calls = [],errors = [];
  const cx = {notify(){}};
  const api = {
    faultCode:message => {
      try { return JSON.parse(message).code; }
      catch (_) { return 'failed'; }
    },
    readDocuments:() => state,
    nextDocumentChange:async () => { throw new Error('closed'); },
    openDocument:async (path,options) => { calls.push(['open',path,options]); },
    documentAction:async (id,action) => { calls.push(['action',id,action]); },
    saveDocument:async id => { calls.push(['save',id]); },
    closeDocument:async (id,options) => {
      calls.push(['close',id,options]);
      const value = state.documents.find(document => document.id === id);
      if (!options?.discard && (value.dirty || value.saving || value.uncertain) &&
          (!options?.confirm || !await confirm(id))) return false;
      state = {...state,cursor:String(BigInt(state.cursor) + 1n),documents:state.documents.filter(document => document.id !== id)};
      await afterClose();
      return true;
    },
  };
  const Documents = vm.runInNewContext(`${source}\nDocuments`,api);
  const documents = new Documents((...error) => errors.push(error));
  return {documents,calls,errors,cx,api,set:value => {state = value;}};
}

test('tab order and selection survive snapshots and exact decimal cursor ordering', () => {
  const fixture = fixtureState('9',[document('first'),document('second')]);
  const {documents,cx} = fixture;
  assert.deepEqual(plain(documents.order),['first','second']);
  assert.equal(documents.selected,'first');
  assert.equal(documents.accept({cursor:'10',documents:[document('second'),document('first'),document('third')],
    reveal:{document:'third',sequence:'1'}},cx),true);
  assert.deepEqual(plain(documents.order),['first','second','third']);
  assert.equal(documents.selected,'third');
  assert.equal(documents.accept({cursor:'9',documents:[],reveal:null},cx),false);
  assert.equal(documents.selected,'third');
});

function fixtureState(cursor,documents,confirm) { return fixture({cursor,documents,reveal:null},undefined,confirm); }

test('managed file documents retain ordinary editing and save actions',async () => {
  const path = '.sailry/worktrees/id/source.txt';
  const {documents,calls,cx} = fixtureState('1',[document('managed',{path,dirty:true,can_save:true})]);
  await documents.open(path,undefined,cx);
  await documents.action('undo',cx);
  await documents.save(cx);
  assert.deepEqual(plain(calls),[['open',path,{}],['action','managed',{kind:'undo'}],['save','managed']]);
  assert.equal(documents.current().readonly,false);
  assert.equal(documents.current().path,path);
});

test('canceled opens preserve the selected successor and its draft', async () => {
  const retained = document('draft',{dirty:true,can_save:true});
  const {documents,cx,api,errors} = fixtureState('1',[document('closing'),retained]);
  let reject;
  api.openDocument = () => new Promise((_,failed) => { reject = failed; });
  const opening = documents.open('closing.txt',undefined,cx);
  await documents.close('closing',cx);
  reject(new Error(JSON.stringify({code:'cancelled',message:'Document read was invalidated'})));
  await opening;
  assert.equal(documents.current(),retained);
  assert.equal(documents.current().dirty,true);
  assert.deepEqual(plain(documents.order),['draft']);
  assert.deepEqual(errors,[]);
});

test('open failures retain their original errors', async () => {
  const messages = ['cancelled',...['unavailable','invalid_request','outcome_unknown'].map(code =>
    JSON.stringify({code,message:'Document read failed'}))];
  for (const message of messages) {
    const retained = document('draft',{dirty:true,can_save:true});
    const {documents,cx,api} = fixtureState('1',[retained]);
    const failure = new Error(message);
    api.openDocument = async () => { throw failure; };
    await assert.rejects(documents.open('missing.txt',undefined,cx),error => error === failure);
    assert.equal(documents.current(),retained);
    assert.equal(documents.current().dirty,true);
  }
});

test('reveal selects existing documents and waits for an admitted loading document', async () => {
  const {documents,cx,calls} = fixtureState('1',[document('one'),document('two',{dirty:true})]);
  await documents.select('two',cx);
  assert.equal(documents.selected,'two');
  assert.deepEqual(plain(calls[0]),['action','two',{kind:'focus'}]);
  documents.accept({cursor:'2',documents:documents.state.documents,reveal:{document:'loading',sequence:'3'}},cx);
  assert.equal(documents.selected,'two');
  documents.accept({cursor:'3',documents:[...documents.state.documents,document('loading')],
    reveal:{document:'loading',sequence:'3'}},cx);
  assert.equal(documents.selected,'loading');
  documents.accept({cursor:'4',documents:documents.state.documents,reveal:{document:'two',sequence:'4'}},cx);
  assert.equal(documents.selected,'two');
  assert.equal(documents.current().dirty,true);
});

test('native confirmation cancellation preserves the draft and selection', async () => {
  const {documents,cx,calls} = fixtureState('1',[document('dirty',{dirty:true}),document('other')]);
  await documents.close('dirty',cx);
  assert.equal(documents.items().length,2);
  assert.equal(documents.selected,'dirty');
  assert.equal(documents.current().dirty,true);
  assert.deepEqual(plain(calls),[['close','dirty',{confirm:true}]]);
});

test('native confirmation closes the captured document after selection changes', async () => {
  let answer;
  const pending = new Promise(resolve => { answer = resolve; });
  const {documents,cx,calls} = fixtureState('1',[document('dirty',{dirty:true}),document('other')],() => pending);
  const closing = documents.close('dirty',cx);
  await documents.select('other',cx);
  answer(true);
  await closing;
  assert.deepEqual(plain(documents.order),['other']);
  assert.equal(documents.selected,'other');
  assert.deepEqual(plain(calls),[['close','dirty',{confirm:true}],['action','other',{kind:'focus'}]]);
});

test('closing the active tab focuses its selected successor for consecutive closes', async () => {
  const {documents,cx,calls} = fixtureState('1',[document('first'),document('second'),document('third')]);
  await documents.select('second',cx);
  calls.length = 0;
  await documents.close('second',cx);
  assert.equal(documents.selected,'third');
  assert.deepEqual(plain(calls),[['close','second',{confirm:true}],['action','third',{kind:'focus'}]]);
  calls.length = 0;
  await documents.close('third',cx);
  assert.equal(documents.selected,'first');
  assert.deepEqual(plain(calls),[['close','third',{confirm:true}],['action','first',{kind:'focus'}]]);
  calls.length = 0;
  await documents.close('first',cx);
  assert.equal(documents.selected,null);
  assert.deepEqual(plain(calls),[['close','first',{confirm:true}]]);
});

test('background and rejected closes preserve focus while confirmed active discard transfers it', async () => {
  let accept = false;
  const {documents,cx,calls} = fixtureState('1',[document('dirty',{dirty:true}),document('background'),document('next')],async () => accept);
  await documents.close('background',cx);
  assert.equal(documents.selected,'dirty');
  assert.equal(calls.some(call => call[0] === 'action'),false);
  await documents.close('dirty',cx);
  assert.equal(documents.selected,'dirty');
  assert.equal(calls.some(call => call[0] === 'action'),false);
  accept = true;
  await documents.close('dirty',cx);
  assert.equal(documents.selected,'next');
  assert.deepEqual(plain(calls.at(-1)),['action','next',{kind:'focus'}]);
});

test('early snapshots preserve close focus handoff without overriding newer selections', async () => {
  for (const select of [false,true]) {
    let complete;
    const pending = new Promise(resolve => { complete = resolve; });
    const {documents,cx,calls} = fixture({cursor:'1',documents:[document('first'),document('second'),document('third')],reveal:null},() => pending);
    await documents.select('third',cx);
    calls.length = 0;
    const closing = documents.close('third',cx);
    documents.refresh(cx);
    assert.equal(documents.selected,'second');
    if (select) await documents.select('first',cx);
    complete();
    await closing;
    assert.equal(documents.selected,select ? 'first' : 'second');
    assert.deepEqual(plain(calls.filter(call => call[0] === 'action')),
      [['action',select ? 'first' : 'second',{kind:'focus'}]]);
  }
});

test('save and editor commands use selected opaque handles and refresh authoritative state', async () => {
  const {documents,cx,calls,set} = fixtureState('1',[document('a'),document('b',{dirty:true,can_save:true})]);
  await documents.save(cx);
  assert.equal(calls.length,0);
  await documents.select('b',cx);
  set({cursor:'2',documents:[document('a'),document('b',{dirty:false,revision:'saved'})],reveal:null});
  await documents.save(cx);
  assert.deepEqual(plain(calls.at(-1)),['save','b']);
  assert.equal(documents.current().revision,'saved');
  await documents.action('find',cx);
  assert.deepEqual(plain(calls.at(-1)),['action','b',{kind:'find'}]);
  await documents.open('nested/资料.txt',42,cx);
  assert.deepEqual(plain(calls.at(-1)),['open','nested/资料.txt',{line:42}]);
});

test('uncertain outcomes preserve drafts and report each changed failure once', async () => {
  const error = {code:'outcome_unknown',message:'Publication is unconfirmed'};
  const state = {cursor:'1',documents:[document('a',{dirty:true,uncertain:true,error})],reveal:null};
  const {documents,cx,errors,calls} = fixture(state);
  assert.equal(errors.length,1);
  documents.accept({...state,cursor:'2'},cx);
  assert.equal(errors.length,1);
  await documents.save(cx);
  assert.equal(calls.length,0);
  await documents.close('a',cx);
  assert.equal(documents.current().dirty,true);
  assert.deepEqual(plain(calls),[['close','a',{confirm:true}]]);
  documents.accept({cursor:'3',documents:[document('a')],reveal:null},cx);
  documents.accept({...state,cursor:'4'},cx);
  assert.equal(errors.length,2);
});

test('native save failures report once across snapshot ordering', async () => {
  for (const code of ['revision_conflict','outcome_unknown','cancelled']) for (const early of [true,false]) {
    const current = document('draft',{dirty:true,can_save:true});
    const failure = {code,message:'Save was not confirmed'};
    const state = {cursor:'2',documents:[{...current,error:failure,uncertain:code === 'outcome_unknown'}],reveal:null};
    const {documents,cx,errors,api,set} = fixtureState('1',[current]);
    api.saveDocument = async () => {
      set(state);
      if (early) documents.accept(state,cx);
      throw new Error(JSON.stringify(failure));
    };
    await documents.save(cx);
    documents.accept(state,cx);
    assert.deepEqual(plain(errors),[[failure,current]]);
    assert.equal(documents.current().dirty,true);
    assert.equal(documents.current().uncertain,code === 'outcome_unknown');
  }
});

test('transport failures retain one save-specific fallback', async () => {
  for (const unreadable of [false,true]) {
    const current = document('draft',{dirty:true,can_save:true});
    const {documents,cx,errors,api} = fixtureState('1',[current]);
    api.saveDocument = async () => { throw new Error(JSON.stringify({code:'outcome_unknown'})); };
    if (unreadable) api.readDocuments = () => { throw new Error('view unavailable'); };
    await documents.save(cx);
    assert.deepEqual(plain(errors),[[{code:'outcome_unknown'},current]]);
    assert.equal(documents.current().dirty,true);
  }
});

test('explicit save retries each report their own failure', async () => {
  const current = document('draft',{dirty:true,can_save:true});
  const failure = {code:'revision_conflict',message:'File changed'};
  const {documents,cx,errors,api,set} = fixtureState('1',[current]);
  let attempts = 0;
  api.saveDocument = async () => {
    const state = {cursor:String(++attempts + 1),documents:[{...current,error:failure}],reveal:null};
    set(state); documents.accept(state,cx);
    throw new Error(JSON.stringify(failure));
  };
  await documents.save(cx);
  await documents.save(cx);
  assert.equal(attempts,2);
  assert.deepEqual(plain(errors.map(([error,document]) => [error,document.id])),[
    [failure,'draft'],[failure,'draft'],
  ]);
});

test('matching failures remain scoped to their documents', async () => {
  const items = [document('first',{dirty:true,can_save:true}),document('second',{dirty:true,can_save:true})];
  const failure = {code:'outcome_unknown',message:'Save was not confirmed'};
  const {documents,cx,errors,api,set} = fixtureState('1',items);
  let cursor = 1;
  api.saveDocument = async id => {
    const state = {cursor:String(++cursor),documents:documents.state.documents.map(item =>
      item.id === id ? {...item,error:failure,uncertain:true} : item),reveal:null};
    set(state); documents.accept(state,cx);
    throw new Error(JSON.stringify(failure));
  };
  await documents.save(cx);
  await documents.select('second',cx);
  await documents.save(cx);
  assert.deepEqual(plain(errors.map(([error,document]) => [error,document.id])),[
    [failure,'first'],[failure,'second'],
  ]);
});

test('concurrent save clicks share the active attempt', async () => {
  const current = document('draft',{dirty:true,can_save:true});
  const {documents,cx,errors,api} = fixtureState('1',[current]);
  let reject, attempts = 0;
  api.saveDocument = () => { attempts++; return new Promise((_,failed) => { reject = failed; }); };
  const saving = documents.save(cx);
  await documents.save(cx);
  reject(new Error(JSON.stringify({code:'outcome_unknown'})));
  await saving;
  assert.equal(attempts,1);
  assert.equal(errors.length,1);
});
