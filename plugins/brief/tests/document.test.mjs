import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../dev.sailry.platform/document/brief.js',import.meta.url),'utf8');

test('keeps HTML general without promising forms',() => {
  const manifest = JSON.parse(readFileSync(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  const skill = readFileSync(new URL('../skills/brief/SKILL.md',import.meta.url),'utf8');
  assert.equal(manifest.name,'brief');
  assert.equal(manifest.version,'0.1.12');
  assert.equal(extension.api_version,'v1');
  assert.equal(extension.display.label,'HTML');
  assert(Object.values(extension.display.locales).every(label => label === 'HTML'));
  assert.match(extension.instructions,/explicit HTML or HTML-preview request authorizes generation directly/);
  assert.match(extension.instructions,/cannot collect answers; use ask_user/);
  assert.doesNotMatch(extension.instructions,/Forms? (is|are) (optional|supported)|supported in document mode/);
  assert.match(skill,/kind=document.*default/);
  assert.match(skill,/kind=preview/);
  assert.match(skill,/a form in it submits nothing/);
  assert.doesNotMatch(skill,/optional forms|Forms? (is|are) optional|class="brief-form"/);
  for (const declaration of [extension.instructions,skill.replace(/\s+/g,' ')]) {
    assert.match(declaration,/full ordinary plan.*conversation first/);
    assert.match(declaration,/exactly one concise offer/);
    assert.match(declaration,/wait for the user's choice/);
    assert.match(declaration,/Do not repeat the offer for that plan/);
    assert.match(declaration,/Do not offer HTML for pure questions or simple changes/);
    assert.match(declaration,/never generate a document or plan HTML version automatically without acceptance/);
    assert.match(declaration,/Without a request, you may create one small `?kind=preview`? visual/);
    assert.match(declaration,/clearly helps the current answer more than text/);
  }
  assert.deepEqual(extension.tools.slice(0,3).map(tool => tool.name),[
    'write_brief','read_brief','list_briefs',
  ]);
});

test('previews follow the host theme and card layout',() => {
  const manifest = JSON.parse(readFileSync(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  const skill = readFileSync(new URL('../skills/brief/SKILL.md',import.meta.url),'utf8').replace(/\s+/g,' ');
  assert.match(extension.instructions,/keep the body background transparent, center the main element, use the host's --sailry-\* theme variables with fallbacks/);
  assert.match(skill,/Leave the `html` and `body` backgrounds transparent/);
  assert.match(skill,/color: var\(--sailry-foreground, CanvasText\)/);
  for (const name of ['background','foreground','muted','muted-foreground','primary','primary-foreground',
    'secondary','secondary-foreground','accent','border','success','warning','danger','chart-1','chart-5',
    'font-family','mono-font-family','font-size','radius']) {
    assert(skill.includes(`--sailry-${name}`),`Missing theme variable: ${name}`);
  }
  assert.match(skill,/Create at most one per answer/);
  const declaration = extension.tools.find(tool => tool.name === 'write_brief');
  assert.match(declaration.description,/one small preview visual that clearly helps the current answer/);
});

test('requested file paths are distinct from session artifact delivery',() => {
  const manifest = JSON.parse(readFileSync(new URL('../plugin.json',import.meta.url),'utf8'));
  const extension = manifest.extensions['dev.sailry.platform'];
  const skill = readFileSync(new URL('../skills/brief/SKILL.md',import.meta.url),'utf8');
  assert.match(extension.instructions,/explicitly requested worktree file path/);
  assert.match(extension.instructions,/create and verify that real file with available file or command tools/);
  assert.match(extension.instructions,/do not substitute write_brief or create an additional artifact unless separately requested/);
  assert.match(extension.instructions,/If writing is unavailable, report the limitation/);
  assert.match(extension.instructions,/returned card already delivers it/);
  assert.match(extension.instructions,/Do not invent a file path or duplicate that card/);
  assert.match(skill,/Preserve the current work mode and\npermissions/);
  assert.match(skill,/generating HTML does not grant filesystem access/);
  assert.match(skill,/returned card already delivers the artifact/);
  const declaration = extension.tools.find(tool => tool.name === 'write_brief');
  assert.match(declaration.description,/only a session-owned artifact, not a worktree file/);
  assert.match(declaration.description,/use available file or command tools/);
  assert.match(declaration.description,/do not invent a filepath or duplicate it/);
});

test('neutral styling retains the host semantic palette',() => {
  const css = readFileSync(new URL('../dev.sailry.platform/document/brief.css',import.meta.url),'utf8');
  const supplied = new Set(['background','foreground','primary','primary-foreground','secondary',
    'secondary-foreground','muted','muted-foreground','accent','border','success','warning','danger',
    'font-family','font-size','mono-font-family','radius'].map(name => `--document-${name}`));
  const derived = new Set([...css.matchAll(/(--document-[a-z-]+)\s*:/g)].map(match => match[1]));
  for (const name of derived) assert(!supplied.has(name),`Host token was overridden: ${name}`);
  for (const [,name] of css.matchAll(/var\((--document-[a-z-]+)\)/g)) {
    assert(supplied.has(name) || derived.has(name),`Unknown theme token: ${name}`);
  }
  assert.doesNotMatch(css,/:\s*(?:#[0-9a-f]{3,8}\b|(?:oklch|rgba?|hsla?)\(|(?:white|black)\b)/i);
  assert.doesNotMatch(css,/@import|url\(/);
  assert.match(css,/a:focus-visible[^{}]*summary:focus-visible\s*\{[^}]*outline:/);
  assert.match(css,/--document-radius-(?:sm|lg):\s*calc\(var\(--document-radius\)/);
  assert.match(css,/--document-action:\s*var\(--document-foreground\)/);
  assert.match(css,/--document-action-foreground:\s*var\(--document-background\)/);
  assert.match(css,/body\s*\{[^}]*font-size:\s*var\(--document-font-size\)/);
  assert.doesNotMatch(css,/var\(--document-(?:primary|accent)\)/);
  // Only the contents dropdown casts a shadow, derived from the foreground token.
  for (const [,rule,shadow] of css.matchAll(/([^{}]*)\{[^}]*box-shadow:\s*([^;]+);/g)) {
    if (rule.trim() === '#document-sidebar') assert.match(shadow,/^[\d\spx]+color-mix\(in srgb, var\(--document-foreground\) \d+%, transparent\)$/);
    else assert.equal(shadow.trim(),'none');
  }
  assert.match(css,/button\s*\{[^}]*min-height:\s*32px[^}]*font-size:\s*\.875em/);
});

test('semantic variants tint callouts and badges with host tokens',() => {
  const css = readFileSync(new URL('../dev.sailry.platform/document/brief.css',import.meta.url),'utf8');
  for (const [variant,token] of [['warn','warning'],['risk','danger'],['ok','success']]) {
    assert.match(css,new RegExp(`\\.callout\\.${variant} \\{ --document-tone: var\\(--document-${token}\\); border-color: color-mix\\(in srgb, var\\(--document-${token}\\)`));
  }
  assert.doesNotMatch(css,/\.callout\.info\s*\{/);
  assert.match(css,/\.badge \{[^}]*border-radius: 999px/);
  assert.match(css,/\.badge\.new \{ background: var\(--document-muted\)/);
  assert.match(css,/\.badge\.removed,\.badge\.risk \{ color: var\(--document-danger\)/);
  assert.match(css,/ol\.steps > li::before \{ content: counter\(document-step\)/);
  assert.match(css,/\.compare \{[^}]*minmax\(min\(100%,220px\),1fr\)/);
  assert.match(css,/(?:^|\n)pre \{[^}]*white-space: pre;/);
  assert.doesNotMatch(css,/pre-wrap/);
});

test('chrome retains responsive and scroll regions',() => {
  const css = readFileSync(new URL('../dev.sailry.platform/document/brief.css',import.meta.url),'utf8');
  const template = readFileSync(new URL('../dev.sailry.platform/document/template.html',import.meta.url),'utf8');
  const ids = [...template.matchAll(/id="([^"]+)"/g)].map(match => match[1]);
  assert.deepEqual(ids,['document-header','document-brand','document-separator','document-title','document-menu',
    'document-menu-label','document-layout','document-sidebar','document-contents','document-toc','brief']);
  assert.match(template,/<span id="document-brand">Sailry<\/span>/);
  assert.match(template,/<button id="document-menu" type="button" aria-expanded="false" aria-controls="document-sidebar">/);
  // Exports serialize only main, so the header and contents must stay outside it.
  assert.match(template,/<\/header>[^]*<aside id="document-sidebar">[^]*<\/aside>\s*<main id="brief"><\/main>/);
  assert.doesNotMatch(template,/<(?:script|style)|\son\w+=/);
  for (const selector of ['pre','.document-table','.document-diagram,.document-chart','.document-math']) {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
    assert.match(css,new RegExp(`(?:^|\\n)${escaped}\\s*\\{[^}]*overflow-x: auto;`));
  }
  assert.match(css,/#document-header \{ position: sticky; top: 0;[^}]*height: var\(--document-header-height\)/);
  assert.match(css,/main \{[^}]*max-width: 720px; margin: 0 auto;/);
  assert.doesNotMatch(css,/margin: auto; min-width/);
  assert.match(css,/@media\(max-width:1000px\) \{[^@]*#document-menu \{ display: inline-flex; \}[^@]*#document-sidebar \{ display: none; position: fixed;[^@]*#document-sidebar\[data-open\] \{ display: block; \}/);
  assert.match(css,/@media\(max-width:520px\) \{[^@]*#document-brand,#document-separator \{ display: none; \}/);
  assert.match(css,/@media print \{ #document-header,#document-sidebar,button \{ display: none !important; \}/);
  assert.match(css,/@media print[^]*details > \* \{ display: block; \}/);
});

function element(id = '',title) {
  const classes = new Set(),attributes = new Map();
  return {id,dataset:{title},children:[],headings:[],scrolled:false,style:{setProperty() {}},
    classList:{toggle(name,enabled) {enabled ? classes.add(name) : classes.delete(name);},
      contains(name) {return classes.has(name);}},
    append(child) {this.children.push(child);},replaceChildren(...children) {this.children = children;},
    replaceWith(replacement) {this.replacement = replacement;},
    setAttribute(name,value) {attributes.set(name,String(value));},removeAttribute(name) {attributes.delete(name);},
    getAttribute(name) {return attributes.get(name);},hasAttribute(name) {return attributes.has(name);},
    toggleAttribute(name,enabled) {enabled ? attributes.set(name,'') : attributes.delete(name);},
    getBoundingClientRect() {return {top:100};},addEventListener() {},focus() {this.focused = true;},
    querySelector() {return this.headings[0] ?? {};},querySelectorAll() {return this.headings;},
    closest(selector) {return selector === 'section[data-title]' ? this.owner : this.container;},
    scrollIntoView() {this.scrolled = true;}};
}

function heading(section,text,container) {
  const item = element();item.textContent = text;item.owner = section;item.container = container;
  section.headings.push(item);return item;
}

function visual(kind,text) {
  const source = element();source.classList.toggle(kind,true);source.textContent = text;return source;
}

function setup(sections,targets = [],labels = [],visuals = []) {
  const chrome = ['brief','document-toc','document-title','document-contents','document-sidebar','document-menu',
    'document-menu-label','document-layout'].map(id => element(id));
  const main = chrome[0],toc = chrome[1],listeners = {},events = [];
  main.querySelectorAll = selector => selector === 'section[data-title]' ? sections
    : selector === 'label' ? labels
    : selector === 'pre.mermaid,pre.chart,pre.math,span.math-inline' ? visuals
    : selector === '[id]' ? [...sections,...targets].filter(item => item.id) : [];
  const document = {
    documentElement:element(),
    getElementById(id) {return chrome.find(item => item.id === id);},
    querySelectorAll() {return [...chrome,...sections,...targets];},
    createElement() {return element();},
    addEventListener(name,handler) {listeners[name] = handler;},
  };
  const window = {mermaid:{initialize() {},async render() {throw Error('Invalid diagram');}},
    MathJax:{startup:{promise:Promise.resolve()},async tex2svgPromise() {throw Error('Invalid formula');}},
    sailryCharts:{render() {throw Error('Invalid chart');}},
    sailryForms:{collect() {throw Error('Invalid form');}},
    sailryDocumentPost:event => events.push(event)};
  vm.runInNewContext(source,{window,document,TextEncoder});
  const click = href => {
    let prevented = false;
    listeners.click({target:{closest(selector) {return selector === 'a[href^="#"]' ? {getAttribute() {return href;}} : null;}},
      preventDefault() {prevented = true;}});
    assert(prevented);
  };
  const press = (...inside) => listeners.click({target:{closest(selector) {return inside.includes(selector) ? {} : null;}},
    preventDefault() {}});
  const chromeElement = id => chrome.find(item => item.id === id);
  return {toc,click,press,chrome:chromeElement,events,scroll:() => listeners.scroll(),
    key:key => listeners.keydown({key}),mermaid:window.mermaid,forms:window.sailryForms,MathJax:window.MathJax,
    submit:form => listeners.submit({target:form,preventDefault() {}}),
    retheme:(value = {}) => window.sailryDocument.retheme(value),
    mount:(identity = {}) => window.sailryDocument.mount({
      labels:{contents:'Contents'},identity:{artifact:'document',revision:1,title:'Brief',...identity},
      theme:{},html:'',writable:true,
    })};
}

test('authored section anchors survive mounting and encoded navigation',async () => {
  const section = element('choices 中文','Choices'),view = setup([section]);
  await view.mount();
  assert.equal(section.id,'choices 中文');
  assert.equal(view.toc.children[0].href,'#choices%20%E4%B8%AD%E6%96%87');
  view.click(view.toc.children[0].href);assert(section.scrolled);
});

test('the directory highlights the section at the current scroll position',async () => {
  const first = element('first','First'),second = element('second','Second'),view = setup([first,second]);
  await view.mount();assert(view.toc.children[0].classList.contains('active'));
  second.getBoundingClientRect = () => ({top:70});view.scroll();
  assert(view.toc.children[1].classList.contains('active'));assert.equal(view.toc.children[0].classList.contains('active'),false);
  assert.doesNotMatch(source,/document-zoom|showModal|setZoom/);
});

test('the directory lists section subheadings and follows them',async () => {
  const first = element('first','First'),second = element('second','Second');
  heading(first,'First',null);
  const sub = heading(first,' Detail ',null);
  heading(first,'Option',{});
  heading(second,'Nested',element());
  const view = setup([first,second],[element('document-heading-0')]);
  await view.mount();
  assert.deepEqual(view.toc.children.map(link => [link.textContent,link.className ?? '']),
    [['First',''],['Detail','sub'],['Second','']]);
  assert.equal(sub.id,'document-heading-1');
  assert.equal(view.toc.children[1].href,'#document-heading-1');
  sub.getBoundingClientRect = () => ({top:90});view.scroll();
  assert(view.toc.children[1].classList.contains('active'));
  assert.equal(view.toc.children[0].classList.contains('active'),false);
});

test('the contents menu toggles and closes from links, Escape and outside clicks',async () => {
  const view = setup([element('first','First')]);
  await view.mount();
  const sidebar = view.chrome('document-sidebar'),menu = view.chrome('document-menu');
  assert.equal(view.chrome('document-menu-label').textContent,'Contents');
  assert.equal(menu.hasAttribute('hidden'),false);
  assert.equal(view.chrome('document-layout').getAttribute('data-contents'),'some');
  view.press('#document-menu');
  assert(sidebar.hasAttribute('data-open'));assert.equal(menu.getAttribute('aria-expanded'),'true');
  view.press('#document-sidebar');assert(sidebar.hasAttribute('data-open'));
  view.click('#first');assert.equal(sidebar.hasAttribute('data-open'),false);
  assert.equal(menu.getAttribute('aria-expanded'),'false');
  view.press('#document-menu');view.key('Escape');
  assert.equal(sidebar.hasAttribute('data-open'),false);assert(menu.focused);
  view.press('#document-menu');view.press();
  assert.equal(sidebar.hasAttribute('data-open'),false);
  view.press('#document-menu');view.press('#document-menu');
  assert.equal(sidebar.hasAttribute('data-open'),false);
});

test('sectionless content hides the contents chrome',async () => {
  const view = setup([]);await view.mount();
  assert(view.chrome('document-sidebar').hasAttribute('hidden'));
  assert(view.chrome('document-menu').hasAttribute('hidden'));
  assert.equal(view.chrome('document-layout').getAttribute('data-contents'),'none');
});

test('generated anchors avoid document and authored target collisions',async () => {
  const unnamed = element('','New'),authored = element('document-section-1','Existing');
  const view = setup([unnamed,authored],[element('document-section-0')]);
  await view.mount();
  assert.equal(unnamed.id,'document-section-2');
  assert.equal(authored.id,'document-section-1');
  assert.deepEqual(view.toc.children.map(link => link.href),['#document-section-2','#document-section-1']);
});

test('fragment links target owned content and tolerate literal percent signs',async () => {
  const owned = element('document-visual'),literal = element('%invalid');
  const view = setup([],[owned,literal]);await view.mount();
  view.click('#document-visual');assert(owned.scrolled);
  view.click('#%invalid');assert(literal.scrolled);
});

test('form labels align only direct inputs and selects',async () => {
  const input = element(),select = element(),textarea = element(),plain = element(),wrapped = element();
  input.children = [{tagName:'INPUT'}];select.children = [{tagName:'SELECT'}];
  textarea.children = [{tagName:'TEXTAREA'}];wrapped.children = [{tagName:'SPAN',children:[{tagName:'INPUT'}]}];
  const labels = [input,select,textarea,plain,wrapped];
  const view = setup([],[],labels);await view.mount();
  assert.deepEqual(labels.map(label => label.classList.contains('document-control')),[true,true,false,false,false]);
});

test('visual failures keep source without automatic notifications',async () => {
  const visuals = [visual('mermaid','first'),visual('mermaid','second'),visual('math','formula'),visual('chart','chart')];
  const view = setup([],[],[],visuals);await view.mount();
  assert.equal(view.events.length,0);
  await view.retheme();await view.mount();
  assert.equal(view.events.length,0);
  for (const source of visuals) {
    assert.equal(source.replacement.children.length,1);
    assert.equal(source.replacement.children[0].textContent,source.textContent);
  }
});

test('new artifact versions preserve failed visual source',async () => {
  const source = visual('mermaid','invalid'),view = setup([],[],[],[source]);
  await view.mount();await view.mount({revision:2});await view.mount({artifact:'another',revision:2});
  await view.mount();await view.retheme();
  assert.equal(view.events.length,0);
  assert.equal(source.replacement.children[0].textContent,'invalid');
});

test('late visual failures cannot replace a newer mount',async () => {
  const source = visual('mermaid','invalid'),view = setup([],[],[],[source]);
  let reject,started;
  const ready = new Promise(resolve => {started = resolve;});
  view.mermaid.render = () => new Promise((_,fail) => {reject = fail;started();});
  const previous = view.mount();await ready;
  const obsolete = source.replacement;
  view.mermaid.render = async () => {throw Error('Invalid diagram');};
  await view.mount({revision:2});reject(Error('Late diagram failure'));await previous;
  assert.equal(obsolete.children.length,0);
  assert.equal(source.replacement.children[0].textContent,'invalid');
  assert.equal(view.events.length,0);
  await view.mount();assert.equal(view.events.length,0);
  assert.equal(source.replacement.children[0].textContent,'invalid');
});

test('required forms still report every invalid submission',async () => {
  const view = setup([],[],[],[visual('mermaid','invalid')]);await view.mount();
  const form = {dataset:{name:'questions'}};
  view.forms.collect = () => ({missing:{querySelector() {return {focus() {}};}}});
  view.submit(form);view.submit(form);
  view.forms.collect = () => {throw Error('Invalid form');};
  view.submit(form);view.submit(form);
  assert.deepEqual(view.events.map(event => event.kind),['required','required','invalid_form','invalid_form']);
});

test('formula words render upright while TeX commands are preserved',async () => {
  const cases = [
    ['rate = paid/visits','\\mathrm{rate} = \\mathrm{paid}/\\mathrm{visits}'],
    ['\\frac{a}{b}','\\frac{a}{b}'],
    ['\\text{rate} = x','\\text{rate} = x'],
    ['\\sin x + \\alpha','\\sin x + \\alpha'],
    // Runs shorter than three letters stay products, so mc remains m times c.
    ['E = mc^2','E = mc^2'],
    ['x_{max} + \\operatorname*{argmin} cost','x_{\\mathrm{max}} + \\operatorname*{argmin} \\mathrm{cost}'],
    ['\\begin{cases} x & \\text{if {nested} word} \\\\ y \\end{cases}','\\begin{cases} x & \\text{if {nested} word} \\\\ y \\end{cases}'],
  ];
  const visuals = cases.map(([source],index) => visual(index % 2 ? 'math' : 'math-inline',source));
  const view = setup([],[],[],visuals),rendered = [];
  view.MathJax.tex2svgPromise = async source => {rendered.push(source);throw Error('Unrendered');};
  await view.mount();
  assert.deepEqual(rendered,cases.map(([,expected]) => expected));
  assert.deepEqual(visuals.map(source => source.replacement.children[0].textContent),cases.map(([source]) => source));
});
