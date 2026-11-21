import { Header, Image, theme } from "sailry";
import { div } from "gpui-kit";
import { Spinner, Tag } from "gpui-component";
import { board } from "./board.js";
import { action, emblem, status } from "./controls.js";
import { stage, portrait } from "./layout.js";
import { history } from "./history.js";

function lobby(view, header) {
  const { text } = view, colors = theme().colors;
  const key = view.loading ? "loading" : view.available ? "ready" : view.error || "configure";
  const card = div().id("xiangqi-lobby").v_flex().items_center().justify_between().gap_4()
    .w_full().max_w(440).min_h(340).p_6()
    .child(div().h_flex().items_center().gap_3().p_4()
      .child(emblem(text, "black", 42)).child(emblem(text, "red", 42)))
    .child(div().v_flex().items_center().gap_1().text_center()
      .child(div().text_3xl().font_bold().text_color(colors.foreground).child(text.title))
      .child(div().text_sm().text_color(colors.muted_foreground).child(text.subtitle)))
    .child(div().id(`xiangqi-status-${key}`).text_sm().text_center()
      .text_color(colors.muted_foreground)
      .child(view.loading || key === "loadFailed" ? "" : text[key]));
  if (view.loading) card.child(new Spinner());
  else if (view.available) card.child(div().h_flex().flex_wrap().justify_center().gap_3()
    .child(div().id("xiangqi-choice-red")
      .child(action("xiangqi-start-red", text.startRed, cx => view.start(cx, "red")).primary()))
    .child(div().id("xiangqi-choice-black")
      .child(action("xiangqi-start-black", text.startBlack, cx => view.start(cx, "black")).outline())));
  card.child(div().text_xs().text_center().text_color(colors.muted_foreground)
    .child(text.practiceRule));
  return div().size_full().v_flex().min_w(0).min_h(0).overflow_hidden().child(header)
    .child(stage(card));
}

function seat(view, side, first) {
  const { game, text } = view, colors = theme().colors;
  const active = game.phase === "play" && game.turn === side;
  const name = side === game.human ? text.you : view.name;
  return div().id(`xiangqi-seat-${side}`).relative().h(88).flex_1().min_w(0).px_4().py_3()
    .border_r(first ? 1 : 0).border_color(colors.border)
    .child(div().h_flex().h_full().items_center().gap_3()
      .child(div().w(40).h(40).flex_shrink(0)
        .child(Image.new(`xiangqi-avatar-${side}`, { path: portrait(view, side) })))
      .child(div().v_flex().flex_1().min_w(0).gap_1()
        .child(div().text_sm().font_semibold().truncate().child(name))
        .child(div().h_flex().items_center().gap_2().text_xs().text_color(colors.muted_foreground)
          .child(text[side])
          .child(side === "red" ? text.first : text.second))))
    .children(active ? [div().id(`xiangqi-active-${side}`).absolute().left(16).right(16).bottom(0).h(2).bg(colors.ring)] : []);
}

function table(view, header) {
  const { game, text } = view, colors = theme().colors;
  const ended = game.phase === "over";
  const outcome = game.winner === null ? "draw" : game.winner === game.human ? "win" : "lose";
  const compact = window.viewport_size().height < 760 || window.viewport_size().width < 900;
  const height = Math.max(compact ? 500 : 590, Math.min(788, window.viewport_size().height - 196));
  const region = div().id("xiangqi-board-region").v_flex().flex_1().min_w(0)
    .items_center().justify_center();
  if (compact) region.gap_2().p_3();
  else region.gap_4().p_4();
  region.child(board(view)).child(status(view));
  if (ended) region.child(div().id("xiangqi-result").relative().h_flex().items_center().gap_2()
    .top(view.motion?.resultVisible === false ? 8 : 0)
    .opacity(view.motion?.resultVisible === false ? 0 : 1)
    .transition("top", { duration: 240, easing: "ease-out" })
    .transition("opacity", { duration: 240, easing: "ease-out" })
    .child(new Tag().variant(outcome === "win" ? "success" : outcome === "lose" ? "danger" : "secondary")
      .child(text[outcome]))
    .child(div().text_sm().text_color(colors.muted_foreground).child(text[game.reason])));
  const content = div().id("xiangqi-content").w_full().max_w(1240).min_w(720).v_flex()
    .child(div().id("xiangqi-player-strip").h_flex().h(89).flex_shrink(0).border_b_1().border_color(colors.border)
      .child(seat(view, game.human, true)).child(seat(view, game.human === "red" ? "black" : "red", false)))
    .child(div().h_flex().h(height + (ended ? 40 : 0)).flex_shrink(0).items_stretch()
      .child(region).child(history(view)));
  return div().id("xiangqi-table").size_full().v_flex().min_w(0).min_h(0).overflow_hidden()
    .text_color(colors.foreground).child(header).child(stage(content, 720));
}

export function render(view) {
  const { text } = view;
  const header = Header.new("xiangqi-header", { content: JSON.stringify({ items: [
    { label: text.wins, value: String(view.wins), tone: "success" },
    { label: text.losses, value: String(view.losses), tone: "danger" },
    { label: text.draws, value: String(view.draws) },
  ], actions: view.game ? [{ id: "xiangqi-header-new-game", label: text.newGame,
    icon: "icons/rotate-cw.svg" }] : [] }) });
  return view.game ? table(view, header) : lobby(view, header);
}
