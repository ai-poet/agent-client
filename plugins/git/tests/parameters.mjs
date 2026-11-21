/** Shared adapter cases retain snapshot and revision values verbatim. */
export function cases() {
  const target = {kind:'worktree',data:{scope:'all'}};
  const snapshot = {target:{kind:'worktree',data:{scope:'all',head:'c'.repeat(40),
    index_revision:'d'.repeat(64)}},metadata_revision:'a'.repeat(64)};
  const rows = [];
  for (const limit of [501,2001,4000]) {
    rows.push(
      {tool:'git_log',handler:'log',input:{limit},valid:true,
        expected:{worktree:null,limit,cursor:null,revision:null,path:null}},
      {tool:'git_show',handler:'show',input:{path:'src/main.rs',limit},valid:true,
        expected:{worktree:null,revision:'HEAD',path:'src/main.rs',offset:0,limit}},
      {tool:'git_blame',handler:'blame',input:{path:'src/main.rs',limit},valid:true,
        expected:{worktree:null,revision:null,path:'src/main.rs',offset:0,limit}},
      {tool:'list_git_changes',handler:'changes',input:{limit},valid:true,
        expected:{worktree:null,target,cursor:null,limit}},
      {tool:'read_git_change',handler:'change',input:{snapshot,path:'src/main.rs',
        expected_metadata_revision:'b'.repeat(64),limit},valid:true,
        expected:{worktree:null,snapshot,path:'src/main.rs',expected_metadata_revision:'b'.repeat(64),
          expected_revision:null,offset:0,limit}},
    );
  }
  for (const entry of rows.slice(0,5)) {
    for (const limit of [0,-1,1.5,null,'4000',Number.MAX_SAFE_INTEGER+1]) {
      rows.push({...entry,input:{...entry.input,limit},valid:false,schemaValid:limit === Number.MAX_SAFE_INTEGER+1});
    }
  }
  const cursor = {head:'a'.repeat(40),after:'b'.repeat(40),offset:20};
  for (const input of [{revision:'main',path:'src/资料.rs'},{cursor:{...cursor,path:'src'},path:'src'},
    {cursor:{...cursor,offset:0}},{cursor:{...cursor,path:'src/资料.rs'},path:'src/资料.rs'},
    {cursor:null,revision:null,path:null}]) {
    rows.push({tool:'git_log',handler:'log',input,valid:true,
      expected:{worktree:null,limit:20,cursor:null,revision:null,path:null,...input}});
  }
  for (const input of [{cursor:{head:cursor.head,offset:5}},
    {cursor:{head:cursor.head,after:cursor.after}},{cursor:{...cursor,after:'short'}},
    {revision:8},{path:[]},{cursor:'HEAD'}]) {
    rows.push({tool:'git_log',handler:'log',input,valid:false});
  }
  for (const offset of [-1,1.5,null,'20',Number.MAX_SAFE_INTEGER+1]) {
    rows.push({tool:'git_log',handler:'log',input:{cursor:{...cursor,offset}},valid:false,
      schemaValid:offset === Number.MAX_SAFE_INTEGER+1});
  }
  return rows;
}
