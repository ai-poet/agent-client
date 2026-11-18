import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {readFile} from 'node:fs/promises';

function deferred() {
  let resolve;
  const promise=new Promise(accept=>{resolve=accept;});
  return {promise,resolve};
}

for(const name of ['reversi','gomoku','xiangqi','poker','doudizhu','liars-dice','city-trader']) {
  test(`${name} starts independent configuration reads together`,async()=>{
    const source=(await readFile(new URL(`../../${name}/dev.sailry.platform/desktop/main.js`,import.meta.url),'utf8'))
      .replace(/^import[\s\S]*?;\n/gm,'').replace('export default class','class');
    const className=source.match(/class (\w+) extends View/)[1];
    const settings=deferred(),models=deferred(),saved=deferred(),calls=[],tasks=[];
    const Controller=vm.runInNewContext(`${source}\n${className}`,{
      View:class{},readSettings:()=>{calls.push('settings');return settings.promise;},
      listModels:()=>{calls.push('models');return models.promise;},
      getValue:key=>{calls.push(key);return saved.promise;},resolveModel:()=>({model:'selected'}),
      context:()=>'{}',messages:()=>({you:'You'}),
    });
    const view=new Controller(),cx={notify(){},spawn:body=>tasks.push(body(cx))};
    Object.assign(view,{text:{you:'You',opponent:'Opponent'},names:['You','First','Second']});
    view.configure(cx);
    assert.deepEqual(calls,name==='city-trader'?['settings','models','match']:['settings','models'],
      'every independent read starts before any result settles');
    assert.equal(view.loading,true);assert.equal(view.available,false);
    settings.resolve({values:{}});models.resolve({models:[]});saved.resolve({present:false});
    await Promise.all(tasks);assert.equal(view.loading,false);assert.equal(view.available,true);
    if(name==='city-trader')assert.equal(view.saved.present,false);
  });
}
