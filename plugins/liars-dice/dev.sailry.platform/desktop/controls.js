import { theme } from "sailry";
import { div } from "gpui-kit";
import { Button, ShimmerText } from "gpui-component";
import { legalBid } from "./game.js";
import { retryable } from "./model.js";

export function button(id, label, action, primary = false) {
  const control = new Button(id).size("small").h(34).rounded_lg().min_w(64)
    .on_click((_event, cx) => action(cx)).label(label);
  return primary ? control.primary() : control.outline();
}

export function controls(view) {
  const { game, text } = view, { colors } = theme();
  const controls = div().id("liars-controls").v_flex().items_center().gap_2().w_full().max_w(920).mx_auto();
  if (retryable.includes(view.error)) controls.child(button("liars-retry", text.retry, cx => view.advance(cx)));
  if (game.phase !== "bid") {
    const winner = game.result.winner;
    if (winner !== null) controls.child(div().id("liars-match-winner").text_lg().font_bold()
      .text_color(colors.foreground).child(text.winner(view.names[winner])));
    return controls.child(div().id("liars-action-next")
      .child(button(game.phase === "match_over" ? "liars-again" : "liars-next",
        game.phase === "match_over" ? text.again : text.next,
        cx => game.phase === "match_over" ? view.reset(cx) : view.next(cx), true)));
  }
  if (view.busy) return controls.child(div().id("liars-status-thinking").h_flex().items_center().gap_2()
    .child(new ShimmerText(`${view.names[game.turn]} ${text.thinking}`).text_sm())
    .child(div().text_xs().text_color(colors.muted_foreground).child(`${view.elapsed} ${text.seconds}`)));
  if (game.turn !== 0) return controls;
  const total = game.counts.reduce((sum, count) => sum + count, 0);
  const quantity = div().id("liars-quantity").h_flex().items_center().gap_2()
    .child(div().text_sm().text_color(colors.muted_foreground).child(text.quantity))
    .child(button("liars-quantity-down", text.decrease, cx => view.setQuantity(view.quantity - 1, cx))
      .disabled(view.quantity <= 1))
    .child(div().min_w(28).text_center().font_bold().child(String(view.quantity)))
    .child(button("liars-quantity-up", text.increase, cx => view.setQuantity(view.quantity + 1, cx))
      .disabled(view.quantity >= total));
  const faces = div().id("liars-faces").h_flex().items_center().justify_center().flex_wrap().gap_1()
    .child(div().text_sm().text_color(colors.muted_foreground).child(text.face))
    .children(Array.from({ length: 6 }, (_, index) => button(`liars-face-${index + 1}`,
      String(index + 1), cx => view.setFace(index + 1, cx), view.face === index + 1)));
  const actions = div().h_flex().items_center().justify_center().flex_wrap().gap_2()
    .child(div().id("liars-action-bid")
      .child(button("liars-bid", text.bid, cx => view.act("bid", cx), true)
        .disabled(!legalBid(game, view.quantity, view.face))));
  if (game.currentBid) actions.child(div().id("liars-action-challenge")
    .child(button("liars-challenge", text.challenge, cx => view.act("challenge", cx))));
  return controls.child(quantity).child(faces).child(actions);
}
