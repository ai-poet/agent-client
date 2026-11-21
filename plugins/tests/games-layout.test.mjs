import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { Element, flatten } from "./elements.mjs";

const games = [["gomoku", "gomoku"], ["reversi", "reversi"], ["xiangqi", "xiangqi"],
  ["poker", "poker"], ["doudizhu", "ddz"], ["liars-dice", "liars"]];
const find = (root, id) => flatten(root).find(item => item instanceof Element && item.props.id === id);

async function renderer(name, dark = false) {
  const base = new URL(`../${name}/dev.sailry.platform/desktop/`, import.meta.url);
  const colors = { foreground: "foreground", muted_foreground: "muted", border: "border", ring: "ring" };
  const context = vm.createContext({ window: { viewport_size: () => ({ width: 960, height: 680 }) } });
  const modules = new Map();
  const bindings = {
    "gpui-kit": { div: () => new Element() },
    "gpui-base": { Button: Element },
    "gpui-component": { Button: Element, Spinner: Element, Tag: Element, ShimmerText: Element, Icon: Element },
    sailry: { Header: Element, Image: Element, theme: () => ({ colors, is_dark: dark }) },
    "sailry/ui": { Modal: Element },
  };
  function module(id) {
    if (!modules.has(id)) {
      const values = bindings[id];
      modules.set(id, values ? new vm.SyntheticModule(Object.keys(values), function () {
        for (const [key, value] of Object.entries(values)) this.setExport(key, value);
      }, { context, identifier: id }) : new vm.SourceTextModule(readFileSync(new URL(id), "utf8"),
        { context, identifier: id }));
    }
    return modules.get(id);
  }
  const entry = module(new URL("view.js", base).href);
  await entry.link((id, parent) => module(id.startsWith(".") ? new URL(id, parent.identifier).href : id));
  await entry.evaluate();
  const { messages } = await import(new URL("locales.js", base));
  const rules = await import(new URL("game.js", base));
  const actions = [];
  const view = { text: messages("en"), game: null, wins: 0, losses: 0, draws: 0, score: 0,
    loading: false, available: true, error: null, name: "Milo", names: ["You", "Milo", "Robin"],
    matchEnabled: [true, true, true], quantity: 1, face: 1, elapsed: 0, busy: false,
    selected: null, motion: {}, start: (...args) => actions.push(args) };
  return { view, rules, actions, render: () => entry.namespace.render(view, {}) };
}

for (const [name, prefix] of games) test(`${name}: lobby shares the themed stage and bounded scrolling`, async () => {
  for (const dark of [false, true]) {
    const { view, actions, render } = await renderer(name, dark);
    const root = render();
    assert.equal(root.items[0].props.id, `${prefix}-header`);
    const stage = find(root, `${prefix}-stage`);
    assert.equal(stage, root.items[1]);
    assert.equal(stage.props.min_h, 0);
    assert.equal(stage.props.min_w, 0);
    const scroll = find(stage, `${prefix}-scroll`);
    assert("overflow_x_scroll" in scroll.props && "overflow_y_scroll" in scroll.props);
    assert.equal(find(stage, `${prefix}-background`).props.opacity, dark ? 0.24 : 0.08);
    assert.equal(actions.length, 0);
    const start = flatten(root).find(item => item instanceof Element && item.props.on_click
      && (item.props.id?.includes("start") || item.props.id === "ddz-deal"));
    assert(start);
    start.props.on_click({}, {});
    assert.equal(actions.length, 1);
    view.available = false;
    assert.equal(flatten(render()).filter(item => item instanceof Element && item.props.on_click).length, 0);
  }
});

test("xiangqi: both sides share a flat strip, board status and fixed history", async () => {
  const { view, rules, render } = await renderer("xiangqi");
  for (const side of ["red", "black"]) {
    view.game = rules.newGame(side);
    const root = render();
    const strip = find(root, "xiangqi-player-strip");
    assert.equal(strip.props.h, 89);
    assert.deepEqual(strip.items.map(item => item.props.id), [`xiangqi-seat-${side}`,
      `xiangqi-seat-${side === "red" ? "black" : "red"}`]);
    for (const color of ["red", "black"]) {
      const seat = find(strip, `xiangqi-seat-${color}`);
      assert.equal(seat.props.h, 88);
      assert.equal(find(seat, `xiangqi-avatar-${color}`).props.path.endsWith(
        color === side ? "character-01.png" : "character-02.png"), true);
      assert(!("bg" in seat.props));
    }
    const region = find(root, "xiangqi-board-region");
    assert.equal(region.items[0].props.id, "xiangqi-board-wrap");
    assert.equal(region.items[1].props.id, `xiangqi-status-${side === "red" ? "yourTurn" : "paused"}`);
    assert.equal(find(root, "xiangqi-history").props.w, 200);
    assert(find(root, "xiangqi-new-game"));
    view.game.moves = Array.from({ length: 50 }, (_, index) =>
      ({ side: "red", piece: "P", fromRow: 6, fromColumn: 0, toRow: 5, toColumn: 0, index }));
    const moves = flatten(find(render(), "xiangqi-history")).filter(item =>
      item instanceof Element && /^xiangqi-move-\d+$/.test(item.props.id));
    assert.equal(moves.length, 50);
    assert.equal(moves[0].props.id, "xiangqi-move-50");
    assert.equal(moves.at(-1).props.id, "xiangqi-move-1");
  }
});

test("liars-dice: disabled seats stay absent and concealed hands reveal in order", async () => {
  const { view, rules, render } = await renderer("liars-dice");
  view.game = rules.deal(() => 0, [5, 5, 0]);
  view.matchEnabled = [true, true, false];
  let root = render();
  assert.deepEqual(find(root, "liars-player-strip").items.map(item => item.props.id),
    ["liars-seat-1", "liars-seat-0"]);
  assert(!find(root, "liars-seat-2") && !find(root, "liars-hand-2"));
  assert(find(root, "liars-hand-region").items.includes(find(root, "liars-hand-0")));
  assert.equal(find(root, "liars-history").props.w, 200);
  const hidden = player => flatten(find(root, `liars-hand-${player}`)).filter(item => item === "?").length;
  assert.equal(hidden(0), 0);
  assert.equal(hidden(1), 5);
  rules.bid(view.game, 0, 1, 1);
  rules.challenge(view.game, 1);
  view.motion = { revealSeats: 0, resultShown: false };
  root = render();
  assert.equal(hidden(1), 5);
  assert.equal(find(root, "liars-round-result").props.opacity, 0);
  view.motion = { revealSeats: 2, resultShown: true };
  root = render();
  assert.equal(hidden(1), 0);
  assert.equal(find(root, "liars-round-result").props.opacity, 1);
  assert(find(root, "liars-next"));
  assert.deepEqual(flatten(find(root, "liars-history")).filter(item => item instanceof Element
    && /^liars-move-\d+$/.test(item.props.id)).map(item => item.props.id), ["liars-move-2", "liars-move-1"]);
});
