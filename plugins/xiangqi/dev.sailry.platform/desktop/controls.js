import { theme } from "sailry";
import { div } from "gpui-kit";
import { Button, ShimmerText, Tag } from "gpui-component";
import { retryable } from "./model.js";

export function action(id, label, onClick) {
  return new Button(id).size("small").h(36).rounded_lg().min_w(96)
    .label(label).on_click((_event, cx) => onClick(cx));
}

export function emblem(text, side, diameter = 36) {
  return div().w(diameter).h(diameter).flex_shrink(0).h_flex().items_center().justify_center()
    .rounded(9999).border_2().border_color(side === "red" ? "#9e4436" : "#45413b")
    .bg("#f3dfb6").font_bold().text_color(side === "red" ? "#a3332d" : "#303338")
    .child(text.glyphs[side === "red" ? "K" : "k"]);
}

export function status(view) {
  const { game, text } = view, colors = theme().colors;
  const key = game.phase === "over" ? game.winner === null ? "draw"
    : game.winner === game.human ? "win" : "lose"
    : view.busy ? "thinking" : game.turn === game.human ? "yourTurn" : "paused";
  const line = div().id(`xiangqi-status-${key}`).h_flex().items_center().gap_2().min_h(28)
    .text_sm().font_medium().text_color(colors.foreground);
  if (key === "thinking") line.child(new ShimmerText(text.thinking).id("xiangqi-thinking").text_sm())
    .child(div().text_color(colors.muted_foreground).child(`· ${view.elapsed} ${text.seconds}`));
  else line.child(text[key]);
  if (game.phase === "play" && game.check)
    line.child(new Tag().variant("warning").size("small").child(text.check));
  if (retryable.includes(view.error)) line.child(action("xiangqi-retry", text.retry,
    cx => view.advance(cx)).outline().h(28));
  return line;
}
