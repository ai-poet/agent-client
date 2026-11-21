// The package owns review drafts and submission; native services retain the capture.
import {div} from 'gpui-kit';
import {Button} from 'gpui-component';
import {BrowserAnnotationImage, theme} from 'sailry';
import {Modal, submitBrowserAnnotation} from 'sailry/ui';
import {createText, readText, releaseText, TextField} from 'sailry/forms';

export function reference(element, note) {
  let length = 0;
  for (const character of note) {
    const value = character.codePointAt(0);
    length += value < 0x80 ? 1 : value < 0x800 ? 2 : value < 0x10000 ? 3 : 4;
  }
  if (length > 4096 || note.includes('\0')) throw new Error('browser_annotation_invalid');
  return JSON.stringify({page_element:element,annotation:note}, null, 2);
}

export class Review {
  constructor(view) { this.view = view; this.current = null; this.pending = false; }
  clear() {
    if (this.current) releaseText(this.current.note);
    this.current = null; this.pending = false;
  }
  accept(capture) {
    if (capture?.token === this.current?.token) return;
    this.clear();
    if (capture) this.current = {...capture,title:this.view.selected()?.title ?? '',note:createText('',{
      multiline:true,rows:[2,5],placeholder:this.view.text.browser_annotation_note,
      label:this.view.text.browser_annotation_note
    })};
  }
  close(cx) {
    const token = this.current?.token;
    this.clear(); cx.notify();
    if (token) this.view.perform('cancel_annotation',{token},cx);
  }
  submit(cx) {
    const current = this.current;
    if (!current || this.pending || this.view.state.annotation?.token !== current.token) return;
    let value;
    try { value = reference(current.element,readText(current.note)); }
    catch (error) { this.view.report(error); return; }
    this.pending = true; cx.notify();
    cx.spawn(async cx => {
      try {
        await submitBrowserAnnotation(current.token,value);
        if (this.current === current) this.close(cx);
      } catch (error) {
        if (this.current === current) this.view.report(error,'browser_annotation_stale');
      } finally {
        if (this.current === current) this.pending = false;
        cx.notify();
      }
    });
  }
}

export function renderReview(view, cx) {
  const review = view.review, current = review.current, text = view.text, colors = theme().colors;
  return Modal.new('browser-annotation-dialog',{
    open:!!current,form:true,title:text.browser_annotation,width:560
  }).children(current ? [div().v_flex().w_full().min_w_0().gap_3()
    .child(div().id('browser-annotation-preview').w_full().max_h(240).rounded_lg().overflow_hidden()
      .child(BrowserAnnotationImage.new('browser-annotation-image',{token:current.token})))
    .child(div().id('browser-annotation-selector').w_full().min_w_0().text_sm().truncate()
      .font_family(cx.theme().typography.mono).text_color(colors.muted_foreground)
      .tooltip([current.title || current.element.title,current.element.url].filter(Boolean).join('\n'))
      .child(current.element.selector))
    .children(current.element.component ? [div().id('browser-annotation-component').w_full().min_w_0()
      .text_sm().truncate().child([current.element.component.name,current.element.component.source]
        .filter(Boolean).join(' · '))] : [])
    .children(current.element.text.trim() && current.element.text.trim() !== current.title.trim() ? [
      div().id('browser-annotation-text').max_h(100).overflow_y_scroll().child(current.element.text)
    ] : [])
    .children(current.element.truncated ? [div().text_sm().text_color(colors.muted_foreground)
      .child(text.browser_annotation_truncated)] : [])
    .child(div().id('browser-annotation-note').max_h(100).overflow_y_scroll()
      .child(TextField.new(current.note))),
    div().h_flex().justify_end().gap_2()
      .child(div().id('browser-annotation-cancel').child(new Button('browser-annotation-cancel')
        .label(text.settings_cancel).on_click((_,cx) => review.close(cx))))
      .child(div().id('browser-annotation-add').child(new Button('browser-annotation-add').primary()
        .label(text.browser_annotation_add).disabled(review.pending).loading(review.pending)
        .on_click((_,cx) => review.submit(cx))))] : []);
}
