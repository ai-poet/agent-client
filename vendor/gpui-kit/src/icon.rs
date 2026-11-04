use gpui::{Hsla, Img, Rems, Styled, Svg, img, svg};

/// A monochrome icon from the caller's asset set, tinted via text color.
pub fn icon(path: &'static str, size: Rems, color: Hsla) -> Svg {
    svg()
        .path(path)
        .w(size)
        .h(size)
        .flex_none()
        .text_color(color)
}

/// A polychrome file icon rendered as an image so authored colors survive.
pub fn file_icon(path: &'static str, size: Rems) -> Img {
    img(path).w(size).h(size).flex_none()
}
