import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import test from 'node:test';
import vm from 'node:vm';
import {cases} from './parameters.mjs';

const root = new URL('../',import.meta.url);
const manifest = JSON.parse(await readFile(new URL('plugin.json',root),'utf8'));
const extension = manifest.extensions['dev.sailry.platform'];
const source = (await readFile(new URL('dev.sailry.platform/host/main.js',root),'utf8')).replace(/^export \{[^\n]+\} from [^\n]+;\n/gm,'').replace(/^export /gm,'');
const host = vm.runInNewContext(`${source}\n({list,read,write,edit,apply,remove,move,mkdir,find,search,notebook_read,notebook_edit})`);
const plain = value => JSON.parse(JSON.stringify(value));
const tool = name => extension.tools.find(tool => tool.name === name);

test('tool declarations preserve operations and unformatted native outputs', () => {
  const expected = [
    ['list_directory','list','files.list','summary'],
    ['read_file','read','files.read','summary'],
    ['write_file','write','files.write','details'],
    ['edit_file','edit','files.edit','details'],
    ['apply_patch','apply','files.apply','details'],
    ['delete_file','remove','files.delete','summary'],
    ['move_file','move','files.move','summary'],
    ['create_directory','mkdir','files.mkdir','summary'],
    ['find_files','find','files.find','summary'],
    ['search_files','search','files.search','summary'],
    ['read_notebook','notebook_read','notebook.read','summary'],
    ['edit_notebook','notebook_edit','notebook.edit','details'],
  ];
  assert.equal(manifest.name,'files');
  assert.deepEqual(extension.tools.map(tool => tool.name),expected.map(([name]) => name));
  assert.deepEqual(extension.actions,['files.read','files.write']);
  assert.deepEqual(extension.host.handlers,expected.map(([,handler]) => handler));
  for (const [name,handler,operation,presentation] of expected) {
    const declaration = tool(name);
    assert.equal(declaration.handler.name,handler);
    assert.equal(declaration.handler.operation,operation);
    assert.equal(declaration.presentation,presentation);
    assert.deepEqual(declaration.contexts,['plugin','workspace'],
      'file tools allow project and assistant worktrees, without database or SSH resources');
    assert.equal(declaration.handler.result,undefined,'the entire native Output must bypass the VM');
    assert.equal(declaration.handler.parameters.additionalProperties,false);
    assert.equal(typeof host[handler],'function');
    for (const ownership of ['node','worktree','session','turn']) {
      assert.equal(declaration.handler.parameters.properties[ownership],undefined);
    }
    assert.ok(declaration.display.locales['zh-CN']);
    assert.match(declaration.display.icon.svg,/viewBox="0 0 24 24"/);
  }
  assert.match(tool('write_file').description,/conflict requires reading the current file/);
  assert.match(tool('read_file').description,/one page is not the whole file/);
  assert.equal(tool('get_office_runtime'),undefined);
});

test('directory schema and pagination retain cursor identity and root spelling', () => {
  const schema = tool('list_directory').handler.parameters;
  assert.deepEqual(schema.required,[]);
  assert.deepEqual(schema.properties.cursor.type,['object','null']);
  assert.deepEqual(schema.properties.cursor.required,['revision','directory','name']);
  assert.equal(schema.properties.cursor.additionalProperties,false);
  assert.equal(schema.properties.cursor.properties.revision.minLength,1);
  assert.equal(schema.properties.cursor.properties.name.minLength,1);
  assert.deepEqual(plain(host.list({})),{path:'',after:null});
  assert.deepEqual(plain(host.list({path:'.',cursor:null})),{path:'',after:null});
  const cursor = {revision:'captured-directory-revision',directory:true,name:'资料'};
  const args = {path:'nested/资料',cursor};
  const prepared = host.list(args);
  assert.equal(prepared.after,cursor);
  assert.deepEqual(plain(prepared),{path:'nested/资料',after:cursor});
  assert.deepEqual(args,{path:'nested/资料',cursor});
  for (const path of ['./','..',' a/b ','资料.txt']) assert.equal(host.list({path}).path,path);
});

test('writes retain exact text and optimistic revision semantics', () => {
  const schema = tool('write_file').handler.parameters;
  assert.deepEqual(schema.required,['path','text','expected_revision']);
  assert.deepEqual(schema.properties.expected_revision.type,['string','null']);
  const text = '  资料 🙂\n\\literal\t\u0001';
  const current = 'a'.repeat(64);
  for (const expected_revision of [null,current]) {
    const args = Object.freeze({path:'nested/资料.txt',text,expected_revision});
    assert.deepEqual(plain(host.write(args)),args);
  }
  // The original serde Option accepts omitted revisions, although the schema asks for one.
  assert.deepEqual(plain(host.write({path:'new.txt',text:''})),{
    path:'new.txt',text:'',expected_revision:null,
  });
  assert.equal(host.write({path:'text.txt',text:'\u0001'.repeat(128 * 1024)}).text.length,128 * 1024);
  assert.deepEqual(plain(host.read({path:'nested/资料.txt'})),{path:'nested/资料.txt',offset:0,limit:2000});
});

test('search defaults keep original expressions and globs without interpreting them', () => {
  const schema = tool('search_files').handler.parameters;
  assert.deepEqual(schema.required,['query']);
  assert.equal(schema.properties.query.minLength,1);
  assert.equal(schema.properties.mode.default,'files');
  assert.equal(schema.properties.globs.maxItems,undefined);
  assert.equal(schema.properties.globs.items.maxLength,undefined);
  assert.deepEqual(plain(host.search({query:' needle\n'})),{
    query:' needle\n',regex:false,case_sensitive:false,globs:[],path:'',mode:'files',context:0,type:null,limit:100,offset:0,multiline:false,
  });
  const globs = ['**/*.rs','资料/*.txt','**/*.rs'];
  const args = {query:'^a.*[中]$',regex:true,case_sensitive:true,globs};
  assert.deepEqual(plain(host.search(args)),{path:'',mode:'files',context:0,type:null,limit:100,offset:0,multiline:false,...args});
  assert.equal(host.search(args).globs,globs);
  assert.deepEqual(args,{query:'^a.*[中]$',regex:true,case_sensitive:true,globs});
});

test('invalid argument shapes fail before requesting an operation', () => {
  const examples = {
    list:{path:'.'},read:{path:'file.txt'},write:{path:'file.txt',text:'x',expected_revision:null},
    edit:{path:'file.txt',old_text:'x',new_text:'y',expected_revision:'a'.repeat(64)},
    apply:{input:'*** Begin Patch\n*** Add File: file.txt\n+x\n*** End Patch'},
    remove:{path:'file.txt',expected_revision:'a'.repeat(64)},
    move:{path:'file.txt',destination:'new.txt',expected_revision:'a'.repeat(64)},
    mkdir:{path:'folder'},
    find:{globs:['**/*.txt']},search:{query:'x'},
  };
  for (const [name,valid] of Object.entries(examples)) {
    for (const args of [null,[],true,'path',42,...['worktree','session','node','turn','unexpected'].map(key => ({...valid,[key]:'foreign'}))]) {
      const result = host[name](args);
      assert.equal(result.isError,true,`${name}: ${JSON.stringify(args)}`);
      assert.equal(result.error.code,'invalid_request');
      assert.equal(result.path,undefined);
    }
  }
  const invalid = {
    list:[{path:null},{cursor:[]},{cursor:{revision:'r',directory:'yes',name:'n'}},{cursor:{revision:'r',directory:true}},{cursor:{revision:'r',directory:true,name:'n',worktree:'foreign'}}],
    read:[{}, {path:4},{path:'a',offset:-1},{path:'a',limit:0},{path:'a',limit:Number.MAX_SAFE_INTEGER+1},{path:'a',offset:1.5}],
    edit:[{}, {path:'a',old_text:'',new_text:'y',expected_revision:'a'.repeat(64)},{path:'a',old_text:'x',new_text:'y',expected_revision:null}],
    remove:[{}, {path:'a',expected_revision:null},{path:'a',expected_revision:'old'}],
    move:[{}, {path:'a',expected_revision:'a'.repeat(64)},{path:'a',destination:1,expected_revision:'a'.repeat(64)}],
    mkdir:[{}, {path:null}],
    find:[{}, {globs:[]},{globs:['']},{globs:[4]}],
    write:[{path:'a'}, {path:'a',text:[]}, {path:'a',text:'x',expected_revision:4}],
    search:[{}, {query:4}, {query:'x',regex:null}, {query:'x',case_sensitive:1}, {query:'x',globs:'*'}, {query:'x',globs:[4]}],
  };
  for (const [name,examples] of Object.entries(invalid)) {
    for (const args of examples) assert.equal(host[name](args).error.code,'invalid_request',`${name}: ${JSON.stringify(args)}`);
  }
});

test('captured display selectors retain path and first-line query presentation', () => {
  assert.deepEqual(tool('search_files').display.input.summary,{path:'/query',code:true});
  for (const name of ['read_file','write_file']) {
    assert.deepEqual(tool(name).display.input.summary,{path:'/path',code:true});
  }
  assert.deepEqual(tool('write_file').display.input.target,{path:'/path'});
});

test('result recipes refer to original data and preserve file-specific choices', () => {
  const reference = (source,path) => ({source,path});
  assert.deepEqual(tool('read_file').display.output.paths,{value:reference('arguments','/path')});
  assert.deepEqual(tool('search_files').display.output.paths,{
    value:reference('result','/data/matches'),item:'/path',
  });
  assert.deepEqual(tool('write_file').display.output.body.text,reference('arguments','/text'));
  for (const name of ['write_file','edit_file']) {
    assert.deepEqual(tool(name).display.output.body.diff,{format:'checkpoint'});
    assert.deepEqual(tool(name).display.output.body.path,reference('arguments','/path'));
  }
  const prompts = tool('write_file').display.approval;
  assert.deepEqual(prompts[0].when,{any:[{value:reference('arguments','/expected_revision'),equals:null}]});
  assert.equal(prompts[0].message.label,'Create %{path}');
  assert.equal(prompts[1].message.label,'Replace %{path}');
  assert.deepEqual(prompts[1].values.path,reference('arguments','/path'));
  const partial = tool('list_directory').display.output.notices[0];
  assert.deepEqual(partial.when.any,[
    {value:reference('result','/data/truncated'),equals:true},
    {value:reference('result','/data/unsupported_names'),equals:0,negate:true},
  ]);
});


test('line pages retain offsets and exact edit fragments', () => {
  assert.deepEqual(plain(host.read({path:'large.txt',offset:3000,limit:25})),{path:'large.txt',offset:3000,limit:25});
  const args = {path:'large.txt',old_text:'old\r\n',new_text:'new\r\n',expected_revision:'a'.repeat(64)};
  assert.deepEqual(plain(host.edit(args)),args);
  assert.deepEqual(tool('edit_file').handler.parameters.required,['path','expected_revision']);
  assert.equal(tool('edit_file').display.approval[0].message.label,'Edit %{path}');
  assert.deepEqual(plain(host.find({globs:['**/*.rs','src/**/mod.rs']})),{globs:['**/*.rs','src/**/mod.rs'],path:'',limit:500,offset:0});
  assert.equal(tool('find_files').handler.operation,'files.find');
  assert.equal(tool('read_file').handler.parameters.properties.offset.minimum,0);
  const limit = tool('read_file').handler.parameters.properties.limit;
  assert.equal(limit.default,2000);
  assert.equal(limit.maximum,undefined);
  assert.match(tool('read_file').description,/32 KiB page budget and a 2 KiB line budget/);
  assert.match(tool('read_file').description,/shortened long line is not a new page/);
  assert.match(tool('read_file').description,/read_tool_output with the returned output reference/);
  assert.deepEqual(tool('read_file').display.output.notices[0].when,{
    any:[
      {value:{source:'result',path:'/data/next_offset'},equals:null,negate:true},
      {value:{source:'result',path:'/data/offset'},equals:0,negate:true},
    ],
    all:[{value:{source:'result',path:'/data/lines'},equals:null,negate:true}],
  });
});

test('image reads keep recent images and describe non-vision facts', () => {
  const declaration = tool('read_file');
  assert.equal(declaration.keep_images,5);
  assert.equal(extension.min_sailry_version,'0.1.6');
  for (const pattern of [/\.png, \.jpg, \.jpeg, \.gif, \.webp or \.bmp/,/5 MiB/,/1568-pixel long side/,
    /format, dimensions and size/,/images lists the reference/,/inspect_image with the path/]) {
    assert.match(declaration.description,pattern);
  }
  assert.match(extension.instructions,/worktree image with read_file/);
});

test('batch edits preserve atomic arguments and enforce exclusive forms', () => {
  const expected_revision = 'a'.repeat(64);
  const edits = Object.freeze([
    Object.freeze({old_text:'first\r\n',new_text:'changed\r\n'}),
    Object.freeze({old_text:'🙂',new_text:'中文',replace_all:true}),
  ]);
  const arguments_ = Object.freeze({path:'file.txt',edits,expected_revision});
  assert.deepEqual(plain(host.edit(arguments_)),arguments_);
  assert.equal(host.edit(arguments_).edits,edits);
  assert.deepEqual(plain(host.edit({path:'file.txt',old_text:'old',new_text:'new',replace_all:true,expected_revision})),{
    path:'file.txt',old_text:'old',new_text:'new',replace_all:true,expected_revision,
  });
  for (const shape of [
    {edits,old_text:'first',new_text:'changed'}, {edits,old_text:'first'}, {edits,new_text:'changed'},
    {edits,replace_all:true}, {edits:[]}, {edits:null},
    {edits:[{old_text:'',new_text:'changed'}]}, {edits:[{old_text:'first',new_text:'changed',replace_all:1}]},
    {edits:[{old_text:'first',new_text:'changed',path:'other.txt'}]},
  ]) {
    const result = host.edit({path:'file.txt',expected_revision,...shape});
    assert.equal(result.error.code,'invalid_request',JSON.stringify(shape));
  }
  const schema = tool('edit_file').handler.parameters;
  assert.equal(schema.properties.edits.maxItems,undefined);
  assert.equal(schema.properties.edits.items.additionalProperties,false);
  assert.deepEqual(schema.properties.edits.items.required,['old_text','new_text']);
  assert.equal(schema.oneOf.length,2);
  assert.match(extension.instructions,/find_files and search_files.*rather than shell commands/);
  assert.doesNotMatch(extension.instructions,/original content/);
  assert.match(tool('edit_file').description,/original file/);
  assert.match(tool('edit_file').description,/fail without writing any change/);
});

test('edit diagnostics identify fields without echoing content', () => {
  const privateValue = 'private-edit-fixture-content';
  const valid = {path:privateValue,old_text:privateValue,new_text:privateValue,expected_revision:'a'.repeat(64)};
  const change = {old_text:privateValue,new_text:privateValue};
  const batch = {path:privateValue,expected_revision:'a'.repeat(64)};
  const cases = [
    [null,'Expected edit arguments with supported fields'],
    [{...valid,[privateValue]:privateValue},'Expected edit arguments with supported fields'],
    [{...valid,path:42},'path must be a string'],
    [{...valid,expected_revision:privateValue},'expected_revision must be the 64-character revision from read_file'],
    [{...valid,expected_revision:null},'expected_revision must be the 64-character revision from read_file'],
    [{path:privateValue,old_text:privateValue,new_text:privateValue},'expected_revision must be the 64-character revision from read_file'],
    [{...valid,old_text:''},'old_text must be a nonempty string'],
    [{...valid,old_text:42},'old_text must be a nonempty string'],
    [{...valid,new_text:42},'new_text must be a string'],
    [{...valid,replace_all:privateValue},'replace_all must be a boolean'],
    [{...valid,edits:[change]},'Use either edits or old_text and new_text'],
    [{...batch,edits:[change],replace_all:true},'Set replace_all on each edits entry'],
    [{...batch,edits:null},'edits must be a nonempty array'],
    [{...batch,edits:[]},'edits must be a nonempty array'],
    [{...batch,edits:[change,{...change,[privateValue]:privateValue}]},'edits[1] must contain supported edit fields'],
    [{...batch,edits:[change,{...change,old_text:''}]},'edits[1].old_text must be a nonempty string'],
    [{...batch,edits:[change,{...change,new_text:42}]},'edits[1].new_text must be a string'],
    [{...batch,edits:[change,{...change,replace_all:privateValue}]},'edits[1].replace_all must be a boolean'],
  ];
  for (const [args,message] of cases) {
    const result = host.edit(args);
    assert.deepEqual(plain(result),{isError:true,error:{code:'invalid_request',message}});
    assert.ok(!JSON.stringify(result).includes(privateValue));
  }
  assert.deepEqual(plain(host.edit({...batch,edits:[change],replace_all:false})),{...batch,edits:[change]});
  assert.equal(host.read({path:42}).error.message,'Invalid file tool arguments');
});

test('file mutations retain revisions and explicit destinations', () => {
  const expected_revision = 'b'.repeat(64);
  assert.deepEqual(plain(host.remove({path:'note.txt',expected_revision})),{path:'note.txt',expected_revision});
  assert.deepEqual(plain(host.move({path:'note.txt',destination:'nested/note.txt',expected_revision})),{
    path:'note.txt',destination:'nested/note.txt',expected_revision,
  });
  assert.deepEqual(plain(host.mkdir({path:'nested'})),{path:'nested'});
  assert.match(tool('delete_file').description,/Trash/);
  assert.match(tool('move_file').description,/never overwrite/);
  assert.match(tool('create_directory').description,/existing directory is unchanged/);
  assert.deepEqual(tool('delete_file').handler.parameters.required,['path','expected_revision']);
  assert.deepEqual(tool('move_file').handler.parameters.required,['path','destination','expected_revision']);
});


test('patches pass through unchanged for Node parsing', () => {
  const input = '*** Begin Patch\n*** Add File: src/新.rs\n+fn main() {}\n*** Update File: src/lib.rs\n@@\n-a\r\n+b\r\n*** End Patch\n';
  const args = Object.freeze({input});
  assert.deepEqual(plain(host.apply(args)),args);
  assert.equal(host.apply(args).input,input);
  for (const value of [null,{},{input:''},{input:4},{input:['patch']},{input,worktree:'foreign'},
    {changes:[{kind:'create',path:'a',text:'x'}]}]) {
    assert.deepEqual(plain(host.apply(value)),
      {isError:true,error:{code:'invalid_request',message:'input must be the whole patch text'}},JSON.stringify(value));
  }
  assert.equal(tool('apply_changes'),undefined,'one multi-file tool remains');
  const declaration = tool('apply_patch');
  const schema = declaration.handler.parameters;
  assert.deepEqual(schema.required,['input']);
  assert.deepEqual(Object.keys(schema.properties),['input']);
  assert.equal(schema.properties.input.type,'string');
  assert.equal(schema.properties.input.minLength,1);
  assert.equal(declaration.display.approval,undefined,'the Node lists each validated file in its approval');
  assert.equal(declaration.display.label,'Apply patch');
  assert.equal(Object.keys(declaration.display.locales).length,10);
  assert.deepEqual(declaration.display.output.paths,{value:{source:'result',path:'/data/files'},item:'/path'});
  for (const pattern of [/\*\*\* Begin Patch\n/,/\*\*\* End Patch\n/,/\*\*\* Move to:/,/\*\*\* End of File/,
    /match the current file exactly/,/Read every file you update, move or delete first/,/right after that line/,/counting a moved and edited file twice/,
    /validated before anything is written/,/without rolling back earlier files/,/Inspect an unknown file/]) {
    assert.match(declaration.description,pattern);
  }
  assert.match(tool('edit_file').description,/Use apply_patch/);
  assert.doesNotMatch(tool('edit_file').description,/128/);
});

test('scoped search modes and discovery pages preserve arguments', () => {
  const args = {query:'first.*second',regex:true,case_sensitive:true,globs:['src/**'],path:'src',
    mode:'count',context:5,type:'python',limit:11,offset:200,multiline:true};
  assert.deepEqual(plain(host.search(Object.freeze(args))),args);
  assert.equal(host.search({...args,path:'.'}).path,'');
  assert.deepEqual(plain(host.find({globs:['**/*.rs'],path:'src',limit:1,offset:500})),{
    globs:['**/*.rs'],path:'src',limit:1,offset:500,
  });
  const schema = tool('search_files').handler.parameters;
  assert.deepEqual(schema.properties.mode.enum,['content','files','count']);
  assert.equal(schema.properties.context.maximum,undefined);
  assert.equal(schema.properties.limit.maximum,undefined);
  assert.equal(tool('find_files').handler.parameters.properties.limit.maximum,undefined);
  assert.match(tool('search_files').description,/ripgrep/);
  assert.match(tool('search_files').description,/32 KiB total budget/);
  assert.match(tool('search_files').description,/original captured result/);
  assert.match(tool('search_files').description,/next_offset.*cannot recover a shortened line/);
  assert.match(tool('search_files').description,/Partial|Incomplete/);
  for (const args of [
    {query:'x',mode:'all'}, {query:'x',context:-1}, {query:'x',context:1.5},
    {query:'x',limit:0}, {query:'x',offset:-1}, {query:'x',offset:1.5},
    {query:'x',multiline:'yes'}, {query:'x',type:1}, {query:'x',path:null}, {query:''},
  ]) assert.equal(host.search(args).error.code,'invalid_request');
  for (const args of [
    {globs:['*'],path:null}, {globs:['*'],limit:0},
    {globs:['*'],offset:-1}, {globs:['*'],offset:0.5},
  ]) assert.equal(host.find(args).error.code,'invalid_request');
});

test('search context and page requests retain caller intent', () => {
  for (const entry of cases()) {
    const args = structuredClone(entry.input), original = structuredClone(args);
    const output = plain(host[entry.handler](args));
    assert.deepEqual(args,original);
    if (entry.valid) assert.deepEqual(output,entry.expected);
    else {
      assert.equal(output.isError,true);
      assert.equal(output.error.code,'invalid_request');
      if (entry.message) assert.equal(output.error.message,entry.message);
      assert.ok(!output.error.message.includes('private fixture'));
    }
  }
});


test('notebook handlers preserve original indices and exact cell sources', () => {
  assert.deepEqual(plain(host.notebook_read({path:'report.ipynb'})),{path:'report.ipynb',offset:0,limit:20});
  const edits = [{kind:'replace',index:1,source:'print("資料")\n'},
    {kind:'insert',index:1,cell_type:'markdown',source:'# Added'}, {kind:'delete',index:2}];
  const input = Object.freeze({path:'report.ipynb',expected_revision:'a'.repeat(64),edits});
  assert.deepEqual(plain(host.notebook_edit(input)),input);
  assert.equal(host.notebook_edit(input).edits,edits);
  for (const change of [{kind:'execute',index:0},{kind:'delete',index:-1},
    {kind:'insert',index:0,cell_type:'javascript',source:''},{kind:'replace',index:0,source:'bad\0'}]) {
    assert.equal(host.notebook_edit({...input,edits:[change]}).isError,true);
  }
  assert.deepEqual(plain(host.notebook_read({path:'report.ipynb',limit:101})),
    {path:'report.ipynb',offset:0,limit:101});
  assert.equal(tool('edit_notebook').handler.parameters.properties.edits.maxItems,undefined);
  assert.match(tool('edit_notebook').description,/retains the cell ID, metadata and saved outputs/);
});
