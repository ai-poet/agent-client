use gpui::{
    Context, Div, InteractiveElement, KeyDownEvent, Stateful, StatefulInteractiveElement, Window,
};

/// Add conventional mouse and keyboard activation to a focusable element.
pub trait ActivationExt: Sized {
    fn on_activation<E>(
        self,
        cx: &mut Context<E>,
        activate: impl Fn(&mut E, &mut Window, &mut Context<E>) + 'static,
    ) -> Self
    where
        E: 'static;
}

impl ActivationExt for Stateful<Div> {
    fn on_activation<E>(
        self,
        cx: &mut Context<E>,
        activate: impl Fn(&mut E, &mut Window, &mut Context<E>) + 'static,
    ) -> Self
    where
        E: 'static,
    {
        let activate = std::rc::Rc::new(activate);
        let click_activate = activate.clone();
        let key_activate = activate;
        self.on_click(cx.listener(move |this, _, window, cx| {
            click_activate(this, window, cx);
            cx.stop_propagation();
        }))
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
            // Modified chords belong to the command that owns them.
            if !event.keystroke.modifiers.modified()
                && matches!(event.keystroke.key.as_str(), "enter" | "space")
            {
                key_activate(this, window, cx);
                cx.stop_propagation();
            }
        }))
    }
}
