const en = {
  create: 'New terminal', access:'Access', workspace:'Workspace', full:'Full filesystem and network access', close: 'Close', loading: 'Loading', retry: 'Retry', cli: 'CLI',
  empty: 'No terminals', failed: 'Terminal action failed', loadFailed: 'Could not load terminals',
  unknown: 'Terminal result is unconfirmed', stopped: 'Terminal stopped', terminal: 'Terminal',
  refresh: 'Refresh', toolsFailed: 'Could not load CLI tools',
  unavailable: 'CLI is no longer available', launchFailed: 'Terminal could not start',
  exited: 'Terminal exited with code {code}', ended: 'Terminal ended unexpectedly',
  tools: {codex:'Codex',claude:'Claude',gemini:'Gemini',agy:'Agy',grok:'Grok',opencode:'OpenCode',kimi:'Kimi'},
};
const zh = {
  create: '新建终端', access:'访问范围', workspace:'工作区', full:'完整文件系统和网络访问', close: '关闭', loading: '加载中', retry: '重试', cli: 'CLI',
  empty: '暂无终端', failed: '终端操作失败', loadFailed: '无法加载终端',
  unknown: '终端结果尚未确认', stopped: '终端已停止', terminal: '终端', tools:en.tools,
  refresh: '刷新', toolsFailed: '无法读取 CLI',
  unavailable: 'CLI 已不可用', launchFailed: '终端启动失败',
  exited: '终端已退出（退出码 {code}）', ended: '终端异常结束',
};
const access = {
  de:{access:'Zugriff',workspace:'Arbeitsbereich',full:'Voller Dateisystem- und Netzwerkzugriff'},
  es:{access:'Acceso',workspace:'Espacio de trabajo',full:'Acceso completo al sistema de archivos y la red'},
  fr:{access:'Accès',workspace:'Espace de travail',full:'Accès complet au système de fichiers et au réseau'},
  ja:{access:'アクセス',workspace:'ワークスペース',full:'ファイルシステムとネットワークへのフルアクセス'},
  ko:{access:'접근 범위',workspace:'작업 공간',full:'전체 파일 시스템 및 네트워크 접근'},
  'pt-BR':{access:'Acesso',workspace:'Área de trabalho',full:'Acesso completo ao sistema de arquivos e à rede'},
  ru:{access:'Доступ',workspace:'Рабочая область',full:'Полный доступ к файловой системе и сети'},
  'zh-TW':{access:'存取範圍',workspace:'工作區',full:'完整檔案系統和網路存取'},
};
export function messages(locale) { return {...(locale === 'zh-CN' ? zh : en),...access[locale]}; }
