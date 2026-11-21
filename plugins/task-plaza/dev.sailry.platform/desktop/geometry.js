// Every scene layer uses the same logical plane and uniform display scale.
export const WORLD_WIDTH = 1120;
export const WORLD_HEIGHT = 900;
export const PERSON_SIZE = 60;
export const DESK_WIDTH = 62;
export const DESK_HEIGHT = DESK_WIDTH * 185 / 216;
const cells = [[0,0],[1,0],[2,0],[0,1],[2,1],[0,2],[1,2],[2,2]];
export const ROOMS = cells.map(([column,row]) => {
  const bounds = {x:20 + column * 380,y:60 + row * 300,width:320,height:240};
  return {bounds,board:{x:bounds.x + 20,y:bounds.y - 52,width:280,height:88}};
});
export function seatPosition(room,index) {
  const {bounds} = ROOMS[room];
  // Seats keep clear of the whiteboard above and the pager on the bottom edge.
  return {x:bounds.x + 68 + index % 3 * 92,y:bounds.y + 140 + Math.floor(index / 3) * 68};
}
export function approvalPosition(index,count = 6) {
  return {x:465 + index % 3 * 95,y:(count <= 3 ? 485 : 440) + Math.floor(index / 3) * 81};
}
export const ATRIUM = {x:400,y:340,width:320,height:220};

// Source crops retain the original alpha and aspect ratios of the worker atlas.
export const SPRITES = [
  [53,81,216,185],[366,81,216,185],[708,128,150,167],
  [1020,40,141,258],[94,357,140,258],[402,357,140,258],
  [714,359,140,256],[1023,359,139,256],
  [104,661,160,258],[413,665,168,254],[723,663,167,256],
  [1029,661,168,258],[106,968,161,257],[415,971,160,254],
  [722,957,163,269],[1025,981,137,251],
];
export const SEATED = [3,4,5,6,7,15];
export const STANDING = [9,11,12,10,13,8];
