import test from 'node:test';
import assert from 'node:assert/strict';
for (const name of ['gomoku','reversi','xiangqi','poker','doudizhu','liars-dice','city-trader']) {
  const {command,select}=await import(`../${name}/dev.sailry.platform/desktop/${name==='city-trader'?'game':'model'}.js`);
  test(`${name}: package validates offered moves`,()=>{
    const player={kind:'provider',model:'provider/model',effort:'high'};
    const request=command(player,{instructions:'Choose',state:{visible:['card']},choices:[{id:0,kind:'pass'}]});
    assert.equal(request.data.effort,'high');
    const state=JSON.parse(request.data.prompt.split('\n').at(-1));
    assert.deepEqual(state,{state:{visible:['card']},choices:[{id:0,kind:'pass'}]});
    const response=text=>({Ok:{kind:'plugin_text',data:{text,tokens:5}}});
    assert.deepEqual(select(response('```json\n{"move":0}\n```'),player,['pass']),{move:'pass',tokens:5});
    for (const move of [-1,'01',0.5,1,null]) assert.throws(()=>select(response(JSON.stringify({move})),player,['pass']),/invalidResponse/);
    assert.throws(()=>select({Err:{code:'not_configured'}},player,[]),/configure/);
    assert.throws(()=>select({Err:{code:'outcome_unknown'}},player,[]),/unconfirmed/);
    assert.throws(()=>select({Err:{message:'plugin model request timed out'}},player,[]),/timedOut/);
  });
}
