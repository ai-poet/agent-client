export class Element {
  constructor(id = null) {
    this.items = [];
    this.props = id === null ? {} : { id };
    return new Proxy(this, { get(target, key, receiver) {
      if (key in target) return typeof target[key] === "function" ? target[key].bind(receiver) : target[key];
      return (...values) => { target.props[key] = values[0]; return receiver; };
    } });
  }
  child(value) { if (value !== null && value !== undefined) this.items.push(value); return this; }
  children(values) { values.forEach(value => this.child(value)); return this; }
  static new(id, props = {}) { const item = new Element().id(id); Object.assign(item.props, props); return item; }
}

export function flatten(root) {
  return root instanceof Element ? [root, ...root.items.flatMap(flatten)] : [root];
}
