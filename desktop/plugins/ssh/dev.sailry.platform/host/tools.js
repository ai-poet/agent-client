// Dynamic schemas retain captured connection choices; declarations live in plugin.json.
export function parameters(name,profiles,primary) {
  if (name === 'ssh_connections') return {type:'object',properties:{},required:[],additionalProperties:false};
  if (name === 'ssh_save_connection') return saveParameters();
  const choices = profiles.map(profile=>[profile.id,profile.name]);
  const properties = {connection:{type:'string',pattern:uuid,description:JSON.stringify(choices)}};
  if (primary) properties.connection.default = primary;
  const required = ['connection'];
  if (name === 'ssh_run') {
    properties.command={type:'string'};
    properties.timeout_ms={type:'integer',minimum:1,maximum:600000};
    required.push('command');
  } else {
    properties.path={type:'string'};properties.remote_path={type:'string'};
    properties.direction={type:'string',enum:['upload','download']};
    required.push('path','remote_path','direction');
  }
  return {type:'object',properties,required,additionalProperties:false};
}

const uuid = '^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$';
function saveParameters() {
  return {type:'object',additionalProperties:false,required:[],properties:{
    connection:{type:['string','null'],pattern:uuid,description:'Saved connection ID; omit with expected_revision to create'},
    expected_revision:{type:['integer','null'],minimum:1},
    name:{type:'string',minLength:1},host:{type:'string',minLength:1},
    port:{type:'integer',minimum:1,maximum:65535},username:{type:'string',minLength:1},
    authentication:{type:'string',enum:['password','private_key','key_path','agent']},
    credential:{oneOf:[
      {type:'object',properties:{kind:{const:'password'},password:{type:'string'}},required:['kind','password'],additionalProperties:false},
      {type:'object',properties:{kind:{const:'private_key'},key:{type:'string'},passphrase:{type:['string','null']}},required:['kind','key'],additionalProperties:false},
      {type:'object',properties:{kind:{const:'key_path'},path:{type:'string'},passphrase:{type:['string','null']}},required:['kind','path'],additionalProperties:false},
      {type:'object',properties:{kind:{const:'agent'}},required:['kind'],additionalProperties:false},
    ]},
    key_path:{type:'string',minLength:1,description:'Path to an existing private key on the execution Node; never its contents'},
  },allOf:[{
    if:{required:['connection'],properties:{connection:{type:'string'}}},
    then:{required:['expected_revision'],properties:{expected_revision:{type:'integer'}}},
    else:{required:['name','host','port','username','authentication'],properties:{expected_revision:{type:'null'}},allOf:[{
      if:{properties:{authentication:{enum:['password','private_key']}}},then:{required:['credential']},
    },{
      if:{properties:{authentication:{const:'key_path'}}},then:{anyOf:[{required:['credential']},{required:['key_path']}]},
    }]},
  }]};
}
