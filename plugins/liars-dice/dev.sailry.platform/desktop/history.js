import { div } from "gpui-kit";
import { Image, theme } from "sailry";
import { portrait } from "./layout.js";

export function history(view) {
  const { game, text } = view, colors = theme().colors;
  return div().id("liars-history").v_flex().w(200).h_full().flex_shrink(0)
    .border_l_1().border_color(colors.border)
    .child(div().text_sm().font_semibold().flex_shrink(0).p_4().child(text.history))
    .child(div().v_flex().flex_1().min_h(0).overflow_y_scroll().gap_3().px_4()
      .children(game.history.length ? game.history.map((entry, index) => ({ ...entry, index })).reverse().map(entry =>
        div().id(`liars-move-${entry.index + 1}`).h_flex().flex_shrink(0).items_center().gap_3()
          .child(div().v_flex().flex_1().min_w(0).gap_1().text_sm()
            .child(div().font_medium().child(entry.kind === "bid" ? `${entry.quantity} × ${entry.face}` : text.challenge))
            .child(div().text_color(colors.muted_foreground).child(view.names[entry.player])))
          .child(div().w(28).h(28).flex_shrink(0)
            .child(Image.new(`liars-history-avatar-${entry.index + 1}`, { path: portrait(entry.player) }))))
        : [div().text_sm().text_color(colors.muted_foreground).child(text.noActions)]))
    .child(div().flex_shrink(0).p_4().border_t_1().border_color(colors.border)
      .text_xs().text_color(colors.muted_foreground).child(text.rule));
}
