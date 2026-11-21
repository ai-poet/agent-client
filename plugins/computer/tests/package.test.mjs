import assert from 'node:assert/strict';
import {access, readFile} from 'node:fs/promises';
import test from 'node:test';

const document = async name => JSON.parse(await readFile(new URL(`../${name}`,import.meta.url),'utf8'));
const selected = [
  'desktop_list_apps','desktop_list_windows','desktop_launch','desktop_snapshot','desktop_find',
  'desktop_get','desktop_is','desktop_screenshot','desktop_wait','desktop_skills_get',
  'desktop_click','desktop_right_click','desktop_type','desktop_set_value','desktop_press',
  'desktop_scroll','desktop_scroll_to','desktop_select','desktop_toggle','desktop_check','desktop_uncheck'
];

test('declares metadata without replacing backend tool authority', async () => {
  const manifest = await document('plugin.json');
  const extension = manifest.extensions['dev.sailry.platform'];
  assert.equal(extension.min_sailry_version,'0.1.6');
  assert.deepEqual(extension.actions,['computer.read','computer.control']);
  assert.equal(extension.scope,'host');
  assert.deepEqual(extension.tools.map(tool => tool.name),selected);
  for (const tool of extension.tools) {
    assert.equal(tool.server,'desktop');
    assert.deepEqual(Object.keys(tool).sort(),['display','name','presentation','server']);
  }
  const mcp = await document('mcp.json');
  assert.equal(mcp.$schema,'https://agent-plugins.org/schemas/1.0.0/mcp.schema.json');
  assert.deepEqual(Object.keys(mcp.mcpServers),['desktop']);
  const server = mcp.mcpServers.desktop;
  assert.equal(server.type,'stdio');
  assert.equal(server.command,'${SAILRY_COMPUTER_USE}');
  assert.deepEqual(server.args,['mcp']);
  assert.deepEqual(server.env,{AGENT_DESKTOP_HOME:'${PLUGIN_DATA}/agent-desktop'});
  assert.equal(server.exclusive,true);
  assert.equal(server.keep_images,3);
  assert.equal(server.reuse,false);
  await assert.rejects(access(new URL('../ocu.json',import.meta.url)));
});

test('selects headless tools that keep ordinary command boundaries', async () => {
  const server = (await document('mcp.json')).mcpServers.desktop;
  assert.deepEqual(server.tools,selected);
  assert.ok(server.tools.length <= 64);
  for (const excluded of [
    'desktop_batch','desktop_drag','desktop_hover','desktop_double_click','desktop_mouse_click',
    'desktop_key_down','desktop_clipboard_get','desktop_clipboard_set','desktop_close_app',
    'desktop_resize_window','desktop_dismiss_notification','desktop_session_start','desktop_trace_export',
    'desktop_cursor_overlay_enable','desktop_permissions','desktop_status'
  ]) {
    assert.ok(!server.tools.includes(excluded),excluded);
  }
});

test('has no settings: the agent cursor always shows', async () => {
  const extension = (await document('plugin.json')).extensions['dev.sailry.platform'];
  assert.equal(extension.settings_schema,undefined);
});

test('keeps snapshots and actions inspectable', async () => {
  const extension = (await document('plugin.json')).extensions['dev.sailry.platform'];
  const detailed = {
    desktop_launch:'/app',desktop_snapshot:'/app',desktop_press:'/combo',
    ...Object.fromEntries([
      'desktop_click','desktop_right_click','desktop_type','desktop_set_value','desktop_scroll',
      'desktop_scroll_to','desktop_select','desktop_toggle','desktop_check','desktop_uncheck'
    ].map(name => [name,'/ref_id']))
  };
  for (const tool of extension.tools) {
    assert.equal(tool.display.icon,'reicon:devices/monitor');
    assert.equal(Object.keys(tool.display.locales).length,10);
    if (tool.name in detailed) {
      assert.equal(tool.presentation,'details');
      assert.deepEqual(tool.display.input,{summary:{path:detailed[tool.name]}});
      assert.deepEqual(tool.display.output,{
        preview:{parts:[{value:{source:'result',path:'/output'},format:'json'}]}
      });
    } else {
      assert.equal(tool.presentation,'summary');
      assert.equal(tool.display.output,undefined);
    }
  }
});

test('instructs the snapshot, act and verify loop', async () => {
  const {instructions} = (await document('plugin.json')).extensions['dev.sailry.platform'];
  for (const phrase of ['STALE_REF','AMBIGUOUS_TARGET','disposition.retry is safe','desktop_snapshot','desktop_wait','Settings → Permissions']) {
    assert.ok(instructions.includes(phrase),phrase);
  }
  assert.ok(!/get_app_state|element indexes/.test(instructions));
});
