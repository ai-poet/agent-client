import {View, div} from "gpui-kit";
import {Attachment, Icon} from "gpui-component";
import {Action, state, theme, nextChange, status, toast} from "sailry/card";
import {messages} from "./locales.js";

export default class Connection extends View {
  init(_props, cx) {
    this.state = state(); this.loading = false; this.action = null;
    this.text = messages(this.state.locale);
    cx.spawn(async cx => {
      let cursor = this.state.cursor;
      try {
        while (true) {
          const update = await nextChange(cursor); cursor = update.cursor;
          this.state = update;
          if (update.action && update.action !== this.action) {
            this.action = update.action; this.refresh(cx);
          }
          cx.notify();
        }
      } catch (_) { /* Native release ends this instance's observation. */ }
    });
    this.refresh(cx);
  }
  refresh(cx) {
    if (this.loading) return;
    this.loading = true; cx.notify();
    cx.spawn(async cx => {
      try { await status(); }
      catch (_) { toast(this.text.failed, true); }
      finally { this.loading = false; cx.notify(); }
    });
  }
  render(_cx) {
    const colors = theme().colors, text = this.text, state = this.state;
    const busy = this.loading || ["starting", "exchanging"].includes(state.login);
    const pending = state.login === "pending";
    const unknown = state.login === "unconfirmed" || (state.login === "failed" && state.can_cancel);
    const statusLabel = state.configured ? text.connected
      : ["starting", "pending", "exchanging"].includes(state.login) ? text[state.login] : text.connect;
    return new Attachment("notion-connection-card").w_full().h_auto().min_w_0().rounded_2xl().py_3()
      .child(div().h_flex().w_full().min_w_0().gap_3().items_center()
        .child(div().p_2().rounded_lg().bg(colors.muted)
          .child(new Icon("reicon:files/file-text").size("large")))
        .child(div().v_flex().flex_1().min_w_0().gap_1()
          .child(div().font_medium().child(text.title))
          .child(div().text_sm().text_color(colors.muted_foreground).child(statusLabel)))
        .child(div().h_flex().gap_1().flex_wrap()
        .children(!state.configured && !pending ? [
          Action.new("notion-login", {label:unknown ? text.check : text.login, icon:unknown ? "icons/refresh-cw.svg" : "reicon:arrows/arrow-up-right", disabled:busy,
            action:unknown ? {kind:"connect"} : {kind:"connect",target:"login",mode:"embedded"}}),
          ...(!unknown ? [Action.new("notion-login-external", {label:text.external,icon:"icons/external-link.svg",disabled:busy,
            action:{kind:"connect",target:"login",mode:"external"}})] : []),
        ] : [])
        .children(pending ? [
          Action.new("notion-open-login", {label:text.open,icon:"reicon:arrows/arrow-up-right",disabled:busy || !state.can_open,action:{kind:"browser",target:"login",mode:"embedded"}}),
          Action.new("notion-open-external", {label:text.openExternal,icon:"icons/external-link.svg",disabled:busy || !state.can_open,action:{kind:"browser",target:"login",mode:"external"}}),
        ] : [])
        .children(state.can_cancel ? [Action.new("notion-cancel-login", {label:text.cancel,icon:"icons/close.svg",disabled:busy,action:{kind:"cancel"}})] : [])
        .child(Action.new("notion-refresh", {label:text.refresh,icon:"icons/refresh-cw.svg",disabled:busy,action:{kind:"refresh"}}))));
  }
}
