import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

export async function host(sdk={}) {
  const bridge=new vm.SyntheticModule(Object.keys(sdk),function() {
    for (const [name,value] of Object.entries(sdk)) this.setExport(name,value);
  });
  const modules=new Map([['sailry/sdk',bridge]]);
  async function load(path) {
    if (modules.has(path)) return modules.get(path);
    const module=new vm.SourceTextModule(await readFile(
      new URL(`../dev.sailry.platform/host/${path}`,import.meta.url),'utf8'),{identifier:path});
    modules.set(path,module);
    await module.link(load);
    return module;
  }
  const main=await load('./main.js');
  await main.evaluate();
  return main.namespace;
}
