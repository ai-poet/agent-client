import { div } from "gpui-kit";
import { Image, theme } from "sailry";
import { portrait } from "./layout.js";

const pips = {
  1: [[1, 1]],
  2: [[0, 0], [2, 2]],
  3: [[0, 0], [1, 1], [2, 2]],
  4: [[0, 0], [0, 2], [2, 0], [2, 2]],
  5: [[0, 0], [0, 2], [1, 1], [2, 0], [2, 2]],
  6: [[0, 0], [0, 1], [0, 2], [2, 0], [2, 1], [2, 2]],
};

// Kit has no dice-face component. These pips are game material; ordinary
// actions, labels and status controls use Kit components in the view.
export function die(face, compact = false, hidden = false) {
  const size = compact ? 32 : 38;
  const dot = compact ? 5 : 6;
  const base = compact ? 5 : 6;
  const step = compact ? 8.5 : 10;
  const tile = div().relative().w(size).h(size).flex_shrink(0).rounded_md().border_1()
    .border_color(hidden ? "#86b6a4" : "#d9cfb5")
    .bg(hidden ? "#1a5260" : "#f8f2df").shadow_sm();
  if (hidden) return tile.h_flex().items_center().justify_center()
    .text_sm().font_bold().text_color("#cde5d9").child("?");
  return tile.children((pips[face] || []).map(([row, column]) =>
    div().absolute().left(base + column * step).top(base + row * step)
      .w(dot).h(dot).rounded(9999).bg("#364a42")));
}

export function hand(view, player, compact) {
  const game = view.game;
  const rolling = view.motion?.rolling === true;
  const hidden = view.motion?.handShown === false || player !== 0
    && (game.phase === "bid" || game.result && (view.motion?.revealSeats ?? 2) < player);
  return div().id(`liars-hand-${player}`).h_flex().justify_center().gap(compact ? 4 : 6)
    .children(game.hands[player].map((face, index) => div().id(`liars-die-${player}-${index}`).relative()
      .transition("opacity", { duration: 210, easing: "ease-out" })
      .transition("top", { duration: 210, easing: "ease-out" })
      .opacity(rolling ? 0.45 : 1).top(rolling ? 14 : 0)
      .child(die(face, compact, hidden))));
}

export function seat(view, player, last) {
  const { game, text } = view, colors = theme().colors;
  const active = game.phase === "bid" && game.turn === player;
  return div().id(`liars-seat-${player}`).relative().h(88).flex_1().min_w(0).px_4().py_3()
    .border_r(last ? 0 : 1).border_color(colors.border)
    .child(div().h_flex().h_full().items_center().gap_3()
      .child(div().w(40).h(40).flex_shrink(0)
        .child(Image.new(`liars-avatar-${player}`, { path: portrait(player) })))
      .child(div().v_flex().flex_1().min_w(0).gap_1()
        .child(div().text_sm().font_semibold().truncate().child(view.names[player]))
        .child(div().text_xs().text_color(colors.muted_foreground)
          .child(game.counts[player] ? `${game.counts[player]} ${text.dice}` : text.eliminated))))
    .children(active ? [div().id(`liars-active-${player}`).absolute().left(16).right(16).bottom(0).h(2).bg(colors.ring)] : []);
}

export function table(view, compact = false, actions) {
  const { game, text } = view, colors = theme().colors;
  const current = game.currentBid;
  const bid = div().id("liars-current-bid").relative().v_flex().items_center().gap_1()
    .transition("top", { duration: 220, easing: "ease-out" })
    .top(view.motion?.bidPulse ? -4 : 0)
    .child(div().text_xs().text_color(colors.muted_foreground).child(text.currentBid))
    .child(current ? div().h_flex().items_center().gap_2()
      .child(div().text_2xl().font_bold().text_color(colors.foreground).child(`${current.quantity} ×`))
      .child(die(current.face, compact))
      .child(div().text_sm().text_color(colors.foreground).child(view.names[current.player]))
      : div().text_lg().text_color(colors.foreground).child(text.noBid));
  const center = div().v_flex().items_center().gap_2().child(bid);
  if (game.result) center.child(div().id("liars-round-result").relative().v_flex().items_center().gap_1()
    .transition("opacity", { duration: 240, easing: "ease-out" })
    .transition("top", { duration: 240, easing: "ease-out" })
    .opacity(view.motion?.resultShown === false ? 0 : 1)
    .top(view.motion?.resultShown === false ? 10 : 0)
    .child(div().text_sm().font_bold().text_color(colors.foreground)
      .child(game.result.actual >= game.result.quantity ? text.bidHeld : text.bluffCaught))
    .child(div().text_xs().text_color(colors.foreground)
      .child(`${text.counted(game.result.actual)} · ${text.loses(view.names[game.result.loser])}`)));
  return div().id("liars-felt").v_flex().flex_1().min_h(0).w_full().p_4()
    .items_center().justify_between().gap_4()
    .child(div().h_flex().items_center().justify_between().w_full().gap_4()
      .children([1, 2].filter(player => view.matchEnabled[player]).map(player =>
        div().h_flex().items_center().gap_2()
          .child(div().w(28).h(28).flex_shrink(0).accessibility_label(view.names[player])
            .child(Image.new(`liars-hand-avatar-${player}`, { path: portrait(player) })))
          .child(hand(view, player, compact)))))
    .child(center).child(actions);
}
