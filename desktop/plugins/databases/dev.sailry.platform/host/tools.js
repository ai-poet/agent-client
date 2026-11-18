// Dynamic schemas retain captured connection choices; declarations live in plugin.json.
export function label(profile) {
  const engine = {sqlite:'SQLite',mysql:'MySQL',postgres:'PostgreSQL'}[profile.engine];
  const quoting = profile.engine === 'mysql'
    ? 'backtick-quoted identifiers; do not use double quotes for table or database names'
    : profile.engine === 'postgres'
      ? 'double-quoted identifiers; qualify tables with schema, not database'
      : 'double-quoted identifiers';
  return `${profile.name}; engine=${engine}; default_database=${profile.database}; read_only=${profile.read_only}; ${quoting}`;
}

export function parameters(name,profiles) {
  if (name === 'database_connections') return {type:'object',properties:{},required:[],additionalProperties:false};
  if (name === 'database_save_connection') return saveParameters();
  const choices = profiles.map(profile=>[profile.id,label(profile)]);
  const properties = {connection:{type:'string',pattern:uuid,description:JSON.stringify(choices)},
    database:{type:'string',description:'Database name; omit to use the saved default'}};
  const required = ['connection'];
  if (name !== 'database_catalog') { properties.sql={type:'string'};required.push('sql'); }
  return {type:'object',properties,required,additionalProperties:false};
}

const uuid = '^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$';
function saveParameters() {
  return {type:'object',additionalProperties:false,required:[],properties:{
    connection:{type:['string','null'],pattern:uuid,description:'Saved connection ID; omit with expected_revision to create'},
    expected_revision:{type:['integer','null'],minimum:1},
    name:{type:'string',minLength:1},engine:{type:'string',enum:['sqlite','mysql','postgres']},
    path:{type:'string',minLength:1,description:'SQLite file path on the execution Node; relative paths use the captured worktree. Saving does not create the file'},
    host:{type:'string',minLength:1},port:{type:'integer',minimum:1,maximum:65535},
    username:{type:'string',minLength:1},database:{type:'string'},read_only:{type:'boolean'},
  },allOf:[{
    if:{required:['connection'],properties:{connection:{type:'string'}}},
    then:{required:['expected_revision'],properties:{expected_revision:{type:'integer'}}},
    else:{required:['name','engine'],properties:{expected_revision:{type:'null'}},allOf:[{
      if:{properties:{engine:{const:'sqlite'}}},then:{required:['path']},
      else:{required:['host','port','username','database']},
    }]},
  }]};
}
