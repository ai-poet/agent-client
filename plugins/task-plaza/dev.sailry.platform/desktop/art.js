import {div} from 'gpui-kit';
import {Image} from 'sailry';
import {SPRITES} from './geometry.js';
export const WORKERS = 'dev.sailry.platform/desktop/assets/workers.png';

// Furniture and people are the only image layers. Structural layout and all
// controls use native Kit elements.
export function crop(id,path,sheet,source,width,height) {
  const [x,y,w,h] = source, sx = width / w, sy = height / h;
  return div().id(`${id}-clip`).relative().w(width).h(height).overflow_hidden()
    .child(div().absolute().left(-x * sx).top(-y * sy)
      .w(sheet[0] * sx).h(sheet[1] * sy)
      .child(Image.new(`${id}-art`,{path,fit:'fill',width:sheet[0] * sx,height:sheet[1] * sy})));
}
export function sprite(id,index,height,width = null) {
  const source = SPRITES[index];width ??= source[2] / source[3] * height;
  return crop(id,WORKERS,[1254,1254],source,width,height);
}
