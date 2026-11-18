import test from 'node:test';
import assert from 'node:assert/strict';
import {overview,percent} from '../dev.sailry.platform/desktop/summary.js';

const totals=(cost)=>({responses:'4',tokens:{input:'100',output:'25',cached_input:'60',reasoning:'7'},
  generation:{output_tokens:'50',elapsed_us:'2000000'},...cost ? {cost} : {}});

test('composition splits cached, uncached and output tokens',()=>{
  const values=overview(totals());
  assert.equal(values.total,125n);
  assert.deepEqual(values.parts,[60n,40n,25n]);
  assert.deepEqual(values.shares.map(percent),['48.0%','32.0%','20.0%']);
  assert.equal(values.reasoning,7n);
  assert.equal(values.hit,60);
  assert.equal(values.responses,4n);
  assert.equal(values.speed,'25.0');
});

test('cost is partial when some responses lack prices',()=>{
  assert.deepEqual([overview(totals()).cost,overview(totals()).partial],[null,false]);
  const partial=overview(totals({usd_micros:'1500000',responses:'3'}));
  assert.deepEqual([partial.cost,partial.priced,partial.partial],['1.50',3n,true]);
  const full=overview(totals({usd_micros:'10000',responses:'4'}));
  assert.deepEqual([full.cost,full.partial],['0.01',false]);
});

test('unknown totals stay unknown',()=>{
  const values=overview({responses:'0',tokens:null});
  assert.deepEqual([values.total,values.parts,values.shares,values.hit,values.reasoning,values.speed],[null,null,null,null,null,null]);
  assert.equal(overview(null).responses,0n);
  assert.equal(overview({responses:'1',tokens:{input:'0',output:'0',cached_input:'0',reasoning:'0'}}).hit,null);
  assert.equal(percent(null),null);
});
