import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const code = await readFile(new URL('../dev.sailry.platform/desktop/format.js',import.meta.url),'utf8');
const format = await import('data:text/javascript;base64,'+Buffer.from(code).toString('base64'));

test('missing usage and prices stay unknown while reported zero remains zero',()=>{
  assert.equal(format.total(null),null);
  assert.equal(format.amount(null),null);
  assert.equal(format.cost({responses:'4',cost:null}),null);
  assert.equal(format.amount('0'),'0.00');
  assert.equal(format.speed({elapsed_us:'0',output_tokens:'9'}),null);
  assert.equal(format.cache({input:'0',cached_input:'0'}),null);
});
test('money and aggregate counts preserve integers beyond the script number limit',()=>{
  assert.equal(format.amount('9007199254740991'),'9007199254.74');
  assert.equal(format.total({input:'18446744073709551615',output:'18446744073709551615'}),36893488147419103230n);
  assert.equal(format.amount('999999'),'0.99');
  assert.equal(format.cost({responses:'5',cost:{responses:'3',usd_micros:'12300'}}),'0.01+');
});
test('formatting retains original compact units and context thresholds',()=>{
  assert.deepEqual(['999','1000','92027','7234549','1200000000'].map(format.compact),['999','1.0k','92.0k','7.2M','1.2B']);
  assert.equal(format.context('120','100'),120);
  assert.equal(format.context('20','0'),null);
  assert.deepEqual([null,79.9,80,99.9,100,120].map(format.severity),['muted','muted','warning','warning','danger','danger']);
});
test('unscaled token counts include their unit',()=>{
  const withUnit=value=>format.tokenCount(value,'%{count} tokens');
  assert.deepEqual([null,'0','400','999','1000','2500000'].map(withUnit),
    [null,'0 tokens','400 tokens','999 tokens','1.0k','2.5M']);
  assert.equal(format.tokenCount('400','Tokens: %{count}'),'Tokens: 400');
});
test('grouping, shares and request prices stay exact',()=>{
  assert.deepEqual([null,'0','999','1712','18446744073709551615'].map(format.grouped),[null,'0','999','1,712','18,446,744,073,709,551,615']);
  assert.equal(format.ratio('1','4'),25);
  assert.equal(format.ratio('1','0'),null);
  assert.equal(format.ratio(null,'4'),null);
  assert.deepEqual(['41200','123','18446744073709551615'].map(format.price),['0.0412','0.0001','18446744073709.5516']);
  assert.equal(format.price(null),null);
});
