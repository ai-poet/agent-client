import {div,PathBuilder,Background} from 'gpui-kit';
import {theme} from 'sailry';

// One native path repeats the watermark without retaining hundreds of image tiles.
export function ground(width,height) {
  const {colors,is_dark} = theme(), grid = PathBuilder.stroke(0.5), marks = PathBuilder.stroke(1).dash_array([4,28]);
  const tile = 32;
  const start = tile / 2 - 2, right = start + Math.ceil(width / tile) * tile, bottom = start + Math.ceil(height / tile) * tile;
  for (let x = 0;x <= width;x += tile) grid.move_to(x,0).line_to(x,height);
  for (let y = 0;y <= height;y += tile) grid.move_to(0,y).line_to(width,y);
  // Native dashes repeat each cross arm without a host call for every tile.
  for (let y = tile / 2;y < height;y += tile) marks.move_to(start,y).line_to(right,y);
  for (let x = tile / 2;x < width;x += tile) marks.move_to(x,start).line_to(x,bottom);
  return div().id('plaza-ground').absolute().inset(0).overflow_hidden()
    .child(window.paint_path(grid.build(),Background.solid(colors.foreground).opacity(is_dark ? 0.025 : 0.035)).w(width).h(height))
    .child(window.paint_path(marks.build(),Background.solid(colors.foreground).opacity(is_dark ? 0.065 : 0.08)).absolute().inset(0).w(width).h(height));
}
