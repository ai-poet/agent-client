import {deflateSync,inflateSync} from 'node:zlib';
import assert from 'node:assert/strict';

// A valid deterministic 512×306 RGB PNG, comparable to the reported 470 KB JPEG.
const width=512,height=306,scanlines=Buffer.alloc((width*3+1)*height);
let random=0x13579bdf;
for(let row=0;row<height;row++) {
  const start=row*(width*3+1);
  for(let column=1;column<=width*3;column++) {
    random^=random<<13;random^=random>>>17;random^=random<<5;
    scanlines[start+column]=random&255;
  }
}
function crc(bytes) {
  let value=0xffffffff;
  for(const byte of bytes) {
    value^=byte;
    for(let bit=0;bit<8;bit++)value=(value>>>1)^((value&1)?0xedb88320:0);
  }
  return (value^0xffffffff)>>>0;
}
function chunk(name,data) {
  const type=Buffer.from(name),header=Buffer.alloc(4),tail=Buffer.alloc(4);
  header.writeUInt32BE(data.length);tail.writeUInt32BE(crc(Buffer.concat([type,data])));
  return Buffer.concat([header,type,data,tail]);
}
const header=Buffer.alloc(13);header.writeUInt32BE(width);header.writeUInt32BE(height,4);header[8]=8;header[9]=2;
const compressed=deflateSync(scanlines,{level:0});
assert.deepEqual(inflateSync(compressed),scanlines);
export const image=Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),
  chunk('IHDR',header),chunk('IDAT',compressed),chunk('IEND',Buffer.alloc(0))]);
assert.ok(image.length>=470176 && image.length<480000);
export const inline={kind:'image',media_type:'image/png',data:{kind:'inline',data_base64:image.toString('base64')}};
