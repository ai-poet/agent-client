/** Shared adapter cases; no filesystem or search service is executed here. */
export function cases() {
  const search = {query:'needle',regex:false,case_sensitive:false,globs:['**/*.rs'],path:'src',
    mode:'content',context:0,type:null,limit:100,offset:0,multiline:false};
  const rows = [0,2,3,4,5,6,8].map(context => ({
    tool:'search_files',handler:'search',input:{...search,context},valid:true,expected:{...search,context},
  }));
  rows.push({tool:'search_files',handler:'search',input:{query:'needle'},valid:true,
    expected:{...search,globs:[],path:'',mode:'files'}});
  rows.push({tool:'read_file',handler:'read',input:{path:'note.txt'},valid:true,
    expected:{path:'note.txt',offset:0,limit:2000}});
  for (const change of [{limit:201},{limit:Number.MAX_SAFE_INTEGER},{query:'x'.repeat(4097)},
    {globs:Array(65).fill('**/*.rs')},{globs:['a'.repeat(4097)]}]) {
    rows.push({tool:'search_files',handler:'search',input:{...search,...change},valid:true,
      expected:{...search,...change}});
  }
  for (const [tool,handler,input,expected] of [
    ['read_file','read',{path:'note.txt',limit:2001},{path:'note.txt',offset:0,limit:2001}],
    ['find_files','find',{globs:['**/*.rs'],limit:501},{globs:['**/*.rs'],path:'',offset:0,limit:501}],
    ['read_notebook','notebook_read',{path:'note.ipynb',limit:101},{path:'note.ipynb',offset:0,limit:101}],
  ]) rows.push({tool,handler,input,expected,valid:true});
  for (const globs of [Array(65).fill('**/*.rs'),['a'.repeat(4097)]]) {
    rows.push({tool:'find_files',handler:'find',input:{globs},valid:true,
      expected:{globs,path:'',offset:0,limit:500}});
  }
  for (const context of [-1,1.5,null,'8',Number.MAX_SAFE_INTEGER+1]) {
    rows.push({tool:'search_files',handler:'search',input:{...search,context},valid:false,
      schemaValid:context === Number.MAX_SAFE_INTEGER+1,
      message:'context must be a non-negative safe integer'});
  }
  for (const limit of [0,-1,1.5,null,'201',Number.MAX_SAFE_INTEGER+1]) {
    rows.push({tool:'search_files',handler:'search',input:{...search,limit},valid:false,
      schemaValid:limit === Number.MAX_SAFE_INTEGER+1,
      message:'limit must be a positive safe integer'});
  }
  for (const input of [{...search,path:null},{...search,unexpected:'private fixture'},
    {...search,query:''},{...search,mode:'all'},{...search,globs:[42]}]) {
    rows.push({tool:'search_files',handler:'search',input,valid:false});
  }
  for (const globs of [[],[''],[42],null]) {
    rows.push({tool:'find_files',handler:'find',input:{globs},valid:false});
  }
  const revision = 'a'.repeat(64);
  for (const [tool,handler,edits] of [
    ['edit_file','edit',Array.from({length:129},(_,i)=>({old_text:`old-${i}`,new_text:`new-${i}`}))],
    ['edit_notebook','notebook_edit',Array.from({length:101},()=>({kind:'insert',index:0,cell_type:'raw',source:'x'}))],
    ['edit_notebook','notebook_edit',[{kind:'replace',index:0,source:'x'.repeat(524289)}]],
  ]) {
    const input = {path:tool === 'edit_file' ? 'note.txt' : 'note.ipynb',expected_revision:revision,edits};
    rows.push({tool,handler,input,expected:input,valid:true});
  }
  return rows;
}
