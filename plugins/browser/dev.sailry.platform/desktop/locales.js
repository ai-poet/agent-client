// Captions preserve the browser UI from Sailry c4bb60a2.
const en = {
  "browser_address": "Search or enter an address",
  "browser_new_tab": "New tab",
  "browser_empty": "Start browsing",
  "browser_back": "Back",
  "browser_forward": "Forward",
  "browser_reload": "Reload",
  "browser_go": "Open",
  "browser_annotate": "Select element",
  "browser_cancel_selection": "Cancel selection",
  "browser_annotation_unavailable": "Could not select or capture the element",
  "browser_invalid_address": "Invalid address",
  "browser_unavailable": "Browser could not start",
  "browser_load_failed": "Page failed to load",
  "settings_browser": "Browser",
  "close": "Close",
  "browser_data": "Local website data",
  "browser_data_local": "Keep website data",
  "browser_data_description": "Keep website data between uses, or only for this time",
  "browser_chrome_import": "Import from Chrome",
  "browser_chrome_description": "Import website cookies; some sites may need you to sign in again",
  "browser_choose_profile": "Choose a profile",
  "browser_import": "Import",
  "browser_imported": "Imported %{count} cookies, skipped %{skipped}",
  "browser_import_failed": "Cookie import failed",
  "browser_chrome_missing": "No Chrome profiles found",
  "browser_chrome_access_denied": "Chrome data access denied; check Full Disk Access",
  "browser_chrome_failed": "Could not read Chrome data; close Chrome and try again",
  "browser_chrome_schema": "This Chrome data format is not supported",
  "browser_chrome_empty": "This profile has no website cookies",
  "browser_chrome_key_denied": "Could not read the Chrome key; check Keychain access",
  "browser_chrome_cipher": "Could not decrypt this cookie",
  "browser_import_unavailable": "Chrome import is unavailable on this system",
  "browser_store_unavailable": "Keeping website data and importing requires macOS 14 or later",
  "resource_browser_preview": "Browser preview; no pages or network requests are loaded"
};
const zh = {
  "browser_address": "搜索或输入网址",
  "browser_new_tab": "新标签页",
  "browser_empty": "开始浏览",
  "browser_back": "后退",
  "browser_forward": "前进",
  "browser_reload": "刷新",
  "browser_go": "打开",
  "browser_annotate": "选择元素",
  "browser_cancel_selection": "取消选择",
  "browser_annotation_unavailable": "无法选择或截取元素",
  "browser_invalid_address": "网址无效",
  "browser_unavailable": "浏览器无法启动",
  "browser_load_failed": "网页加载失败",
  "resource_browser_preview": "浏览器 UI 预览，未加载网页或访问网络",
  "close": "关闭",
  "settings_browser": "浏览器",
  "browser_data": "本机网站数据",
  "browser_data_local": "保留网站数据",
  "browser_data_description": "开启后保留网站数据；关闭后仅在本次使用期间保留",
  "browser_chrome_import": "从 Chrome 导入",
  "browser_chrome_description": "导入网站 Cookie；部分网站可能需要重新登录",
  "browser_choose_profile": "选择档案",
  "browser_import": "导入",
  "browser_imported": "已导入 %{count} 个 Cookie，跳过 %{skipped} 个",
  "browser_import_failed": "Cookie 导入失败",
  "browser_chrome_missing": "未找到 Chrome 档案",
  "browser_chrome_access_denied": "无法访问 Chrome 数据，请检查“完全磁盘访问”设置",
  "browser_chrome_failed": "无法读取 Chrome 数据，请关闭 Chrome 后重试",
  "browser_chrome_schema": "暂不支持此 Chrome 数据格式",
  "browser_chrome_empty": "此档案没有网站 Cookie",
  "browser_chrome_key_denied": "无法读取 Chrome 密钥，请检查钥匙串授权",
  "browser_chrome_cipher": "无法解密此 Cookie",
  "browser_import_unavailable": "当前系统暂不支持 Chrome 导入",
  "browser_store_unavailable": "保留登录状态和导入需要 macOS 14 或更新版本"
};
const annotation = {
  'zh-TW':['選擇元素','取消選擇','無法選擇或擷取元素'],
  ja:['要素を選択','選択をキャンセル','要素を選択または撮影できませんでした'],
  ko:['요소 선택','선택 취소','요소를 선택하거나 캡처할 수 없습니다'],
  de:['Element auswählen','Auswahl abbrechen','Das Element konnte nicht ausgewählt oder erfasst werden'],
  fr:['Sélectionner un élément','Annuler la sélection','Impossible de sélectionner ou capturer l’élément'],
  es:['Seleccionar elemento','Cancelar selección','No se pudo seleccionar o capturar el elemento'],
  'pt-BR':['Selecionar elemento','Cancelar seleção','Não foi possível selecionar ou capturar o elemento'],
  ru:['Выбрать элемент','Отменить выбор','Не удалось выбрать или снять элемент']
};
const reviews = {
  "en": {
    "browser_annotation": "Element",
    "browser_annotation_note": "Annotation (optional)",
    "browser_annotation_add": "Add to conversation",
    "browser_annotation_stale": "The page or conversation changed",
    "browser_annotation_invalid": "Could not read the selected element",
    "browser_annotation_unavailable": "Could not capture the selected element",
    "browser_annotation_truncated": "Text is truncated",
    "settings_cancel": "Cancel",
    "chat_attachment_limit": "Up to 8 attachments per message"
  },
  "zh-CN": {
    "browser_annotation": "元素",
    "browser_annotation_note": "批注（可选）",
    "browser_annotation_add": "添加到会话",
    "browser_annotation_stale": "网页或会话已更改",
    "browser_annotation_invalid": "无法读取所选元素",
    "browser_annotation_unavailable": "无法截取所选元素",
    "browser_annotation_truncated": "文字已截断",
    "settings_cancel": "取消",
    "chat_attachment_limit": "每条消息最多 8 个附件"
  },
  "zh-TW": {
    "browser_annotation": "元素",
    "browser_annotation_note": "批註（選填）",
    "browser_annotation_add": "加入對話",
    "browser_annotation_stale": "網頁或對話已變更",
    "browser_annotation_invalid": "無法讀取所選元素",
    "browser_annotation_unavailable": "無法擷取所選元素",
    "browser_annotation_truncated": "文字已截斷",
    "settings_cancel": "取消",
    "chat_attachment_limit": "每條訊息最多 8 個附件"
  },
  "ja": {
    "browser_annotation": "要素",
    "browser_annotation_note": "注釈（任意）",
    "browser_annotation_add": "会話に追加",
    "browser_annotation_stale": "ページまたは会話が変更されました",
    "browser_annotation_invalid": "選択した要素を読み取れませんでした",
    "browser_annotation_unavailable": "選択した要素を撮影できませんでした",
    "browser_annotation_truncated": "テキストは省略されています",
    "settings_cancel": "キャンセル",
    "chat_attachment_limit": "メッセージ当たり最大 8 つの添付ファイル"
  },
  "ko": {
    "browser_annotation": "요소",
    "browser_annotation_note": "주석 (선택 사항)",
    "browser_annotation_add": "대화에 추가",
    "browser_annotation_stale": "페이지 또는 대화가 변경되었습니다",
    "browser_annotation_invalid": "선택한 요소를 읽을 수 없습니다",
    "browser_annotation_unavailable": "선택한 요소를 캡처할 수 없습니다",
    "browser_annotation_truncated": "텍스트가 잘렸습니다",
    "settings_cancel": "취소",
    "chat_attachment_limit": "메시지당 최대 8개의 첨부 파일"
  },
  "de": {
    "browser_annotation": "Element",
    "browser_annotation_note": "Anmerkung (optional)",
    "browser_annotation_add": "Zur Unterhaltung hinzufügen",
    "browser_annotation_stale": "Die Seite oder Unterhaltung wurde geändert",
    "browser_annotation_invalid": "Das ausgewählte Element konnte nicht gelesen werden",
    "browser_annotation_unavailable": "Das ausgewählte Element konnte nicht erfasst werden",
    "browser_annotation_truncated": "Text ist gekürzt",
    "settings_cancel": "Abbrechen",
    "chat_attachment_limit": "Bis zu 8 Anhänge pro Nachricht"
  },
  "fr": {
    "browser_annotation": "Élément",
    "browser_annotation_note": "Annotation (facultative)",
    "browser_annotation_add": "Ajouter à la conversation",
    "browser_annotation_stale": "La page ou la conversation a changé",
    "browser_annotation_invalid": "Impossible de lire l’élément sélectionné",
    "browser_annotation_unavailable": "Impossible de capturer l’élément sélectionné",
    "browser_annotation_truncated": "Texte tronqué",
    "settings_cancel": "Annuler",
    "chat_attachment_limit": "Jusqu'à 8 pièces jointes par message"
  },
  "es": {
    "browser_annotation": "Elemento",
    "browser_annotation_note": "Anotación (opcional)",
    "browser_annotation_add": "Añadir a la conversación",
    "browser_annotation_stale": "La página o conversación cambió",
    "browser_annotation_invalid": "No se pudo leer el elemento seleccionado",
    "browser_annotation_unavailable": "No se pudo capturar el elemento seleccionado",
    "browser_annotation_truncated": "Texto truncado",
    "settings_cancel": "Cancelar",
    "chat_attachment_limit": "Hasta 8 archivos adjuntos por mensaje"
  },
  "pt-BR": {
    "browser_annotation": "Elemento",
    "browser_annotation_note": "Anotação (opcional)",
    "browser_annotation_add": "Adicionar à conversa",
    "browser_annotation_stale": "A página ou conversa mudou",
    "browser_annotation_invalid": "Não foi possível ler o elemento selecionado",
    "browser_annotation_unavailable": "Não foi possível capturar o elemento selecionado",
    "browser_annotation_truncated": "Texto truncado",
    "settings_cancel": "Cancelar",
    "chat_attachment_limit": "Até 8 anexos por mensagem"
  },
  "ru": {
    "browser_annotation": "Элемент",
    "browser_annotation_note": "Примечание (необязательно)",
    "browser_annotation_add": "Добавить в разговор",
    "browser_annotation_stale": "Страница или разговор изменились",
    "browser_annotation_invalid": "Не удалось прочитать выбранный элемент",
    "browser_annotation_unavailable": "Не удалось снять выбранный элемент",
    "browser_annotation_truncated": "Текст сокращён",
    "settings_cancel": "Отмена",
    "chat_attachment_limit": "До 8 вложений на сообщение"
  }
};
export function messages(locale) {
  if (locale === 'zh-CN') return {...zh,...reviews[locale]};
  const value = annotation[locale];
  const base = value ? {...en,browser_annotate:value[0],browser_cancel_selection:value[1],browser_annotation_unavailable:value[2]} : en;
  return {...base,...(reviews[locale] ?? reviews.en)};
}
export function errorKey(error, fallback = 'browser_unavailable') {
  const message = String(error?.message ?? error ?? '');
  return Object.keys(messages('en')).find(key => message === key || message.endsWith(': ' + key)) ?? fallback;
}
export function imported(text, count, skipped) {
  return text.browser_imported.replace('%{count}', String(count)).replace('%{skipped}', String(skipped));
}
