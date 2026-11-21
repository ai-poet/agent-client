import {entries, rules} from './rules.js';

function expression(pattern) {
  let source = '^';
  for (let index = 0; index < pattern.length; index++) {
    const character = pattern[index];
    if (character === '*' && pattern[index+1] === '*') {
      index++;
      if (pattern[index+1] === '/') { source += '(?:.*/)?'; index++; }
      else source += '.*';
    } else if (character === '*') source += '[^/]*';
    else if (character === '?') source += '[^/]';
    else if (character === '{') source += '(?:';
    else if (character === '}') source += ')';
    else if (character === ',') source += '|';
    else source += '\\^$+?.()|[]'.includes(character) ? '\\'+character : character;
  }
  return new RegExp(source+'$', 'i');
}
const patterns = entries.map(entry => ({...entry,match:expression(entry.pattern)}));
const objc = /^(?:#import|#include|#pragma|#if|#define|@import|@interface|@implementation|@class|@protocol|\/\/|\/\*)/;
export function rule(path, firstLine) {
  let selected = patterns.find(entry => entry.match.test(path))?.rule ?? 'default.md';
  if (selected === 'matlab.md' && typeof firstLine === 'string' && objc.test(firstLine.trimStart())) selected = 'objc.md';
  return {file:selected,...rules[selected]};
}
