import { Header, theme } from "sailry";
import { div } from "gpui-kit";
import { Spinner } from "gpui-component";
import { die, table, seat, hand } from "./table.js";
import { button, controls } from "./controls.js";
import { stage } from "./layout.js";
import { history } from "./history.js";

function lobby(view, header) {
  const { colors } = theme(), text = view.text;
  const status = view.loading ? "loading" : view.available ? "ready" : view.error || "configure";
  const hero = div().h_flex().items_center().justify_center().gap_2().w(220).h(98)
    .child(die(2)).child(die(5)).child(die(6));
  const panel = div().id(`liars-status-${status}`).v_flex().items_center().justify_between().gap_4()
    .w_full().max_w(390).min_h(350).p_6()
    .child(hero)
    .child(div().v_flex().items_center().gap_1()
      .child(div().text_3xl().font_bold().text_color(colors.foreground).child(text.title))
      .child(div().text_sm().text_color(colors.muted_foreground).child(text.subtitle)))
    .child(div().text_center().text_sm().text_color(colors.muted_foreground).child(text.rule));
  if (view.loading) panel.child(new Spinner());
  else if (view.available) panel.child(button("liars-start", text.start, cx => view.start(cx), true).w_full().h(40));
  else if (status !== "loadFailed") panel.child(div().text_sm().text_color(colors.muted_foreground).child(text[status]));
  return div().id("liars-lobby").size_full().v_flex().min_w(0).min_h(0).overflow_hidden()
    .child(header).child(stage(panel));
}

export function render(view) {
  const { colors } = theme();
  const header = Header.new("liars-header", { content: JSON.stringify({ items: [
    { label: view.text.round, value: String(view.game?.round || 0) },
    { label: view.text.dice, value: String(view.game?.counts[0] ?? 5) },
  ], actions: [{ id: "liars-reset", label: view.text.resetScore, icon: "icons/rotate-cw.svg" }] }) });
  if (!view.game) return lobby(view, header);
  const game = view.game;
  const compact = window.viewport_size().height < 760;
  const status = game.phase === "match_over" ? view.text.matchOver
    : game.phase === "round_over" ? view.text.roundOver
    : view.busy ? `${view.names[game.turn]} ${view.text.thinking}`
    : game.turn === 0 ? view.text.yourTurn : view.text.waiting;
  const statusId = game.phase === "match_over" ? "matchOver"
    : game.phase === "round_over" ? "roundOver"
    : view.busy ? "thinking"
    : game.turn === 0 ? "yourTurn" : "waiting";
  const notice = div().id(view.busy ? "liars-notice" : `liars-status-${statusId}`).text_sm().text_color(colors.muted_foreground)
    .children(view.busy ? [] : [status]);
  const players = [1, 0, 2].filter(player => view.matchEnabled[player]);
  const height = Math.max(448, Math.min(788, window.viewport_size().height - 196));
  const content = div().id("liars-content").w_full().max_w(1240).min_w(820).v_flex()
    .child(div().id("liars-player-strip").h_flex().h(89).flex_shrink(0).border_b_1().border_color(colors.border)
      .children(players.map((player, index) => seat(view, player, index === players.length - 1))))
    .child(div().h_flex().h(height).flex_shrink(0).items_stretch()
      .child(div().v_flex().h_full().flex_1().min_w(0)
        .child(table(view, compact, div().id("liars-action-region").v_flex().items_center().gap_2()
          .child(notice).child(controls(view))))
        .child(div().id("liars-hand-region").v_flex().flex_shrink(0).items_center().p_4()
          .border_t_1().border_color(colors.border).child(hand(view, 0, compact))))
      .child(history(view)));
  return div().id("liars-table").size_full().v_flex().min_w(0).min_h(0).overflow_hidden()
    .text_color(colors.foreground).child(header).child(stage(content, 820));
}
