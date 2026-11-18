import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../dev.sailry.platform/document/forms.js',import.meta.url),'utf8'),window = {};
const document = {createElement(tagName) {return {tagName,children:[],append(...values) {this.children.push(...values);}};}};
vm.runInNewContext(source,{window,document,TextEncoder});
const {collect,readonly,snapshot,restore} = window.sailryForms;
const plain = value => JSON.parse(JSON.stringify(value));
function control(name,type,value,label,checked = false) {
  return {tagName:type === 'textarea' ? 'TEXTAREA' : 'INPUT',name,type,value,checked,disabled:false,
    closest() {return {textContent:label};}};
}
function field(name,label,controls,required = false) {
  return {dataset:{name},hasAttribute() {return required;},querySelector() {return {textContent:label};},
    querySelectorAll() {return controls;}};
}
function form(fields,name = 'decisions') {return {dataset:{name},querySelectorAll() {return fields;}};}

test('selected values and free text preserve Unicode without executable content',() => {
  const fields = [field('approach','Approach',[control('approach','radio','a','A',true),control('approach','radio','b','B')],true),
    field('note','Context',[control('note','textarea','  中文 🙂 <script>literal</script>  ','')])];
  assert.deepEqual(plain(collect(form(fields))),{form:'decisions',fields:[
    {name:'approach',label:'Approach',values:[{value:'a',label:'A'}]},
    {name:'note',label:'Context',values:[{value:'  中文 🙂 <script>literal</script>  ',label:'  中文 🙂 <script>literal</script>  '}]},
  ]});
});
test('required empty choices identify the field without submitting answers',() => {
  const missing = field('approach','Approach',[control('approach','radio','a','A')],true);
  const result = collect(form([missing]));assert.equal(result.missing,missing);assert.equal(result.form,'decisions');
  assert.equal(result.fields,undefined);
});
test('names controls and payloads remain bounded',() => {
  const valid = () => field('note','Context',[control('note','textarea','answer','')]);
  for (const value of [form([valid()],'invalid name'),form([valid(),valid()]),
    form([field('note','Context',[control('different','text','A','')])]),
    form([field('note','Context',[control('note','file','A','')])]),
    form([field('note','Context',[control('note','textarea','中'.repeat(1400),'')])]),
    form(Array.from({length:33},(_,index) => field(`item${index}`,'Label',[control(`item${index}`,'text','A','')]))),
    form(Array.from({length:8},(_,index) => field(`item${index}`,'Label',[control(`item${index}`,'textarea','a'.repeat(4096),'')]))),
  ]) assert.throws(() => collect(value));
  const disabled = valid();disabled.querySelectorAll()[0].disabled = true;assert.throws(() => collect(form([disabled])));
});
test('select choices use declared labels and values',() => {
  const item = {tagName:'SELECT',name:'choice',disabled:false,selectedOptions:[{value:'a',textContent:' Alpha '},{value:'b',textContent:'Beta'}]};
  assert.deepEqual(plain(collect(form([field('choice','Choice',[item])])).fields[0].values),[
    {value:'a',label:'Alpha'},{value:'b',label:'Beta'},
  ]);
});
test('duplicate form targets cannot submit ambiguous answers',() => {
  const first = form([field('note','Context',[control('note','text','Answer','')])]);
  const second = form([field('note','Context',[control('note','text','Other','')])]);
  first.ownerDocument = {querySelectorAll() {return [first,second];}};
  assert.throws(() => collect(first),/Duplicate form name/);
  second.dataset.name = 'other';assert.equal(collect(first).form,'decisions');
});
test('readonly export contains static choice states and answers',() => {
  const result = readonly(form([field('choice','Choice',[control('choice','checkbox','a','Alpha',true),control('choice','checkbox','b','Beta')]),
    field('note','Context',[control('note','textarea','Literal <answer>','')])]));
  assert.equal(result.tagName,'section');assert.equal(result.children[1].children[0].textContent,'☑ Alpha');
  assert.equal(result.children[1].children[1].textContent,'☐ Beta');assert.equal(result.children[3].children[0].textContent,'Literal <answer>');
  assert.doesNotMatch(source,/fetch\(|XMLHttpRequest|eval\(|new Function|innerHTML/);
});

test('drafts retain unsubmitted choices and exact free text across mounts',() => {
  const choices = [control('choice','checkbox','a','Alpha',true),control('choice','checkbox','b','Beta')];
  const note = control('note','textarea','  中文 🙂  ','');
  const original = form([field('choice','Choice',choices),field('note','Context',[note])]);
  const saved = plain(snapshot(original));
  choices[0].checked = false;choices[1].checked = true;note.value = '';
  assert.equal(restore(original,saved),true);assert.equal(choices[0].checked,true);assert.equal(choices[1].checked,false);assert.equal(note.value,'  中文 🙂  ');
  assert.equal(restore(original,{...saved,form:'another'}),false);
});

test('drafts preserve empty required fields and reject oversized values',() => {
  const choice = control('choice','radio','a','Alpha'),note = control('note','text','','');
  const original = form([field('choice','Choice',[choice],true),field('note','Context',[note])]);
  assert.deepEqual(plain(snapshot(original)),{form:'decisions',fields:[{name:'choice',values:[]},{name:'note',values:['']}]});
  assert.equal(restore(original,{form:'decisions',fields:[{name:'note',values:['a'.repeat(4097)]}]}),false);
  note.value = 'a'.repeat(4097);assert.throws(() => snapshot(original));
});
