// Stand-ins for the desktop element builders: every style call chains, and
// the identifier, children, label, disabled state and click action are kept.
export function element(id,props) {
  const node={identity:id,props,items:[],text:[]};
  const methods={
    id(value) {node.identity=value;return proxy;},
    child(value) {(typeof value==='string' ? node.text : node.items).push(value);return proxy;},
    children(values) {for (const value of values) methods.child(value);return proxy;},
    label(value) {node.labelValue=value;return proxy;},
    disabled(value) {node.disabledValue=value;return proxy;},
    on_click(action) {node.action=action;return proxy;},
  };
  const proxy=new Proxy(node,{get:(target,name)=>name in target ? target[name] : methods[name] ?? (()=>proxy)});
  return proxy;
}
export function find(root,identity) {
  if (root.identity===identity) return root;
  for (const item of root.items) {const found=find(item,identity);if (found) return found;}
  return null;
}
export const actions=panel=>find(panel,'goal-actions').items.map(item=>item.identity);
