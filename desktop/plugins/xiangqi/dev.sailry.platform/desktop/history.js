import { div } from "gpui-kit";
import { Image, theme } from "sailry";
import { coordinate } from "./game.js";
import { portrait } from "./layout.js";
import { action } from "./controls.js";

export function history(view) {
  const { game, text } = view, colors = theme().colors;
  return div().id("xiangqi-history").v_flex().w(200).h_full().flex_shrink(0)
    .border_l_1().border_color(colors.border)
    .child(div().h_flex().flex_shrink(0).items_center().justify_between().p_4()
      .child(div().text_sm().font_semibold().child(text.moves))
      .child(div().text_xs().text_color(colors.muted_foreground).child(String(game.moves.length))))
    .child(div().v_flex().flex_1().min_h(0).overflow_y_scroll().gap_3().px_4()
      .children(game.moves.length ? game.moves.map((move, index) => ({ ...move, index })).reverse().map(move =>
        div().id(`xiangqi-move-${move.index + 1}`).h_flex().flex_shrink(0).items_center().gap_3()
          .child(div().v_flex().flex_1().min_w(0).gap_1().text_sm()
            .child(div().font_medium().child(`${coordinate(move.fromRow, move.fromColumn)}–${coordinate(move.toRow, move.toColumn)}`))
            .child(div().text_color(colors.muted_foreground).child(text.glyphs[move.piece])))
          .child(div().w(28).h(28).flex_shrink(0)
            .accessibility_label(move.side === game.human ? text.you : view.name)
            .child(Image.new(`xiangqi-history-avatar-${move.index + 1}`, { path: portrait(view, move.side) }))))
        : [div().text_sm().text_color(colors.muted_foreground).child(text.noMoves)]))
    .child(div().v_flex().flex_shrink(0).gap_3().p_4().border_t_1().border_color(colors.border)
      .child(div().text_xs().text_color(colors.muted_foreground).child(text.practiceRule))
      .child(div().id("xiangqi-new-game-action")
        .child(action("xiangqi-new-game", game.phase === "over" ? text.again : text.newGame,
          cx => view.start(cx, game.human)).outline().w_full())));
}
