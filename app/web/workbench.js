'use strict';

const $ = id => document.getElementById(id);
function el(tag, className = '', text = '') {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = String(text);
  return node;
}
const fragment = new URLSearchParams(location.hash.slice(1));
const token = fragment.get('token') || sessionStorage.getItem('voucher-session-token') || '';
if (fragment.has('token')) {
  sessionStorage.setItem('voucher-session-token', token);
  history.replaceState(null, '', location.pathname + location.search);
}
const DEFAULT_RULES = '# 一行一条；以 # 开头的行为注释\n**/indexImg/**\n**/index.html\n**/.DS_Store\n**/._*\n**/Thumbs.db';
const active = job => job && ['queued', 'running'].includes(job.status);
const batchJob = job => job && ['batch_analyze', 'batch_generate', 'batch_scan'].includes(job.kind);
const scanJob = job => job?.kind === 'batch_scan';
const jobKindLabel = job => scanJob(job) ? '资料扫描' : job.kind === 'batch_analyze' ? '图片方向调整' : '批量合并';
const labels = {queued:'等待处理',running:'正在处理',completed:'已完成',completed_with_errors:'部分完成',cancelled:'已取消',interrupted:'意外中断',failed:'处理失败',skipped:'已核验，复用结果',skipped_existing:'跳过同名文件'};
const fmt = value => Number(value || 0).toLocaleString('zh-CN');
const basename = path => String(path || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop();
const shortTime = value => value ? new Date(value).toLocaleString('zh-CN', {hour12:false,month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'}) : '';
const prettyPath = value => String(value || '').replace(/^\\\\\?\\UNC\\/i, '\\\\').replace(/^\\\\\?\\/, '');
const samePath = (a, b) => prettyPath(a).replace(/[\\/]+$/, '') === prettyPath(b).replace(/[\\/]+$/, '');
const S = {
  ready:false, status:null, operations:0, operationLabel:'', plan:null, plans:[], jobs:[], job:null,
  initialization:null, indexInspection:null, groupId:null, fileId:null, sourcePage:1, selected:new Set(),
  drafts:new Map(), sourceDirty:false, preparationCollapsed:false, orientationThreads:4, mergeThreads:4,
  previewEpoch:0, renderEpoch:0, previewKey:null, previewUrl:null, image:null, pdf:null, pdfTask:null,
  pdfRender:null, pdfModule:null, zoom:1, fit:true, polling:null, syncing:false, pollCount:0,
  toastTimer:null, mergeRetry:null, orientationRetry:null, historyLoading:false,
  renamePreview:null, renameOverrides:new Map(), preparationEpoch:0, namingEpoch:0, previewTimer:null,
  editTimer:null, editError:false, scan:null, scanPolling:false, logFilter:'all', events:[],
  pollEpoch:0, conflictKey:null, conflictRequest:0, conflictBusy:false, cancelInFlightId:null, cancelRequestedJobId:null,
  browser:{kind:'folder',target:'root-path',path:'',parent:null,selected:null}, closeAfterSave:false
};
const selectedGroup = () => S.plan?.groups.find(g => g.id === S.groupId);
const selectedFile = () => selectedGroup()?.files.find(f => f.id === S.fileId);
const busy = () => S.operations > 0 || active(S.scan) || active(S.job) || S.jobs.some(active);
const hasDrafts = () => S.drafts.size > 0;
const selectedGroups = () => S.plan?.groups.filter(g => S.selected.has(g.id)) || [];
const allFiles = plan => (plan?.groups || []).flatMap(g => g.files);
const hasErrors = group => group.empty || !group.files.length || groupProblems(group).some(i => i.severity === 'error');
const hasPlanErrors = () => (S.plan?.issues || []).some(i => i.severity === 'error' && i.code !== 'output_existing');
const successfulOutput = result => Boolean(result.output_path && ['completed','skipped'].includes(result.status));
const imageFile = file => file?.kind === 'image';
const pdfFile = file => file?.kind === 'pdf';
function pdfReviewedPages(file) { return Array.from({length:file.pages}, (_, i) => Boolean(file.reviewed_pages?.[i])); }
function rotationChange(change) {
  return change && [0,90,180,270].includes(change.from_cw) && [0,90,180,270].includes(change.to_cw) && change.from_cw !== change.to_cw ? change : null;
}
function appliedAutoChange(file) {
  const change = rotationChange(file?.auto_rotation_change);
  return imageFile(file) && !file.manual_rotation && change && file.rotation_cw === change.to_cw ? change : null;
}
function autoTouched(file) { return Boolean(appliedAutoChange(file)); }
function pendingReview(file) {
  return autoTouched(file) ? 1 : 0;
}
function pendingPdfPages(file) { return pdfReviewedPages(file).filter((reviewed, i) => !reviewed && (Number(file.rotation_cw || 0) !== 0 || Number(file.page_rotations?.[i] || 0) !== 0)).length; }
function fileStatus(file) {
  if (file.error) return {tone:'error',label:'来源有问题',symbol:'!'};
  if (imageFile(file)) {
    if (file.manual_rotation) return {tone:'manual',label:'已手动调整',symbol:'↻'};
    const change = appliedAutoChange(file);
    if (change) return {tone:'review',label:`自动调整 ${change.from_cw}° → ${change.to_cw}°`,symbol:'↻'};
    if (file.reviewed) return {tone:'reviewed',label:file.orientation_error ? '已人工核对 · 原识别失败' : '已人工核对',symbol:'✓'};
    if (file.orientation_error) return {tone:'original',label:'识别未完成 · 方向未变',symbol:'—'};
    if (file.orientation?.needs_review) return {tone:'original',label:'方向未决 · 未调整',symbol:'—'};
    return {tone:'original',label:'保留当前方向',symbol:'—'};
  }
  const reviews = pdfReviewedPages(file).filter(Boolean).length;
  if (reviews === file.pages && file.pages) return {tone:'reviewed',label:`已核对 ${fmt(reviews)} / ${fmt(file.pages)} 页`,symbol:'✓'};
  const rotated = Array.from({length:file.pages}, (_, i) => (Number(file.rotation_cw || 0) + Number(file.page_rotations?.[i] || 0)) % 360).filter(Boolean).length;
  if (file.manual_rotation) return {tone:'manual',label:rotated ? `已手动调整 ${fmt(rotated)} 页` : '已手动调整，当前为原页向',symbol:'↻'};
  if (reviews) return {tone:'original',label:`已核对 ${fmt(reviews)} / ${fmt(file.pages)} 页`,symbol:'✓'};
  return {tone:'original',label:'保留原 PDF 页向',symbol:'—'};
}
function groupTone(group) {
  if (hasErrors(group)) return 'error';
  if (group.files.some(f => fileStatus(f).tone === 'error')) return 'error';
  if (group.files.some(f => pendingReview(f))) return 'review';
  if (group.files.some(f => f.manual_rotation)) return 'manual';
  if (group.files.some(f => imageFile(f) ? f.reviewed : pdfReviewedPages(f).some(Boolean))) return 'reviewed';
  return 'original';
}
function groupDraft(group) {
  return {group_id:group.id,file_ids:group.files.map(f => f.id),output_name:group.output_name,...S.drafts.get(group.id)};
}
function setDraft(group, changes) {
  S.drafts.set(group.id, {group_id:group.id,...S.drafts.get(group.id),...changes}); S.editError = false; controls();
}
function orderedFiles(group) {
  const map = new Map(group.files.map(f => [f.id, f]));
  return groupDraft(group).file_ids.map(id => map.get(id)).filter(Boolean);
}
async function api(path, options = {}) {
  const headers = new Headers(options.headers || {}); headers.set('X-Session-Token', token);
  if (options.body && typeof options.body !== 'string') {
    options.body = JSON.stringify(options.body); headers.set('Content-Type', 'application/json');
  }
  let response;
  try { response = await fetch('/api' + path, {...options,headers,cache:'no-store'}); }
  catch (_) { throw new Error('无法连接 PDF工作台，请确认应用仍在运行，必要时从启动入口重新打开。'); }
  if (!response.ok) {
    let message;
    try { message = (await response.json()).error; } catch (_) { /* not a JSON error */ }
    throw new Error(message || `操作未完成（${response.status}），请检查输入或重新打开应用。`);
  }
  return options.blob ? response.blob() : response.json();
}
function nativeInvoke() { return window.__TAURI__?.core?.invoke; }
function showError(error) {
  $('global-alert').textContent = error.message || String(error); $('global-alert').hidden = false;
}
function action(fn) {
  return async event => {
    $('global-alert').hidden = true;
    try { await fn(event); } catch (error) { showError(error); } finally { controls(); }
  };
}
async function operation(label, fn) {
  const previous = S.operationLabel; S.operations++; S.operationLabel = label; controls();
  try { return await fn(); } finally { S.operations--; S.operationLabel = previous; controls(); }
}
function toast(message) {
  clearTimeout(S.toastTimer); $('toast').textContent = message; $('toast').hidden = false;
  S.toastTimer = setTimeout(() => { $('toast').hidden = true; }, 4300);
}
function setView(view) {
  $('batch-view').hidden = view !== 'batch'; $('history-view').hidden = view !== 'history';
  $('tab-batch').classList.toggle('active', view === 'batch'); $('tab-history').classList.toggle('active', view === 'history');
  $('tab-batch').setAttribute('aria-selected', String(view === 'batch')); $('tab-history').setAttribute('aria-selected', String(view === 'history'));
  if (view === 'history') refreshHistory().catch(showError);
}
function setPreparationCollapsed(collapsed) {
  S.preparationCollapsed = collapsed; $('source-form').hidden = collapsed; $('source-summary').hidden = !collapsed;
  $('toggle-preparation').textContent = collapsed ? '更改设置' : '收起设置';
  $('source-form').closest('.preparation').classList.toggle('collapsed', collapsed);
  $('preparation-intro').hidden = collapsed;
}

function controls() {
  const locked = busy() || !S.ready;
  if (locked || S.sourceDirty || hasDrafts()) clearDragState();
  $('source-form').querySelectorAll('input,select,textarea,button').forEach(node => { node.disabled = locked; });
  $('merge-level').disabled = locked || !S.initialization;
  for (const id of ['index-column','name-column']) $(id).disabled = locked || !S.indexInspection?.columns?.length;
  $('rename-section').hidden = !$('rename-enabled').checked;
  $('refresh-names').disabled = locked || !$('root-path').value.trim() || !$('index-path').value.trim();
  $('scan-button').disabled = locked || !$('root-path').value.trim();
  $('scan-button').textContent = S.operations ? S.operationLabel || '正在处理…' : active(S.scan) ? '正在扫描资料…' : '扫描资料 →';
  $('source-dirty-note').hidden = !S.sourceDirty || !S.plan;
  $('save-edits').hidden = !S.editError || !hasDrafts(); $('save-edits').disabled = locked || S.sourceDirty;
  $('draft-notice').hidden = !S.editError || !hasDrafts();
  $('edit-state').textContent = S.editError ? '保存失败' : hasDrafts() ? '正在保存…' : S.plan ? '调整已保存' : '';
  const selected = selectedGroups();
  $('merge-button').disabled = locked || !selected.length || S.sourceDirty || hasDrafts();
  $('merge-button').textContent = selected.length ? `执行合并 · ${fmt(selected.length)} 份` : '执行合并';
  $('merge-brief').textContent = `已选择 ${fmt(selected.length)} 个文件夹，将生成 ${fmt(selected.length)} 份 PDF，共 ${fmt(selected.reduce((n,g) => n + Number(g.pages || 0),0))} 页。`;
  $('merge-destination').textContent = S.plan ? `输出位置：${prettyPath(S.plan.output_dir)} · 点击执行合并后可更改` : '';
  const invalid = selected.filter(hasErrors).length;
  $('merge-state').hidden = !S.sourceDirty && !invalid && !hasPlanErrors() && !hasDrafts();
  $('merge-state').textContent = S.sourceDirty ? '资料设置已更改，请重新扫描。' : hasDrafts() ? '正在保存调整，请稍候。' : invalid ? `${fmt(invalid)} 个所选文件夹存在问题，请在上方处理，或取消选择。` : '扫描发现资料范围错误，请查看日志并处理。';
  const imageCount = selected.reduce((n,g) => n + g.files.filter(f => imageFile(f) && !f.error).length,0);
  $('auto-orient').disabled = locked || !imageCount || S.sourceDirty || hasDrafts();
  $('auto-orient').title = !imageCount ? '所选文件夹没有可识别图片，PDF可逐页手动旋转。' : '可选预处理，识别后可查看和修正方向。';
  $('start-orientation').disabled = locked || !$('orientation-consent').checked || !imageCount || !S.status?.model_ready;
  $('orientation-threads').disabled = locked || Boolean(S.orientationRetry);
  $('merge-threads').disabled = locked || Boolean(S.mergeRetry);
  $('select-all-groups').disabled = locked || S.sourceDirty;
  const groups = S.plan?.groups.filter(g => !g.empty && g.files.length) || [];
  $('select-all-groups').checked = groups.length > 0 && groups.every(g => S.selected.has(g.id));
  $('select-all-groups').indeterminate = selected.length > 0 && selected.length < groups.length;
  $('group-list').querySelectorAll('input[type=checkbox]').forEach(node => {
    const g = S.plan.groups.find(g => g.id === node.value); node.disabled = locked || S.sourceDirty || !g || g.empty || !g.files.length;
  });
  $('group-list').querySelectorAll('[data-edit]').forEach(node => { node.disabled = locked || S.sourceDirty; });
  for (const id of ['file-sort','sort-direction','sort-separator','apply-sort']) $(id).disabled = locked || S.sourceDirty || !selectedGroup();
  $('file-list').querySelectorAll('[data-edit]').forEach(node => { node.disabled = locked || S.sourceDirty || node.dataset.limit === 'true' || (node.dataset.edit === 'drag' && hasDrafts()); });
  $('removed-file-list').querySelectorAll('button').forEach(node => { node.disabled = locked || S.sourceDirty; });
  const file = selectedFile();
  const canEditFile = Boolean(file && !file.error && !locked && !S.sourceDirty && !hasDrafts());
  for (const id of ['rotate-left','rotate-right','rotate-half','reset-rotation','rotation-scope']) $(id).disabled = !canEditFile;
  $('rotation-scope').disabled = !canEditFile || !pdfFile(file);
  $('previous-source-page').disabled = !file || !pdfFile(file) || S.sourcePage <= 1;
  $('next-source-page').disabled = !file || !pdfFile(file) || S.sourcePage >= file.pages;
  $('source-page-number').disabled = !file || !pdfFile(file);
  for (const id of ['zoom-in','zoom-out','fit-preview']) $(id).disabled = !file || Boolean(file.error);
  $('retry-job').disabled = locked || S.sourceDirty || hasDrafts();
  $('start-merge').disabled = locked || !S.plan || !selected.length || hasPlanErrors() || selected.some(hasErrors) || !samePath($('output-path').value.trim(),S.plan.output_dir);
  $('choose-output').disabled = locked || Boolean(S.mergeRetry); $('output-path').disabled = locked || Boolean(S.mergeRetry);
  $('check-output').disabled = locked;
  $('deselect-errors').hidden = !selected.some(hasErrors); $('deselect-errors').disabled = locked;
  $('summary-selected').textContent = `已选 ${fmt(selected.length)}`;
  $('group-selection-count').textContent = `已选择 ${fmt(selected.length)} / ${fmt(groups.length)}`;
  $('cancel-scan').disabled = !active(S.scan) || Boolean(S.scan.cancelRequested);
  const external = S.jobs.find(job => active(job) && !batchJob(job)); $('external-task').hidden = !external;
  $('external-task').textContent = external ? '旧版任务仍在处理中，完成或取消后即可开始新任务。' : '';
}
function changedSource() {
  S.namingEpoch++;
  if (S.plan) S.sourceDirty = true;
  if (S.initialization && !samePath($('root-path').value.trim(),S.initialization.root)) {
    S.initialization = null; S.renamePreview = null; $('directory-preview').hidden = true; $('level-hint').textContent = '选择文件夹后自动获取';
  }
  controls();
}
function issueList(issues) {
  const list = el('ul', 'issue-list');
  for (const issue of issues) {
    const row = el('li', issue.severity === 'error' ? 'error' : '', issue.message);
    if (issue.path) row.append(el('small', '', ` · ${prettyPath(issue.path)}`));
    list.append(row);
  }
  return list;
}
async function initializeFolder({level} = {}) {
  const root = $('root-path').value.trim(); if (!root) throw new Error('请选择目标文件夹。');
  const previous = level ?? (S.initialization && samePath(S.initialization.root,root) ? Number($('merge-level').value || 0) : 0);
  const epoch = ++S.preparationEpoch;
  return operation('正在准备目录…',async () => {
    $('initialization-note').hidden = false; $('initialization-note').textContent = '正在获取目录信息…';
    const result = await api('/batch/initialize',{method:'POST',body:{root,level:previous,exclude_rules:$('exclude-rules').value}});
    if (epoch !== S.preparationEpoch || !samePath(root,$('root-path').value.trim())) return null;
    S.initialization = result; S.renamePreview = null; $('root-path').value = prettyPath(result.root);
    renderLevels(result.level ?? previous); renderDirectoryPreview(result);
    $('initialization-note').hidden = true; changedSource(); return result;
  });
}
function renderLevels(preferred = 0) {
  const select = $('merge-level'); select.replaceChildren();
  const levels = S.initialization?.levels || [];
  for (const info of levels) {
    const option = el('option','',info.level === 0 ? '当前文件夹' : `第 ${info.level} 级文件夹`); option.value = info.level; select.append(option);
  }
  if (!select.options.length) { const option = el('option','','当前文件夹'); option.value = 0; select.append(option); }
  select.value = [...select.options].some(option => Number(option.value) === preferred) ? String(preferred) : '0';
  renderLevelHint(); controls();
}
function renderLevelHint() {
  if (!S.initialization) return;
  const info = S.initialization.levels?.find(item => Number(item.level) === Number($('merge-level').value));
  const count = S.initialization.summary?.nonempty_groups ?? S.initialization.groups?.filter(g => !g.empty && g.candidate_count).length ?? info?.folder_count ?? 0;
  $('level-hint').textContent = `${fmt(count)} 个文件夹待合并`;
}
function renderDirectoryPreview(data = S.initialization) {
  const groups = data?.groups || []; $('directory-preview').hidden = !data;
  const pending = data?.summary?.nonempty_groups ?? groups.filter(g => !g.empty && Number(g.candidate_count || 0) > 0).length;
  const files = data?.summary?.candidate_count ?? groups.reduce((total,g) => total + Number(g.candidate_count || 0),0);
  const empty = Math.max(0,Number(data?.summary?.group_count ?? groups.length) - Number(pending));
  $('directory-count').textContent = `${fmt(pending)} 个待合并 · ${fmt(files)} 个文件 · ${fmt(empty)} 个空目录`;
  const box = $('directory-list'); box.replaceChildren();
  for (const group of groups) {
    const row = el('div','directory-row'); const path = el('span','directory-path',group.relative_path || basename(data.root)); path.title = prettyPath(group.path || group.relative_path); row.append(el('span','directory-icon','▣'),path);
    const count = Number(group.candidate_count ?? group.files?.length ?? 0);
    row.append(el('span','directory-meta',count ? `${fmt(count)} 个文件` : '无待合并文件'));
    if (group.shallow) row.append(el('span','pill','直属文件'));
    box.append(row);
  }
  if (!groups.length) box.append(el('p','empty-pane','此范围内没有待合并文件，可调整合并级别或排除规则。'));
  renderLevelHint();
}
async function refreshPreparation({level,inspect = false} = {}) {
  await initializeFolder({level});
  if ($('rename-enabled').checked && $('index-path').value.trim()) {
    if (inspect || !S.indexInspection || !samePath(S.indexInspection.path,$('index-path').value.trim())) await inspectIndex();
    else if (!S.indexInspection.sheet_error) await refreshNamingPreview();
  } else renderNamingPreview();
}
function scheduleNamingPreview() {
  clearTimeout(S.previewTimer); changedSource();
  S.previewTimer = setTimeout(async () => {
    if (busy()) { scheduleNamingPreview(); return; }
    try { await refreshNamingPreview(); } catch (error) { showError(error); }
  },350);
}
async function inspectIndex({sheet,headerRow,refresh = true} = {}) {
  const path = $('index-path').value.trim();
  if (!path) { S.indexInspection = null; $('index-settings').hidden = true; S.renamePreview = null; renderNamingPreview(); changedSource(); return null; }
  let data;
  await operation('正在读取索引…',async () => {
    $('index-loading').hidden = false;
    try {
      const body = {path}; if (sheet) body.sheet = sheet;
      if (headerRow !== undefined) {
        if (!Number.isInteger(headerRow) || headerRow < 1) throw new Error('表头行应为从 1 开始的整数。');
        body.header_row = headerRow;
      }
      data = await api('/batch/index/inspect',{method:'POST',body});
      S.indexInspection = data; $('index-path').value = prettyPath(data.path); $('index-settings').hidden = false;
      $('index-sheet-warning').hidden = !data.sheet_error;
      $('index-sheet-warning').textContent = data.sheet_error ? `${data.sheet_error}。可改选其他工作表，也可关闭重命名。` : '';
      $('index-sheet').replaceChildren(...data.sheets.map(info => { const option = el('option','',info.name); option.value = info.name; return option; }));
      $('index-sheet').value = data.sheet; $('header-row').value = data.header_row;
      for (const id of ['index-column','name-column']) {
        $(id).replaceChildren(...data.columns.map(column => { const option = el('option','',`${column.letter} · ${column.name || '未命名列'}`); option.value = column.index; return option; }));
      }
      $('index-column').value = String(data.suggested_index_column ?? data.columns[0]?.index ?? '');
      $('name-column').value = String(data.suggested_name_column ?? data.columns[1]?.index ?? data.columns[0]?.index ?? '');
      renderIndexPreview(data); changedSource();
    } finally { $('index-loading').hidden = true; }
  });
  if (refresh && data && !data.sheet_error && $('root-path').value.trim()) await refreshNamingPreview();
  else { S.renamePreview = null; renderNamingPreview(); }
  return data;
}
function renderIndexPreview(data) {
  if (!data.columns.length) { $('index-preview-table').replaceChildren(el('p','help','当前工作表没有可预览的列，请改选工作表或调整表头行。')); return; }
  const table = el('table'); const head = el('thead'); const headRow = el('tr'); headRow.append(el('th','','行号'));
  for (const column of data.columns) headRow.append(el('th','',`${column.letter} · ${column.name || '未命名列'}`));
  head.append(headRow); table.append(head); const body = el('tbody');
  for (const record of (data.preview_rows || []).slice(0,8)) {
    const row = el('tr'); row.append(el('td','',record.row));
    for (const column of data.columns) { const value = record.values[column.index] ?? ''; const cell = el('td','',value); cell.title = value; row.append(cell); }
    body.append(row);
  }
  table.append(body); $('index-preview-table').replaceChildren(table);
}
function indexRequest() {
  if (!$('rename-enabled').checked) return null;
  const path = $('index-path').value.trim(); if (!path) return null;
  if (!S.indexInspection || !samePath(S.indexInspection.path,path)) throw new Error('索引尚未读取，请重新选择索引文件。');
  if (S.indexInspection.sheet_error) throw new Error(`${S.indexInspection.sheet_error}。请改选工作表，或关闭重命名。`);
  if ($('index-column').value === '' || $('name-column').value === '') throw new Error('请选择文件夹名称索引和重命名文件名。');
  if (Number($('header-row').value) !== Number(S.indexInspection.header_row)) throw new Error('表头行正在更新，请稍候再扫描。');
  return {path,sheet:$('index-sheet').value,index_column:Number($('index-column').value),name_column:Number($('name-column').value),header_row:Number($('header-row').value),match_rule:$('match-rule').value,naming:$('naming-mode').value,duplicates:'first'};
}
function preparationRequest() {
  const validPaths = new Set((S.initialization?.groups || []).map(g => g.relative_path));
  const output_names = [...S.renameOverrides].filter(([path]) => validPaths.has(path)).map(([relative_path,output_name]) => ({relative_path,output_name}));
  return {root:$('root-path').value.trim(),level:Number($('merge-level').value),exclude_rules:$('exclude-rules').value,index:indexRequest(),sort:{mode:'name',descending:false},output_names:$('rename-enabled').checked ? output_names : []};
}
async function refreshNamingPreview() {
  if (!$('rename-enabled').checked || !$('root-path').value.trim() || !$('index-path').value.trim()) { S.renamePreview = null; renderNamingPreview(); return; }
  if (!S.initialization || !samePath(S.initialization.root,$('root-path').value.trim())) await initializeFolder();
  await operation('正在更新命名预览…',async () => {
    $('rename-preview-note').textContent = '正在生成文件名预览…';
    const epoch = ++S.namingEpoch; const request = preparationRequest(); const signature = JSON.stringify(request);
    const data = await api('/batch/preview',{method:'POST',body:request});
    if (epoch !== S.namingEpoch || !$('rename-enabled').checked || !samePath(request.root,$('root-path').value.trim()) || request.level !== Number($('merge-level').value)) return;
    try { if (JSON.stringify(preparationRequest()) !== signature) return; } catch (_) { return; }
    S.renamePreview = data;
    const allowed = new Set(data.groups.map(g => g.relative_path)); S.renameOverrides = new Map([...S.renameOverrides].filter(([path]) => allowed.has(path)));
    for (const group of data.groups) if (S.renameOverrides.has(group.relative_path)) S.renameOverrides.set(group.relative_path,group.output_name);
    renderNamingPreview();
  });
}
function canonicalOutputName(value) { const name = String(value || '').trim(); return (name.toLowerCase().endsWith('.pdf') ? name : name + '.pdf').normalize('NFC').toLocaleLowerCase(); }
function namingCollisions(groups, valueOf) {
  const map = new Map(); for (const group of groups.filter(g => !g.empty)) { const key = canonicalOutputName(valueOf(group)); if (!map.has(key)) map.set(key,[]); map.get(key).push(group.relative_path || group.id); }
  return new Set([...map.values()].filter(paths => paths.length > 1).flat());
}
function renderNamingPreview() {
  const box = $('rename-preview-list'); box.replaceChildren();
  const groups = S.renamePreview?.groups || [];
  $('rename-preview-count').textContent = groups.length ? `${fmt(groups.length)} 个文件夹` : '';
  $('rename-collision').hidden = true;
  if (!groups.length) { $('rename-preview-note').textContent = S.indexInspection?.sheet_error ? '请改选可读取的工作表。' : '选择索引文件并指定两列后，自动显示预期文件名。'; return; }
  const valueOf = group => S.renameOverrides.get(group.relative_path) ?? group.output_name;
  const collisions = namingCollisions(groups,valueOf);
  $('rename-collision').hidden = !collisions.size;
  $('rename-collision').textContent = `${fmt(collisions.size)} 个文件夹的输出名称相同，请直接修改下方文件名。`;
  $('rename-preview-note').textContent = S.renamePreview.index_error ? `索引未生效：${S.renamePreview.index_error}。当前使用文件夹名。` : '可直接修改单个文件名；修改会用于本次扫描。';
  const heading = el('div','rename-preview-row rename-preview-columns'); heading.append(el('span','','原文件夹'),el('span','','输出 PDF 文件名'),el('span','','匹配情况')); box.append(heading);
  for (const group of groups) {
    const row = el('div',`rename-preview-row${collisions.has(group.relative_path) ? ' has-collision' : ''}`);
    const path = el('span','rename-source',group.relative_path || basename(S.initialization.root)); path.title = group.relative_path; row.append(path);
    const field = el('div','rename-name-field'); const input = el('input'); input.value = valueOf(group); input.setAttribute('aria-label',`预期文件名 ${group.relative_path || group.name}`);
    input.addEventListener('input',() => { S.renameOverrides.set(group.relative_path,input.value); changedSource(); updateNamingCollisionLabels(); });
    input.addEventListener('change',action(refreshNamingPreview)); field.append(input);
    if (S.renameOverrides.has(group.relative_path)) { const reset = el('button','text-button','恢复'); reset.type = 'button'; reset.setAttribute('aria-label',`恢复自动命名 ${group.relative_path || group.name}`); reset.addEventListener('click',action(async () => { S.renameOverrides.delete(group.relative_path); changedSource(); await refreshNamingPreview(); })); field.append(reset); }
    row.append(field,el('span','rename-match',group.rename?.message || (group.rename?.status === 'matched' ? '已匹配' : '使用文件夹名'))); row.dataset.path = group.relative_path; box.append(row);
  }
}
function updateNamingCollisionLabels() {
  const groups = S.renamePreview?.groups || []; const collisions = namingCollisions(groups,group => S.renameOverrides.get(group.relative_path) ?? group.output_name);
  $('rename-collision').hidden = !collisions.size;
  $('rename-collision').textContent = `${fmt(collisions.size)} 个文件夹的输出名称相同，请修改文件名。`;
  $('rename-preview-list').querySelectorAll('[data-path]').forEach(row => row.classList.toggle('has-collision',collisions.has(row.dataset.path)));
}
async function scan() {
  if (busy()) return;
  clearTimeout(S.previewTimer); clearTimeout(S.editTimer);
  return operation('正在扫描资料…',async () => {
    if (!S.initialization || !samePath(S.initialization.root,$('root-path').value.trim())) await initializeFolder();
    if ($('rename-enabled').checked && $('index-path').value.trim() && (!S.indexInspection || !samePath(S.indexInspection.path,$('index-path').value.trim()))) await inspectIndex({refresh:false});
    const started = await api('/batch/scans',{method:'POST',body:preparationRequest()});
    S.job = null; $('job-panel').hidden = true; S.events = []; await followScan(started);
  });
}
async function followScan(initial,{syncForm = false} = {}) {
  if (S.scanPolling) return; S.scanPolling = true; S.scan = initial;
  $('welcome').hidden = true; $('workspace').hidden = false; $('scan-progress-panel').hidden = false; $('scan-details').hidden = false;
  $('scan-results').hidden = true; $('merge-section').hidden = true;
  try {
    while (true) {
      renderScanProgress(); renderLogs(); controls();
      if (!active(S.scan)) break;
      await new Promise(resolve => setTimeout(resolve,450));
      try { S.scan = await api(`/batch/scans/${S.scan.id}`); }
      catch (error) { showError(error); await new Promise(resolve => setTimeout(resolve,1200)); }
    }
    if (S.scan.status === 'completed' && S.scan.plan_id) {
      const plan = await api(`/batch/plans/${S.scan.plan_id}`);
      if (!S.initialization || !samePath(S.initialization.root,plan.root)) {
        try { S.initialization = await api('/batch/initialize',{method:'POST',body:{root:plan.root,level:plan.level,exclude_rules:plan.exclude_rules}}); renderLevels(plan.level); renderDirectoryPreview(); } catch (_) { S.initialization = null; }
      }
      acceptPlan(plan,{syncForm,collapse:true,preserveSelection:false});
      toast('扫描完成，可以调整文件名、顺序和方向。');
    } else if (S.scan.error) showError(new Error(S.scan.error));
  } finally {
    S.scanPolling = false;
    if (!active(S.scan)) { try { await refreshHistory(false); } catch (error) { showError(error); } }
    if (S.plan) { $('scan-results').hidden = false; $('merge-section').hidden = false; }
    renderScanProgress(); renderLogs(); controls();
  }
}
function renderScanProgress() {
  const scan = S.scan; if (!scan) return;
  const phases = {enumerating:'正在获取目录',validating:'正在检查文件',naming:'正在生成文件名',saving:'正在保存扫描结果',completed:'扫描完成'};
  $('scan-progress-panel').hidden = scan.status === 'completed';
  $('scan-progress-title').textContent = active(scan) ? phases[scan.phase] || '正在扫描资料' : labels[scan.status] || scan.status;
  $('scan-progress-detail').textContent = scan.total ? `${fmt(scan.done)} / ${fmt(scan.total)} 个文件` : '正在统计待处理文件';
  $('scan-progress-bar').style.width = `${scan.total ? Math.min(100,Number(scan.done) / Number(scan.total) * 100) : 0}%`;
  $('scan-current-file').textContent = scan.current_file || (active(scan) ? '请稍候，完成后将显示文件详情。' : scan.error || '');
  $('scan-elapsed').textContent = scan.elapsed_ms === undefined ? '' : `已用 ${formatDuration(scan.elapsed_ms)}`;
  $('cancel-scan').hidden = !active(scan);
}
async function cancelScan() {
  if (!active(S.scan)) return; S.scan.cancelRequested = true; controls();
  try { await api(`/jobs/${S.scan.id}/cancel`,{method:'POST'}); toast('已请求取消扫描，正在保存已完成状态。'); }
  catch (error) { S.scan.cancelRequested = false; throw error; }
}
function formatDuration(ms) { const seconds = Math.max(0,Math.round(Number(ms || 0) / 1000)); return seconds < 60 ? `${seconds} 秒` : `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`; }
function acceptPlan(plan, {syncForm = false, collapse = S.preparationCollapsed, preserveSelection = true} = {}) {
  const previous = S.plan; const oldGroup = selectedGroup(); const oldFile = selectedFile();
  const oldIds = new Set(S.selected); const oldPaths = new Set((previous?.groups || []).filter(g => oldIds.has(g.id)).map(g => g.relative_path));
  S.plan = plan; S.drafts.clear(); S.editError = false; S.sourceDirty = false; rememberPlan(plan);
  S.selected = new Set(plan.groups.filter(g => !g.empty && g.files.length && (!previous || !preserveSelection || oldIds.has(g.id) || oldPaths.has(g.relative_path))).map(g => g.id));
  S.groupId = plan.groups.find(g => g.id === oldGroup?.id)?.id || plan.groups.find(g => g.relative_path === oldGroup?.relative_path)?.id || plan.groups.find(g => g.files.length)?.id || plan.groups[0]?.id || null;
  const currentGroup = selectedGroup();
  S.fileId = currentGroup?.files.find(f => f.id === oldFile?.id)?.id || currentGroup?.files[0]?.id || null;
  if (S.fileId !== oldFile?.id) resetSourceView();
  S.sourcePage = Math.min(Math.max(1, S.sourcePage), selectedFile()?.pages || 1);
  if (syncForm) {
    $('root-path').value = prettyPath(plan.root); $('merge-level').value = plan.level;
    if (!S.initialization || !samePath(S.initialization.root, plan.root)) {
      S.initialization = null; const option = el('option','',plan.level ? `第 ${plan.level} 级（历史配置）` : '当前文件夹（历史配置）'); option.value = plan.level;
      $('merge-level').replaceChildren(option); $('level-hint').textContent = `${fmt(plan.groups.filter(g => !g.empty).length)} 个文件夹待合并`; $('initialization-note').hidden = true;
    } else { $('merge-level').value = String(plan.level); renderLevelHint(); }
    $('exclude-rules').value = plan.exclude_rules ?? DEFAULT_RULES;
    const index = plan.index?.request || plan.index?.config || plan.index;
    $('index-path').value = prettyPath(index?.path || ''); $('rename-enabled').checked = Boolean(index?.path);
    $('naming-history-note').hidden = !index?.path || index.duplicates === 'first';
    if (index?.path) {
      const inspected = S.indexInspection && samePath(S.indexInspection.path, index.path) && S.indexInspection.sheet === index.sheet && Number(S.indexInspection.header_row) === Number(index.header_row) && S.indexInspection.columns.length > 0;
      if (!inspected) S.indexInspection = {path:index.path,sheet:index.sheet,header_row:index.header_row,columns:[],sheets:[]};
      $('index-settings').hidden = false;
      $('index-sheet-warning').hidden = !S.indexInspection.sheet_error;
      if (!inspected) $('index-sheet').replaceChildren(el('option', '', index.sheet));
      $('index-sheet').value = index.sheet;
      $('header-row').value = index.header_row || 1;
      for (const [id, value] of [['index-column', index.index_column], ['name-column', index.name_column]]) {
        if (!inspected) { const option = el('option', '', `第 ${Number(value) + 1} 列`); option.value = value; $(id).replaceChildren(option); }
        $(id).value = String(value);
      }
      $('match-rule').value = index.match_rule || 'auto'; $('naming-mode').value = index.naming || 'index';
      if (!inspected) $('index-preview-table').replaceChildren(el('p','help','历史配置已载入；更换工作表或列前可重新读取索引。'));
    } else { S.indexInspection = null; $('index-settings').hidden = true; $('index-sheet-warning').hidden = true; }
  }
  $('source-summary').textContent = `${prettyPath(plan.root)} · ${plan.level ? `第 ${plan.level} 级分组` : '当前文件夹'} · ${fmt(plan.groups.length)} 个单元`;
  $('welcome').hidden = true; $('workspace').hidden = false; $('scan-results').hidden = false; $('merge-section').hidden = false; $('scan-details').hidden = false; $('toggle-preparation').hidden = false;
  setPreparationCollapsed(collapse); renderPlan(); controls();
}
function rememberPlan(plan) {
  const summary = {id:plan.id,parent_plan_id:plan.parent_plan_id,created_at:plan.created_at,root:plan.root,output_dir:plan.output_dir,level:plan.level,index:plan.index,group_count:plan.groups.length,file_count:allFiles(plan).length,pages:allFiles(plan).reduce((n,f) => n + Number(f.pages || 0),0),removed_file_count:plan.removed_sources?.length || 0};
  S.plans = [summary,...S.plans.filter(p => p.id !== plan.id)]; $('history-count').textContent = fmt(historyFamilies(S.plans).length);
}
function groupProblems(group) {
  const changedName = S.drafts.get(group.id)?.output_name;
  const errors = (group.issues || []).filter(issue => !['source_error','output_collision','output_existing'].includes(issue.code) && !(changedName !== undefined && changedName !== group.output_name && ['output_name','output_is_source'].includes(issue.code)));
  for (const file of group.files) if (file.error) errors.push({severity:'error',code:'source_error',message:`${file.name}：${file.error}`,path:file.relative_path});
  const name = groupDraft(group).output_name;
  if (!String(name || '').trim()) errors.push({severity:'error',code:'output_name',message:'输出文件名不能为空。'});
  const conflicts = (S.plan?.groups || []).filter(other => other.id !== group.id && !other.empty && canonicalOutputName(groupDraft(other).output_name) === canonicalOutputName(name));
  if (!group.empty && conflicts.length) errors.push({severity:'error',code:'output_collision',message:`输出名称与“${conflicts.map(g => g.relative_path || g.name).join('”“')}”相同，请修改文件名。`});
  return errors;
}
function currentLogs() {
  if (active(S.scan)) return (S.scan.logs || []).map(log => ({...log,level:log.level || 'info'}));
  const plan = S.plan; const logs = [];
  if (!plan) return (S.scan?.logs || []).map(log => ({...log,level:log.level || 'info'}));
  if (S.scan?.status === 'failed' && S.scan.error) logs.push({level:'error',message:S.scan.error});
  for (const issue of plan.issues || []) if (issue.code !== 'output_existing') logs.push({level:issue.severity || 'warning',message:issue.message,path:issue.path});
  for (const group of plan.groups) for (const issue of groupProblems(group)) logs.push({level:issue.severity || 'warning',message:`${group.relative_path || group.name}：${issue.message}`});
  if (plan.index_error) logs.push({level:'warning',message:`索引未生效：${plan.index_error}，当前使用文件夹名。`});
  for (const file of allFiles(plan).filter(autoTouched)) {
    const change = appliedAutoChange(file); logs.push({level:'info',code:'auto_rotation_change',message:`${file.relative_path}：自动调整 ${change.from_cw}° → ${change.to_cw}°。`});
  }
  const orientationJob = [S.job,...S.jobs].find(job => job?.kind === 'batch_analyze' && job.plan_id === plan.id && job.orientation_summary);
  if (orientationJob) logs.push({level:'info',code:'orientation_summary',message:orientationSummaryText(orientationJob)});
  else {
    const failures = allFiles(plan).filter(file => imageFile(file) && file.orientation_error && !file.manual_rotation).length;
    if (failures) logs.push({level:'info',code:'orientation_summary',message:`${fmt(failures)} 张图片方向识别未完成，未因该结果调整方向。`});
  }
  logs.push({level:'info',message:`当前 ${fmt(plan.groups.filter(g => !g.empty).length)} 个合并文件夹，${fmt(allFiles(plan).length)} 个文件，共 ${fmt(allFiles(plan).reduce((n,f) => n + Number(f.pages || 0),0))} 页。`});
  for (const rule of plan.exclusions || []) if (rule.matches?.length) logs.push({level:'info',message:`排除“${rule.rule}”，命中 ${fmt(rule.matches.length)} 个文件或目录。`});
  if (plan.ignored_files) logs.push({level:'info',message:`已跳过 ${fmt(plan.ignored_files)} 个非支持格式或辅助文件。`});
  if (plan.removed_sources?.length) logs.push({level:'info',message:`本次合并已移除 ${fmt(plan.removed_sources.length)} 个文件，原文件保留，可在文件列表下方撤销。`});
  logs.push(...S.events);
  const seen = new Set(); return logs.filter(log => { const key = `${log.level}:${log.message}:${log.path || ''}`; if (seen.has(key)) return false; seen.add(key); return true; });
}
function renderLogs() {
  const logs = currentLogs(); const counts = {all:logs.length,warning:0,error:0,info:0};
  for (const log of logs) if (log.level in counts) counts[log.level]++;
  for (const level of ['all','warning','error','info']) { $(`log-count-${level}`).textContent = fmt(counts[level]); $(`log-filter-${level}`).classList.toggle('active',S.logFilter === level); }
  $('summary-issues').textContent = fmt(counts.error + counts.warning);
  $('scan-log-summary').textContent = counts.error ? `${fmt(counts.error)} 个错误 · ${fmt(counts.warning)} 个警告` : counts.warning ? `${fmt(counts.warning)} 个警告` : '当前无错误';
  const box = $('scan-issues'); box.replaceChildren();
  for (const log of logs.filter(log => S.logFilter === 'all' || log.level === S.logFilter)) {
    const row = el('div',`scan-log-row ${log.level}`); row.append(el('span','log-level',{info:'信息',warning:'警告',error:'错误'}[log.level] || '信息'),el('span','scan-log-message',log.message + (log.path ? ` · ${prettyPath(log.path)}` : ''))); box.append(row);
  }
  if (!box.children.length) box.append(el('div','empty-pane','此分类没有日志。'));
  const metrics = S.scan || S.plan?.scan_metrics;
  $('scan-timing').hidden = !metrics?.timings;
  if (metrics?.timings) {
    const t = metrics.timings; const counts = metrics.counts || {};
    $('scan-timing').textContent = `扫描用时 ${formatDuration(metrics.elapsed_ms)} · 图片 ${fmt(counts.image_files)} 张 · PDF ${fmt(counts.pdf_files)} 份。目录 ${formatDuration(t.metadata_ms)}，读取 ${formatDuration(t.read_ms)}，校验 ${formatDuration(t.hash_ms)}，PDF检查 ${formatDuration(t.pdf_ms)}，图片检查 ${formatDuration(t.image_ms)}。阶段数据为累计工时。`;
  }
}
function renderPlan() {
  const p = S.plan; if (!p) return;
  const files = allFiles(p); $('summary-groups').textContent = fmt(p.groups.length); $('summary-files').textContent = fmt(files.length);
  $('summary-pages').textContent = fmt(files.reduce((n,f) => n + Number(f.pages || 0),0)); $('summary-review').textContent = fmt(files.reduce((n,f) => n + pendingReview(f),0)); $('groups-count').textContent = fmt(p.groups.length);
  const box = $('scan-exclusions'); box.replaceChildren();
  for (const rule of p.exclusions || []) {
    const detail = el('details','exclusion-detail'); const title = rule.kind === 'output' ? '输出目录自动排除' : rule.kind === 'state' ? '处理记录自动排除' : '排除规则';
    detail.append(el('summary','',`${title}：${rule.rule} · ${fmt(rule.matches?.length)} 个命中项`)); const list = el('ul'); list.append(...(rule.matches || []).map(path => el('li','',path))); detail.append(list); box.append(detail);
  }
  if (p.empty_directories?.length) box.append(el('p','help',`当前范围包含 ${fmt(p.empty_directories.length)} 个空目录，不生成空 PDF。`));
  renderGroups(); renderFiles(); renderLogs(); renderPreview().catch(showError);
}
function renameDescription(group) {
  const rename = group.rename; if (!rename) return '';
  if (rename.message) return rename.message;
  return {folder:'按文件夹名命名',matched:'索引已匹配',unmatched:'未匹配索引，使用文件夹名',conflict:'索引名称冲突，使用文件夹名',empty:'索引名称为空，使用文件夹名'}[rename.status] || '';
}
function renderGroups() {
  if (!S.plan) return;
  const box = $('group-list'); const scroll = box.scrollTop; box.replaceChildren();
  const search = $('group-search').value.trim().toLocaleLowerCase();
  for (const group of S.plan.groups.filter(g => `${g.relative_path} ${g.output_name} ${groupDraft(g).output_name}`.toLocaleLowerCase().includes(search))) {
    const tone = groupTone(group); const row = el('div', `group-row tone-${tone}${group.id === S.groupId ? ' active' : ''}`); row.setAttribute('role','listitem'); row.dataset.groupId = group.id;
    const main = el('div', 'group-main'); const check = document.createElement('input'); check.type = 'checkbox'; check.value = group.id; check.checked = S.selected.has(group.id); check.setAttribute('aria-label', `选择合并单元 ${group.relative_path || group.name}`);
    check.addEventListener('change', () => { if (check.checked) S.selected.add(group.id); else S.selected.delete(group.id); controls(); });
    const open = el('button', 'group-open'); open.type = 'button'; open.title = group.relative_path || group.name;
    open.append(el('span', 'group-path', group.relative_path || basename(S.plan.root)), el('span','group-output', groupDraft(group).output_name));
    open.addEventListener('click', () => selectGroup(group.id)); main.append(check, open); row.append(main);
    const meta = el('div', 'group-meta'); meta.append(el('span','',`${fmt(group.files.length)} 文件 · ${fmt(group.pages)} 页`));
    if (group.shallow) meta.append(el('span', 'pill', '浅层直属'));
    const pending = group.files.reduce((n, f) => n + pendingReview(f), 0);
    if (pending) meta.append(el('span','pill warning',`${fmt(pending)} 方向提示`));
    if (group.empty) meta.append(el('span','pill','空单元'));
    const signatureWarning = group.issues?.find(issue => issue.code === 'pdf_signature');
    const signed = signatureWarning || group.files.some(file => file.pdf_has_signature);
    if (signed) meta.append(el('span','pill warning','含签名 PDF'));
    row.append(meta);
    if (signed) row.append(el('span','group-status warning',signatureWarning?.message || '按静态页面合并，不保留签名验证；原文件不变。'));
    const naming = renameDescription(group);
    if (naming) {
      const warning = group.rename && ['unmatched','conflict','empty'].includes(group.rename.status);
      row.append(el('span', `group-status${warning ? ' warning' : ''}`, naming));
    }
    if (group.id === S.groupId) {
      const edit = el('div', 'group-rename'); const name = document.createElement('input'); name.value = groupDraft(group).output_name; name.dataset.edit = 'name'; name.dataset.groupId = group.id; name.setAttribute('aria-label', `输出文件名 ${group.relative_path || group.name}`); name.title = '名称会在输入暂停或失去焦点后自动保存';
      name.addEventListener('input', () => { setDraft(group, {output_name:name.value}); open.querySelector('.group-output').textContent = name.value; updateGroupProblemViews(); scheduleEdits(); });
      name.addEventListener('change',() => scheduleEdits(0));
      const prefix = el('button', 'text-button', '加原名'); prefix.type = 'button'; prefix.dataset.edit = 'name'; prefix.title = '在当前名称前加入文件夹路径，区分同名输出';
      prefix.addEventListener('click', () => {
        const base = group.relative_path ? group.relative_path.replace(/[\\/]/g, '__') : basename(S.plan.root);
        const value = `${base}-${groupDraft(group).output_name}`; name.value = value; setDraft(group,{output_name:value}); open.querySelector('.group-output').textContent = value; updateGroupProblemViews(); scheduleEdits(0);
      });
      edit.append(name, prefix); row.append(edit);
    }
    const problems = el('ul','group-problems'); const errors = groupProblems(group).filter(i => i.severity === 'error'); problems.hidden = !errors.length;
    problems.append(...errors.map(issue => el('li','',issue.message))); row.append(problems);
    box.append(row);
  }
  if (!box.children.length) box.append(el('div','empty-pane',S.plan.groups.length ? '没有匹配的合并单元' : '没有可合并文件，请检查文件格式、层级和排除规则。'));
  box.scrollTop = scroll; controls();
}
function updateGroupProblemViews() {
  $('group-list').querySelectorAll('[data-group-id]').forEach(row => {
    if (!row.classList.contains('group-row')) return;
    const group = S.plan.groups.find(g => g.id === row.dataset.groupId); if (!group) return;
    row.className = `group-row tone-${groupTone(group)}${group.id === S.groupId ? ' active' : ''}`;
    const box = row.querySelector('.group-problems'); const errors = groupProblems(group).filter(i => i.severity === 'error'); box.hidden = !errors.length; box.replaceChildren(...errors.map(issue => el('li','',issue.message)));
  });
  renderLogs(); controls();
}
function selectGroup(id) {
  if (id === S.groupId) return;
  clearDragState();
  S.groupId = id; S.fileId = selectedGroup()?.files[0]?.id || null; resetSourceView();
  renderGroups(); renderFiles(); renderPreview().catch(showError); controls();
}
function resetSourceView() {
  S.sourcePage = 1; S.fit = true; S.zoom = 1;
  $('rotation-scope').value = pdfFile(selectedFile()) ? 'page' : 'file';
}
let sortingPointer = null;
function clearDragState() {
  const state = sortingPointer; sortingPointer = null;
  if (state?.frame) cancelAnimationFrame(state.frame);
  if (state) {
    try { if (state.handle.hasPointerCapture(state.pointerId)) state.handle.releasePointerCapture(state.pointerId); }
    catch (_) { /* A detached handle may already have lost pointer capture. */ }
  }
  const list = $('file-list'); list.classList.remove('pointer-sorting');
  list.querySelectorAll('.dragging,.drop-target').forEach(row => row.classList.remove('dragging','drop-target'));
}
function validSortPointer(state) {
  return sortingPointer === state && !busy() && !S.sourceDirty && !hasDrafts() && state.groupId === S.groupId && state.row.isConnected && $('file-list').contains(state.row) && state.handle.hasPointerCapture(state.pointerId) && Boolean(selectedGroup()?.files.some(file => file.id === state.fileId));
}
function pointerSortTarget(state,x,y) {
  const list = $('file-list'); const bounds = list.getBoundingClientRect();
  if (!Number.isFinite(x) || !Number.isFinite(y) || x < bounds.left || x >= bounds.right || y < bounds.top || y >= bounds.bottom) return null;
  const hit = document.elementFromPoint(x,y); const row = hit?.closest('.file-row');
  if (!row || !list.contains(row) || row.dataset.groupId !== state.groupId || !selectedGroup()?.files.some(file => file.id === row.dataset.fileId)) return null;
  return row;
}
function updateSortPointer(state,x,y) {
  state.x = x; state.y = y;
  const target = pointerSortTarget(state,x,y);
  if (state.target !== target) {
    state.target?.classList.remove('drop-target'); state.target = target;
    if (target && target !== state.row) target.classList.add('drop-target');
  }
  const list = $('file-list'); const bounds = list.getBoundingClientRect(); const max = Math.max(0,list.scrollHeight - list.clientHeight);
  let step = target && y < bounds.top + 28 ? -10 : target && y > bounds.bottom - 28 ? 10 : 0;
  if ((step < 0 && list.scrollTop <= 0) || (step > 0 && list.scrollTop >= max)) step = 0;
  state.scrollStep = step;
  if (step && !state.frame) state.frame = requestAnimationFrame(scrollSortPointer);
  if (!step && state.frame) { cancelAnimationFrame(state.frame); state.frame = null; }
}
function scrollSortPointer() {
  const state = sortingPointer; if (!state) return; state.frame = null;
  if (!state.dragging || !validSortPointer(state)) { clearDragState(); return; }
  const list = $('file-list'); const previous = list.scrollTop; const max = Math.max(0,list.scrollHeight - list.clientHeight);
  list.scrollTop = Math.min(max,Math.max(0,previous + state.scrollStep));
  if (list.scrollTop !== previous) updateSortPointer(state,state.x,state.y);
}
function beginSortPointer(event,handle,row,groupId,fileId) {
  if (!event.isPrimary || event.button !== 0 || !Number.isInteger(event.pointerId) || handle.disabled || busy() || S.sourceDirty || hasDrafts() || groupId !== S.groupId) return;
  clearDragState(); event.preventDefault();
  sortingPointer = {pointerId:event.pointerId,handle,row,groupId,fileId,startX:event.clientX,startY:event.clientY,x:event.clientX,y:event.clientY,dragging:false,target:null,frame:null,scrollStep:0};
  try { handle.setPointerCapture(event.pointerId); if (!handle.hasPointerCapture(event.pointerId)) clearDragState(); }
  catch (_) { clearDragState(); }
}
function moveSortPointer(event) {
  const state = sortingPointer; if (!state || event.pointerId !== state.pointerId) return;
  if (!validSortPointer(state)) { clearDragState(); return; }
  if (!state.dragging && Math.hypot(event.clientX - state.startX,event.clientY - state.startY) < 6) return;
  if (!state.dragging) { state.dragging = true; state.row.classList.add('dragging'); $('file-list').classList.add('pointer-sorting'); }
  event.preventDefault(); updateSortPointer(state,event.clientX,event.clientY);
}
function finishSortPointer(event) {
  const state = sortingPointer; if (!state || event.pointerId !== state.pointerId) return;
  const target = state.dragging && event.button === 0 && event.isPrimary && validSortPointer(state) ? pointerSortTarget(state,event.clientX,event.clientY) : null;
  const ids = target ? groupDraft(selectedGroup()).file_ids : [];
  const from = ids.indexOf(state.fileId); const to = ids.indexOf(target?.dataset.fileId);
  clearDragState();
  if (target && from >= 0 && to >= 0 && from !== to) { event.preventDefault(); moveFile(from,to).catch(showError); }
}
function cancelSortPointer(event) {
  if (sortingPointer?.pointerId === event.pointerId) clearDragState();
}
function renderFiles() {
  clearDragState(); const box = $('file-list'); const scroll = box.scrollTop; box.replaceChildren(); const group = selectedGroup();
  $('files-count').textContent = fmt(group?.files.length); $('selected-group-name').textContent = group?.relative_path || (group ? basename(S.plan.root) : '');
  $('selected-group-name').title = group?.relative_path || '';
  if (!group) { box.append(el('div','empty-pane','选择左栏文件夹，查看文件顺序。')); $('file-list-summary').textContent = '选择文件夹'; renderRemovedFiles(); controls(); return; }
  const sort = groupDraft(group).sort || group.sort || S.plan.sort || {mode:'name',descending:false};
  $('file-sort').value = S.drafts.get(group.id)?.file_ids ? 'manual' : sort.mode || 'name';
  if (group.sort?.mode === 'manual') $('file-sort').value = 'manual';
  $('sort-direction').value = sort.descending ? 'desc' : 'asc'; $('sort-separator').value = sort.separator || '';
  renderSortHint(); const files = orderedFiles(group); let first = 1;
  files.forEach((file,index) => {
    const status = fileStatus(file); const row = el('div',`file-row tone-${status.tone}${file.id === S.fileId ? ' active' : ''}`); row.setAttribute('role','listitem'); row.dataset.groupId = group.id; row.dataset.fileId = file.id;
    const line = el('div','file-line'); const position = el('button','file-position file-drag-handle',index + 1); position.type = 'button'; position.draggable = false; position.dataset.edit = 'drag'; position.title = '按住左侧手柄拖动，松开后保存顺序'; position.setAttribute('aria-label',`拖动调整顺序：${file.name}`);
    position.addEventListener('pointerdown',event => beginSortPointer(event,position,row,group.id,file.id));
    const select = el('button','file-select'); select.type = 'button'; select.title = file.relative_path;
    const pages = Number(file.pages || 0);
    select.append(el('span','file-name',file.name), el('span','file-meta',pages ? `${imageFile(file) ? '图片' : 'PDF'} · ${fmt(pages)} 页 · 合并后 ${fmt(first)}–${fmt(first + pages - 1)} 页` : `${imageFile(file) ? '图片' : 'PDF'} · 无法读取页数`));
    select.addEventListener('click', () => { S.fileId = file.id; resetSourceView(); renderFiles(); renderPreview().catch(showError); controls(); });
    line.append(position,select); row.append(line); first += Number(file.pages || 0);
    const bottom = el('div','file-bottom'); bottom.append(el('span','file-review-label',`${status.symbol} ${status.label}`));
    const moves = el('div','file-moves');
    for (const [label,text,to,limit] of [['置顶','置顶',0,index === 0],['上移','↑',index - 1,index === 0],['下移','↓',index + 1,index === files.length - 1]]) {
      const button = el('button',label === '置顶' ? 'to-top' : '',text); button.type = 'button'; button.dataset.edit = 'order'; button.dataset.limit = String(limit); button.setAttribute('aria-label',`${label} ${file.name}`);
      button.addEventListener('click',action(() => moveFile(index,to))); moves.append(button);
    }
    const remove = el('button','remove-file','移除'); remove.type = 'button'; remove.dataset.edit = 'remove'; remove.title = '从本次合并中移除，原文件保留'; remove.setAttribute('aria-label',`移除 ${file.name}，原文件保留`); remove.addEventListener('click',action(() => removeFile(file.id))); moves.append(remove);
    bottom.append(moves); row.append(bottom);
    if (file.error) row.append(el('p','file-error',file.error));
    if (file.pdf_has_signature) row.append(el('p','file-warning','签名 PDF：合并不保留签名验证，原文件不变。'));
    box.append(row);
  });
  if (!files.length) box.append(el('div','empty-pane','该单元为空，不生成空 PDF。'));
  $('file-list-summary').textContent = `${fmt(files.length)} 个文件 · ${fmt(group.pages)} 页`;
  renderRemovedFiles(); box.scrollTop = scroll; controls();
}
function renderSortHint() {
  const mode = $('file-sort').value; const segmented = ['prefix','suffix'].includes(mode); $('sort-separator').hidden = !segmented;
  const ordering = mode === 'modified' ? '按修改时间排序。' : segmented ? `按${mode === 'prefix' ? '第一段' : '最后一段'}自然排序，默认以 _、- 或空格分段。` : mode === 'manual' ? '当前为手动顺序。' : '自然顺序：2 在 10 前。';
  $('sort-hint').textContent = `${ordering}拖动左侧手柄，或使用置顶、上下移调整顺序。`;
}
async function moveFile(from,to) {
  const group = selectedGroup(); if (!group || busy() || S.sourceDirty) return;
  const ids = [...groupDraft(group).file_ids]; if (from < 0 || to < 0 || from >= ids.length || to >= ids.length || from === to) return;
  const [id] = ids.splice(from,1); ids.splice(to,0,id); setDraft(group,{file_ids:ids,sort:{mode:'manual',descending:false}}); renderFiles(); await saveEdits();
}
async function editPlan(body) {
  if (!S.plan || busy() || S.sourceDirty) return;
  return operation('正在保存调整…', async () => {
    const plan = await api(`/batch/plans/${S.plan.id}/edit`, {method:'POST',body});
    acceptPlan(plan,{preserveSelection:true}); return plan;
  });
}
function scheduleEdits(delay = 800) {
  clearTimeout(S.editTimer);
  S.editTimer = setTimeout(async () => {
    if (!hasDrafts() || S.sourceDirty) return;
    if (busy()) { scheduleEdits(200); return; }
    try { await saveEdits(); } catch (error) { showError(error); }
  },delay);
}
async function saveEdits() {
  if (!hasDrafts()) return;
  clearTimeout(S.editTimer);
  const focused = document.activeElement; const restore = focused?.dataset?.edit === 'name' ? {groupId:focused.dataset.groupId,start:focused.selectionStart,end:focused.selectionEnd} : null;
  const groups = [...S.drafts.values()].map(value => {
    const {sort,...rest} = value; return rest;
  });
  try { await editPlan({groups}); S.editError = false; }
  catch (error) { S.editError = true; throw error; }
  finally {
    controls();
    if (restore) { const input = $('group-list').querySelector(`input[data-group-id="${restore.groupId}"]`); if (input && !input.disabled) { input.focus({preventScroll:true}); input.setSelectionRange(restore.start,restore.end); } }
  }
}
async function removeFile(fileId) {
  if (hasDrafts()) await saveEdits();
  const file = allFiles(S.plan).find(f => f.id === fileId); if (!file) return;
  await editPlan({remove_file_ids:[fileId]}); S.events.push({level:'info',message:`已移除“${file.relative_path}”，原文件保留。`}); renderLogs(); toast('已从本次合并中移除，原文件保留，可在列表下方撤销。');
}
function renderRemovedFiles() {
  const removed = (S.plan?.removed_sources || []).filter(item => item.group_id === S.groupId);
  $('removed-files').hidden = !removed.length; $('removed-count').textContent = fmt(removed.length); const box = $('removed-file-list'); box.replaceChildren();
  for (const item of removed) {
    const row = el('div','removed-file-row'); const name = el('span','',item.file.name); name.title = item.file.relative_path; const restore = el('button','text-button','撤销移除'); restore.type = 'button'; restore.setAttribute('aria-label',`撤销移除 ${item.file.name}`);
    restore.addEventListener('click',action(async () => { if (hasDrafts()) await saveEdits(); await editPlan({restore_file_ids:[item.file.id]}); S.selected.add(item.group_id); S.events.push({level:'info',message:`已恢复“${item.file.relative_path}”到本次合并。`}); renderGroups(); renderLogs(); controls(); })); row.append(name,restore); box.append(row);
  }
}
async function applySort() {
  const group = selectedGroup(); if (!group || $('file-sort').value === 'manual') return;
  const sort = {mode:$('file-sort').value,descending:$('sort-direction').value === 'desc',separator:$('sort-separator').value || null};
  const changed = {group_id:group.id,sort};
  if (S.drafts.has(group.id)) changed.output_name = groupDraft(group).output_name;
  const others = [...S.drafts.values()].filter(value => value.group_id !== group.id).map(({sort,...value}) => value);
  await editPlan({groups:[...others,changed]}); toast('已应用本单元的排序。');
}

function clearPreview() {
  S.previewEpoch++; S.renderEpoch++;
  if (S.previewUrl) URL.revokeObjectURL(S.previewUrl); S.previewUrl = null;
  if (S.pdfRender) { S.pdfRender.cancel(); S.pdfRender = null; }
  if (S.pdfTask) { const task = S.pdfTask; S.pdfTask = null; Promise.resolve(task.destroy()).catch(() => {}); }
  S.pdf = null; S.image = null; S.previewKey = null;
}
function previewMessage(message, error = false) {
  const box = el('div', `preview-placeholder${error ? ' error' : ''}`); box.append(el('span','','▤'),el('p','',message)); $('preview-surface').replaceChildren(box);
}
async function pdfLibrary() {
  if (!S.pdfModule) {
    S.pdfModule = import('/pdfjs/pdf.mjs').then(pdfjs => { pdfjs.GlobalWorkerOptions.workerSrc = '/pdfjs/pdf.worker.mjs'; return pdfjs; }).catch(error => { S.pdfModule = null; throw new Error(`PDF预览组件无法加载，请确认应用文件完整：${error.message}`); });
  }
  return S.pdfModule;
}
function previewFileKey(file) { return file ? `${file.id}:${file.sha256}:${file.kind}:${file.pages}` : null; }
function renderPreviewMeta() {
  const file = selectedFile(); $('preview-filename').textContent = file?.name || '选择文件以预览'; $('preview-filename').title = file?.relative_path || '';
  $('preview-kind').hidden = !file; $('preview-kind').textContent = imageFile(file) ? '图片' : 'PDF';
  $('source-page-number').value = S.sourcePage; $('source-page-number').max = file?.pages || 1; $('source-page-total').textContent = `/ ${fmt(file?.pages)}`;
  const status = file ? fileStatus(file) : null;
  $('preview-file-status').textContent = status ? `${status.symbol} ${status.label}` : '';
  $('preview-file-status').className = status ? ({review:'warning',error:'error',reviewed:'success',manual:'manual'}[status.tone] || '') : '';
  $('rotation-scope').value = pdfFile(file) ? $('rotation-scope').value : 'file';
  const angle = file ? (Number(file.rotation_cw || 0) + Number(pdfFile(file) ? file.page_rotations?.[S.sourcePage - 1] || 0 : 0)) % 360 : 0;
  $('rotation-note').textContent = angle ? `额外旋转 ${angle}°` : '旋转不改动原文件';
  $('preview-summary').textContent = !file ? '原件保留' : `${pdfFile(file) ? `第 ${fmt(S.sourcePage)} / ${fmt(file.pages)} 页` : '已应用照片自身方向信息'} · ${S.fit ? '适合窗口' : `${Math.round(S.zoom * 100)}%`}`;
  let evidence = '';
  if (file?.orientation_error) evidence = file.manual_rotation ? '先前自动识别未完成，当前使用您手动设置的角度。' : `识别未完成：${file.orientation_error}。可手动调整，也可按当前角度合并。`;
  else if (file?.orientation) evidence = `${file.manual_rotation ? '人工角度优先。' : '自动结果仅供参考。'}${file.orientation.title ? `文字证据：${file.orientation.title}。` : ''}${file.orientation.reason || ''}`;
  else if (pdfFile(file)) evidence = 'PDF保持原有页向；当前操作是在原有旋转上追加角度。自动方向识别仅处理图片。';
  else if (file) evidence = '可直接使用当前方向，也可手动调整；原文件保留，调整自动保存。';
  if (file?.pdf_has_signature) evidence = `签名 PDF 按静态页面合并，不保留签名验证；原文件不变。${evidence}`;
  $('preview-evidence').textContent = evidence; controls();
}
async function renderPreview() {
  const file = selectedFile(); renderPreviewMeta();
  if (!file) { clearPreview(); previewMessage('选择中间栏的文件，查看页面与方向。'); return; }
  if (file.error) { clearPreview(); previewMessage(file.error, true); return; }
  const key = previewFileKey(file);
  if (S.previewKey === key && (S.image || S.pdf)) { await drawPreview(); return; }
  clearPreview(); S.previewKey = key; const epoch = S.previewEpoch;
  previewMessage('正在读取本地原件…');
  try {
    const blob = await api(`/batch/plans/${S.plan.id}/files/${file.id}${imageFile(file) ? '?thumbnail=true' : ''}`, {blob:true});
    if (epoch !== S.previewEpoch || previewFileKey(selectedFile()) !== key) return;
    if (imageFile(file)) {
      S.previewUrl = URL.createObjectURL(blob); const image = new Image(); image.alt = file.name;
      await new Promise((resolve,reject) => { image.onload = resolve; image.onerror = () => reject(new Error('图片预览无法显示，请检查源文件。')); image.src = S.previewUrl; });
      if (epoch !== S.previewEpoch) return; S.image = image;
    } else {
      const pdfjs = await pdfLibrary(); if (epoch !== S.previewEpoch) return;
      const data = new Uint8Array(await blob.arrayBuffer()); if (epoch !== S.previewEpoch) return;
      const task = pdfjs.getDocument({data,cMapUrl:'/pdfjs/cmaps/',cMapPacked:true,standardFontDataUrl:'/pdfjs/standard_fonts/',wasmUrl:'/pdfjs/wasm/',enableXfa:false});
      S.pdfTask = task;
      const document = await task.promise; if (epoch !== S.previewEpoch) return; S.pdf = document;
      if (document.numPages !== file.pages) throw new Error('PDF预览页数与扫描不符，请重新扫描。');
    }
    await drawPreview();
  } catch (error) {
    if (epoch !== S.previewEpoch) return;
    previewMessage(error.message || '无法预览此文件，请检查应用与来源文件。', true);
  }
}
async function drawPreview() {
  const file = selectedFile(); if (!file || S.previewKey !== previewFileKey(file)) return;
  const stage = $('preview-surface'); const width = Math.max(80, stage.clientWidth - 36); const height = Math.max(80, stage.clientHeight - 36);
  const angle = (Number(file.rotation_cw || 0) + Number(pdfFile(file) ? file.page_rotations?.[S.sourcePage - 1] || 0 : 0)) % 360;
  const epoch = ++S.renderEpoch;
  if (S.pdfRender) { S.pdfRender.cancel(); S.pdfRender = null; }
  if (imageFile(file) && S.image) {
    const image = S.image; const quarter = angle % 180 !== 0;
    const originalWidth = quarter ? image.naturalHeight : image.naturalWidth;
    const originalHeight = quarter ? image.naturalWidth : image.naturalHeight;
    const scale = S.fit ? Math.min(width / originalWidth, height / originalHeight, 2) : S.zoom;
    if (S.fit) S.zoom = scale;
    const page = el('div','preview-page'); page.style.width = `${Math.ceil(originalWidth * scale)}px`; page.style.height = `${Math.ceil(originalHeight * scale)}px`;
    image.style.width = `${image.naturalWidth * scale}px`; image.style.height = `${image.naturalHeight * scale}px`; image.style.transform = `translate(-50%, -50%) rotate(${angle}deg)`;
    page.append(image); stage.replaceChildren(page);
  } else if (pdfFile(file) && S.pdf) {
    try {
      const page = await S.pdf.getPage(S.sourcePage); if (epoch !== S.renderEpoch) return;
      const rotation = (page.rotate + angle) % 360; const base = page.getViewport({scale:1,rotation});
      const scale = S.fit ? Math.min(width / base.width, height / base.height, 2) : S.zoom;
      if (S.fit) S.zoom = scale;
      const view = page.getViewport({scale,rotation}); const density = Math.min(window.devicePixelRatio || 1, 2);
      const canvas = document.createElement('canvas'); canvas.width = Math.ceil(view.width * density); canvas.height = Math.ceil(view.height * density);
      canvas.style.width = `${view.width}px`; canvas.style.height = `${view.height}px`; canvas.setAttribute('aria-label', `${file.name} 第 ${S.sourcePage} 页`);
      const container = el('div','preview-page'); container.style.width = `${view.width}px`; container.style.height = `${view.height}px`; container.append(canvas);
      stage.replaceChildren(container);
      const task = page.render({canvasContext:canvas.getContext('2d',{alpha:false}),viewport:view,transform:density === 1 ? undefined : [density,0,0,density,0,0]});
      S.pdfRender = task; await task.promise;
      if (S.pdfRender === task) S.pdfRender = null;
    } catch (error) {
      if (epoch !== S.renderEpoch || error.name === 'RenderingCancelledException') return;
      throw error;
    }
  }
  renderPreviewMeta();
}
async function navigateSourcePage(page) {
  const file = selectedFile(); if (!file) return;
  const value = Math.min(Math.max(1, Math.trunc(Number(page) || 1)), file.pages || 1);
  if (value === S.sourcePage) return; S.sourcePage = value; await drawPreview(); renderPreviewMeta();
}
async function rotateFile(delta, reset = false) {
  const file = selectedFile(); if (!file || busy() || S.sourceDirty || hasDrafts()) return;
  const change = {file_id:file.id};
  if (pdfFile(file) && $('rotation-scope').value === 'page') {
    const angles = Array.from({length:file.pages}, (_, i) => Number(file.page_rotations?.[i] || 0));
    angles[S.sourcePage - 1] = reset ? (360 - Number(file.rotation_cw || 0)) % 360 : (angles[S.sourcePage - 1] + delta + 360) % 360;
    change.page_rotations = angles;
  } else {
    change.rotation_cw = reset ? 0 : (Number(file.rotation_cw || 0) + delta + 360) % 360;
    if (reset && pdfFile(file)) change.page_rotations = Array(file.pages).fill(0);
  }
  await editPlan({files:[change]}); toast('方向已保存。');
}
function selectedImageCount() { return selectedGroups().reduce((n, group) => n + group.files.filter(file => imageFile(file) && !file.error).length, 0); }
function openOrientation(retry = null) {
  if (busy() || !S.plan || hasDrafts() || S.sourceDirty) return;
  const count = selectedImageCount(); if (!count) throw new Error('所选单元没有可识别图片；PDF请在预览中手动调整。');
  S.orientationRetry = retry;
  $('orientation-scope-summary').textContent = `将处理所选 ${fmt(selectedGroups().length)} 个合并单元中的 ${fmt(count)} 张图片。PDF页向不自动调整。`;
  $('orientation-consent').checked = false; $('orientation-threads').value = String(retry?.threads || S.orientationThreads);
  if (!S.status?.model_ready) {
    $('orientation-scope-summary').textContent += ` 方向模型当前不可用：${S.status?.model_error || '请检查随包组件'}。可取消此步骤，按当前方向合并。`;
  }
  $('orientation-dialog').showModal(); controls();
}
async function startOrientation() {
  if (busy() || !$('orientation-consent').checked || !S.status?.model_ready) return;
  const threads = Number($('orientation-threads').value);
  await operation('正在启动方向调整…', async () => {
    if (!S.orientationRetry) await saveThreadPreferences({orientation_threads:threads});
    const path = S.orientationRetry ? `/jobs/${S.orientationRetry.id}/retry` : `/batch/plans/${S.plan.id}/analyze`;
    const job = await api(path,{method:'POST',body:{accept_disclaimer:true,threads,group_ids:selectedGroups().map(g => g.id)}});
    const plan = await api(`/batch/plans/${job.plan_id}`); acceptPlan(plan,{preserveSelection:true});
    $('orientation-dialog').close(); S.orientationRetry = null; trackJob(job); toast(`方向调整已开始 · ${job.threads || threads} 线程`);
  });
}
function openMerge(retry = null) {
  if (busy() || !S.plan || hasDrafts() || S.sourceDirty) return;
  if (!selectedGroups().length) throw new Error('请先选择要合并的单元。');
  S.mergeRetry = retry; $('merge-dialog-title').textContent = retry ? '按原任务配置继续合并' : '准备生成 PDF';
  $('output-path').value = prettyPath(S.plan.output_dir); $('merge-threads').value = String(retry?.threads || S.mergeThreads);
  renderMergeSummary(); $('merge-dialog').showModal(); controls();
}
function renderMergeSummary() {
  if (!S.plan) return;
  const groups = selectedGroups(); const files = groups.flatMap(g => g.files);
  const pages = groups.reduce((n, g) => n + Number(g.pages || 0),0); const pending = files.reduce((n,f) => n + pendingReview(f),0);
  const summary = $('merge-summary'); summary.replaceChildren();
  const line = el('p'); line.append(document.createTextNode('本次生成 '),el('strong','',fmt(groups.length)),document.createTextNode(' 份 PDF，共 '),el('strong','',fmt(pages)),document.createTextNode(' 页。'));
  summary.append(line,el('p','help',`图片 ${fmt(files.filter(imageFile).length)} 张 · PDF ${fmt(files.filter(pdfFile).length)} 份 · 每个合并单元独立输出。`));
  if (S.mergeRetry) summary.append(el('p','help',`沿用原任务的计划、输出位置和 ${S.mergeRetry.threads || 4} 线程设置，已核验的成品可以复用。`));
  const warning = [];
  if (pending) warning.push(`本次有 ${fmt(pending)} 张图片的方向被自动调整。可以继续合并；请在交付前检查成品内容与方向。`);
  const autoErrors = files.filter(f => f.orientation_error && !f.manual_rotation).length;
  if (autoErrors) summary.append(el('p','help',`${fmt(autoErrors)} 张图片方向识别未完成，将使用当前方向。`));
  const signed = files.filter(file => file.pdf_has_signature).length;
  if (signed) warning.push(`${fmt(signed)} 份来源 PDF 含签名，本次仅合并静态页面，不保留签名验证。原文件保持不变。`);
  const renamedFallbacks = groups.filter(g => ['unmatched','conflict','empty'].includes(g.rename?.status)).length;
  if (renamedFallbacks) warning.push(`${fmt(renamedFallbacks)} 个单元使用文件夹名回退命名，详情见下方交付名称与左栏提示。`);
  $('merge-warning').hidden = !warning.length; $('merge-warning').textContent = warning.join(' ');
  const invalid = groups.filter(hasErrors);
  const errors = [];
  if (hasPlanErrors()) errors.push('资料范围存在扫描错误，请先处理后重新扫描。');
  if (invalid.length) errors.push(`${fmt(invalid.length)} 个所选单元存在错误或为空。请更改名称/输出位置，或返回取消选择有问题的单元；不会静默漏过来源。`);
  $('merge-errors').hidden = !errors.length; $('merge-errors').textContent = errors.join(' ');
  $('merge-name-list').replaceChildren(...groups.map(g => el('li','',`${g.output_name} · ${fmt(g.pages)} 页${hasErrors(g) ? ' · 需处理问题' : ''}`)));
  updateOutputDraft();
}
function updateOutputDraft() {
  const dirty = Boolean(S.plan && !samePath($('output-path').value.trim(), S.plan.output_dir));
  $('output-dirty-note').hidden = !dirty; $('check-output').hidden = !dirty; controls();
}
async function checkOutput() {
  const path = $('output-path').value.trim(); if (!path) throw new Error('请选择输出文件夹。');
  await editPlan({output_dir:path}); $('output-path').value = prettyPath(S.plan.output_dir); renderMergeSummary();
  toast('输出位置已预检，请核对当前单元、页数与名称后执行合并。');
}
async function startMerge() {
  if (busy() || !S.plan || hasDrafts() || S.sourceDirty || selectedGroups().some(hasErrors) || hasPlanErrors()) return;
  if (!samePath($('output-path').value.trim(), S.plan.output_dir)) throw new Error('输出位置已修改，请先更新预检。');
  const threads = Number($('merge-threads').value);
  await operation('正在启动合并…', async () => {
    if (!S.mergeRetry) await saveThreadPreferences({merge_threads:threads});
    const path = S.mergeRetry ? `/jobs/${S.mergeRetry.id}/retry` : `/batch/plans/${S.plan.id}/generate`;
    const job = await api(path,{method:'POST',body:{group_ids:selectedGroups().map(g => g.id),threads}});
    if (job.plan_id !== S.plan.id) acceptPlan(await api(`/batch/plans/${job.plan_id}`),{preserveSelection:true});
    $('merge-dialog').close(); S.mergeRetry = null; trackJob(job); toast(`合并已开始 · ${job.threads || threads} 线程`);
  });
}
function pendingOutputConflict(job = S.job) {
  return active(job) && job.kind === 'batch_generate' && S.cancelRequestedJobId !== job.id ? job.output_conflict || null : null;
}
function conflictIdentity(job = S.job) {
  const conflict = pendingOutputConflict(job); return conflict ? `${job.id}:${conflict.id}` : null;
}
function clearOutputConflict() {
  S.conflictKey = null; S.conflictBusy = false; S.conflictRequest++;
  $('output-conflict-apply').checked = false; $('output-conflict-error').hidden = true;
  if ($('output-conflict-dialog').open) $('output-conflict-dialog').close();
}
function conflictButtons() {
  const waiting = Boolean(S.conflictKey && S.conflictKey === conflictIdentity());
  const cancelling = S.cancelInFlightId === S.job?.id || S.cancelRequestedJobId === S.job?.id;
  for (const id of ['output-conflict-skip','output-conflict-overwrite','output-conflict-apply']) $(id).disabled = !waiting || S.conflictBusy || cancelling;
  $('output-conflict-cancel').disabled = !active(S.job) || cancelling;
  $('output-conflict-cancel').textContent = cancelling ? '正在取消…' : '取消任务';
}
function renderOutputConflict() {
  const conflict = pendingOutputConflict(); const key = conflictIdentity();
  if (!conflict || !key) { clearOutputConflict(); return; }
  if (S.conflictKey !== key) {
    S.conflictKey = key; S.conflictBusy = false; S.conflictRequest++;
    $('output-conflict-apply').checked = false; $('output-conflict-error').hidden = true;
    $('output-conflict-hint').textContent = '请明确选择跳过、覆盖或取消任务；已完成的其他 PDF 会保留。';
  }
  $('output-conflict-name').textContent = basename(conflict.output_path) || conflict.group_name;
  $('output-conflict-path').textContent = prettyPath(conflict.output_path);
  $('output-conflict-reason').textContent = conflict.reason;
  conflictButtons();
  if (!$('output-conflict-dialog').open) $('output-conflict-dialog').showModal();
}
async function resolveOutputConflict(action) {
  const job = S.job; const conflict = pendingOutputConflict(job); const key = conflictIdentity(job);
  if (!conflict || !key || S.conflictKey !== key || S.conflictBusy || S.cancelInFlightId === job.id || !['skip','overwrite'].includes(action)) return;
  S.conflictBusy = true; const request = ++S.conflictRequest;
  const apply = $('output-conflict-apply').checked; $('output-conflict-error').hidden = true; conflictButtons();
  try {
    const updated = await api(`/batch/jobs/${job.id}/output-conflict`,{method:'POST',body:{conflict_id:conflict.id,action,apply_to_batch:apply}});
    if (S.job?.id !== job.id || S.conflictKey !== key || S.conflictRequest !== request) return;
    trackJob(updated);
  } catch (error) {
    if (S.job?.id === job.id && S.conflictKey === key && S.conflictRequest === request) {
      $('output-conflict-error').textContent = `${error.message}。可重试当前选择，或取消任务。`; $('output-conflict-error').hidden = false;
    }
  } finally {
    if (S.conflictKey === key && S.conflictRequest === request) { S.conflictBusy = false; conflictButtons(); }
  }
}
function trackJob(job) {
  if (S.job?.id !== job.id) { S.cancelInFlightId = null; S.cancelRequestedJobId = null; }
  S.pollEpoch++; clearTimeout(S.polling); S.job = job; S.pollCount = 0;
  S.jobs = [job,...S.jobs.filter(j => j.id !== job.id)]; renderJob(); controls();
  if (active(job)) S.polling = setTimeout(pollJob,700);
}
async function pollJob() {
  if (!S.job) return;
  const id = S.job.id; const epoch = S.pollEpoch; S.polling = null;
  try {
    const job = await api(`/jobs/${id}`); if (S.job?.id !== id || S.pollEpoch !== epoch) return;
    S.job = job; S.pollCount++; S.jobs = [job,...S.jobs.filter(j => j.id !== id)];
    if (batchJob(job) && S.plan?.id === job.plan_id && (job.kind === 'batch_analyze' || !active(job))) {
      const plan = await api(`/batch/plans/${job.plan_id}`); if (S.job?.id !== id || S.pollEpoch !== epoch) return; acceptPlan(plan,{preserveSelection:true});
    }
    renderJob(); controls();
    if (!active(job)) {
      S.polling = null; await refreshHistory(false);
      toast(job.kind === 'batch_analyze' ? '方向调整已结束，可核对修正，也可按当前结果合并。' : labels[job.status] || '处理已结束');
      return;
    }
  } catch (error) { if (S.job?.id !== id || S.pollEpoch !== epoch) return; showError(error); }
  S.polling = setTimeout(pollJob,1200);
}
function renderJob() {
  const job = S.job; if (!job) return;
  $('job-panel').hidden = false; const analysis = ['batch_analyze','analyze'].includes(job.kind);
  $('job-title').textContent = analysis ? '图片方向调整' : '批量合并进度';
  $('job-status').textContent = labels[job.status] || job.status;
  $('job-status').className = `pill${analysis ? '' : job.status === 'failed' ? ' error' : ['completed_with_errors','cancelled','interrupted'].includes(job.status) ? ' warning' : ''}`;
  $('job-threads').textContent = `${job.threads || 4} 线程`;
  $('job-current').textContent = job.current || labels[job.status] || '准备中';
  $('job-progress-text').textContent = `${fmt(job.done)} / ${fmt(job.total)}${analysis ? ' 张' : ' 页'}`;
  $('job-progress').style.width = `${job.total ? Math.min(100,Number(job.done) / Number(job.total) * 100) : 0}%`;
  $('cancel-job').hidden = !active(job); $('cancel-job').disabled = S.cancelInFlightId === job.id || S.cancelRequestedJobId === job.id; $('retry-job').hidden = active(job) || job.status === 'completed' || !batchJob(job);
  $('save-report').hidden = active(job); $('job-error').hidden = !job.error; $('job-error').className = analysis ? 'notice neutral' : 'notice error'; $('job-error').textContent = job.error || '';
  const box = $('job-results'); box.replaceChildren();
  const successes = (job.results || []).filter(successfulOutput); const skippedExisting = (job.results || []).filter(result => result.status === 'skipped_existing').length;
  $('job-result-summary').textContent = analysis ? orientationSummaryText(job) : `已生成 ${fmt(successes.length)} 份 · 成功 ${fmt(successes.reduce((n,result) => n + Number(result.pages || 0),0))} 页 · 跳过同名 ${fmt(skippedExisting)} 份 · 失败 ${fmt((job.results || []).filter(result => result.status === 'failed').length)} 份`;
  const details = analysis ? (job.results || []).filter(result => rotationChange(result.rotation_change)) : job.results || [];
  for (const result of details) {
    if (analysis) {
      const change = rotationChange(result.rotation_change); const row = el('div','result-row orientation-result');
      row.append(el('span','',result.folder),el('span','',`${change.from_cw}° → ${change.to_cw}°`),el('span','','已调整')); box.append(row); continue;
    }
    const resultLabel = analysis && result.status === 'skipped' ? '保留人工角度或核对结果' : labels[result.status] || result.status;
    const row = el('div','result-row'); row.append(el('span','',result.folder),el('span','',analysis ? '' : `${fmt(result.pages)} 页`),el('span',result.status === 'failed' ? 'error-text' : '',resultLabel));
    if (successfulOutput(result)) {
      const save = el('button','text-button','保存 PDF'); save.type = 'button'; save.addEventListener('click',action(() => saveOutput(job,result))); row.append(save);
    } else row.append(el('span'));
    if (result.error) row.append(el('div','result-error',result.error));
    else if (result.needs_review) row.append(el('div','result-error warning-text','请检查成品内容与方向。'));
    box.append(row);
  }
  if (analysis && !details.length) box.append(el('p','help',job.orientation_summary ? '本次暂无实际调整的文件。' : '历史记录未保存明确的角度变化明细。'));
  $('open-output').hidden = analysis || !successes.length;
  renderOutputConflict();
}
function orientationSummaryText(job) {
  const summary = job.orientation_summary;
  const processed = `已处理 ${fmt(job.done)} / ${fmt(job.total)} 张`;
  return summary ? `${processed} · 调整 ${fmt(summary.changed)} · 未变 ${fmt(summary.unchanged)} · 跳过 ${fmt(summary.skipped)} · 失败 ${fmt(summary.failed)}` : `${processed} · 历史记录未保存分类汇总，仅显示明确的角度变化。`;
}
async function retryJob() {
  const job = S.job; if (!job || busy() || !batchJob(job) || hasDrafts() || S.sourceDirty) return;
  if (S.plan?.id !== job.plan_id) acceptPlan(await api(`/batch/plans/${job.plan_id}`),{syncForm:true,preserveSelection:false,collapse:true});
  const ids = job.request?.voucher_ids;
  S.selected = new Set(S.plan.groups.filter(g => !ids || ids.includes(g.id)).map(g => g.id)); renderGroups(); controls();
  if (job.kind === 'batch_analyze') openOrientation(job); else openMerge(job);
}
async function cancelJob() {
  const id = S.job?.id; if (!id || !active(S.job) || S.cancelInFlightId === id || S.cancelRequestedJobId === id) return;
  S.cancelInFlightId = id; S.conflictRequest++; conflictButtons(); $('cancel-job').disabled = true;
  try {
    const job = await api(`/jobs/${id}/cancel`,{method:'POST'}); if (S.job?.id !== id) return;
    S.cancelRequestedJobId = id; trackJob(job); toast('已请求取消，等待当前工作安全收尾；已完成的成品保留。');
  } catch (error) {
    if (S.job?.id !== id) return;
    if (S.conflictKey) { $('output-conflict-error').textContent = `取消任务失败：${error.message}。可重试取消。`; $('output-conflict-error').hidden = false; }
    else showError(error);
    S.conflictBusy = false;
  } finally {
    if (S.cancelInFlightId === id) S.cancelInFlightId = null;
    if (S.job?.id === id) renderJob();
  }
}
async function downloadBlob(path,name) {
  const blob = await api(path,{blob:true}); const url = URL.createObjectURL(blob);
  const link = el('a'); link.href = url; link.download = name; document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url),30000);
}
async function saveReport(job = S.job) {
  if (!job) return;
  const invoke = nativeInvoke();
  if (invoke) { const result = await invoke('save_job_report',{jobId:job.id}); if (result?.saved) toast('处理报告已保存。'); }
  else await downloadBlob(`/jobs/${job.id}/report`,`合并报告-${job.id}.json`);
}
async function saveOutput(job,result) {
  const invoke = nativeInvoke();
  if (invoke) { const saved = await invoke('save_job_output',{jobId:job.id,groupId:result.voucher_id}); if (saved?.saved) toast('PDF已保存。'); }
  else await downloadBlob(`/jobs/${job.id}/outputs/${result.voucher_id}`,basename(result.output_path));
}

async function refreshHistory(render = true) {
  if (S.historyLoading) return; S.historyLoading = true;
  try {
    const [plans,jobs] = await Promise.all([api('/batch/plans?summary=true'),api('/jobs')]);
    S.plans = plans; S.jobs = jobs; $('history-count').textContent = fmt(historyFamilies(plans).length);
    if (render) renderHistory(); controls();
  } finally { S.historyLoading = false; }
}
function historyFamilies(plans) {
  const map = new Map(plans.map(plan => [plan.id,plan])); const families = new Map();
  for (const plan of plans) {
    let root = plan; const seen = new Set([root.id]);
    while (root.parent_plan_id && map.has(root.parent_plan_id) && !seen.has(root.parent_plan_id)) {
      root = map.get(root.parent_plan_id); seen.add(root.id);
    }
    if (!families.has(root.id)) families.set(root.id,{latest:plan,ids:new Set()});
    const family = families.get(root.id); family.ids.add(plan.id);
    if (plan.created_at > family.latest.created_at) family.latest = plan;
  }
  return [...families.values()].sort((a,b) => b.latest.created_at.localeCompare(a.latest.created_at));
}
async function loadPlan(planId,job = null) {
  if (busy() && (!job || S.job?.id !== job.id)) throw new Error('当前任务仍在运行，请等待完成或取消后再切换资料。');
  if (hasDrafts() && !S.sourceDirty) await saveEdits();
  clearTimeout(S.editTimer); clearTimeout(S.previewTimer);
  const plan = await api(`/batch/plans/${planId}`); S.scan = null; S.events = []; S.renamePreview = null; S.renameOverrides.clear();
  if (!S.initialization || !samePath(S.initialization.root,plan.root) || Number(S.initialization.level) !== plan.level) {
    try { S.initialization = await api('/batch/initialize',{method:'POST',body:{root:plan.root,level:plan.level,exclude_rules:plan.exclude_rules}}); renderLevels(plan.level); renderDirectoryPreview(); }
    catch (_) { S.initialization = null; $('directory-preview').hidden = true; }
  }
  acceptPlan(plan,{syncForm:true,collapse:true,preserveSelection:false}); setView('batch'); $('scan-progress-panel').hidden = true;
  if (job) {
    const loaded = await api(`/jobs/${job.id}`); const ids = loaded.request?.voucher_ids;
    if (ids) { S.selected = new Set(ids); renderGroups(); }
    if (scanJob(loaded)) { S.scan = await api(`/batch/scans/${loaded.id}`); S.job = null; $('job-panel').hidden = true; renderLogs(); }
    else trackJob(loaded);
  } else { clearTimeout(S.polling); S.job = null; $('job-panel').hidden = true; }
  controls();
}
function renderHistory() {
  const box = $('history-list'); box.replaceChildren(); const families = historyFamilies(S.plans);
  if (!families.length) box.append(el('div','card empty-pane','还没有批量合并记录。扫描后，您的资料范围和调整会保存在这里。'));
  for (const family of families) {
    const plan = family.latest; const card = el('article','card history-card'); const info = el('div');
    info.append(el('h2','',basename(plan.root)),el('p','',prettyPath(plan.root)));
    const meta = el('div','history-meta');
    meta.append(el('span','',`${fmt(plan.group_count ?? plan.groups?.length)} 个合并文件夹`),el('span','',`${fmt(plan.pages ?? plan.groups?.reduce((n,g) => n + Number(g.pages || 0),0))} 页`),el('span','',plan.level ? `第 ${plan.level} 级分组` : '当前文件夹'),el('span','',plan.index ? '使用重命名索引' : '按文件夹名命名'));
    info.append(meta,el('small','',shortTime(plan.created_at)));
    const jobs = S.jobs.filter(job => batchJob(job) && family.ids.has(job.plan_id));
    if (jobs.length) {
      const taskBox = el('div','history-tasks');
      for (const job of jobs.slice(0,12)) {
        const row = el('div','history-task'); const content = el('div');
        content.append(el('span','',jobKindLabel(job)),el('span',`pill${['failed','completed_with_errors'].includes(job.status) ? ' warning' : ''}`,labels[job.status] || job.status),el('small','',`${shortTime(job.created_at)} · ${job.threads || 4} 线程`));
        const open = el('button','text-button','查看任务'); open.type = 'button'; open.addEventListener('click',action(() => loadPlan(job.plan_id,job)));
        row.append(content,open); taskBox.append(row);
      }
      info.append(taskBox);
    }
    const resume = el('button','secondary-button compact','继续整理'); resume.type = 'button'; resume.addEventListener('click',action(() => loadPlan(plan.id)));
    card.append(info,resume); box.append(card);
  }
  const legacy = S.jobs.filter(job => !batchJob(job)); $('legacy-history').hidden = !legacy.length;
  const legacyBox = $('legacy-history-list'); legacyBox.replaceChildren();
  for (const job of legacy.slice(0,50)) {
    const item = el('article','legacy-job'); item.append(el('p','',`${job.kind === 'analyze' ? '旧版方向识别' : '旧版合并'} · ${labels[job.status] || job.status}`),el('small','',`${shortTime(job.created_at)} · ${job.threads || 1} 线程 · ${fmt(job.done)} / ${fmt(job.total)}`));
    const actions = el('div','legacy-result-actions');
    if (!active(job)) { const report = el('button','text-button','保存原报告'); report.type = 'button'; report.addEventListener('click',action(() => saveReport(job))); actions.append(report); }
    for (const result of job.results || []) {
      if (!result.output_path || result.status === 'failed') continue;
      const save = el('button','text-button',`保存 ${basename(result.output_path)}`); save.type = 'button'; save.addEventListener('click',action(() => saveOutput(job,result))); actions.append(save);
    }
    if (active(job)) item.append(el('p','help','该旧版任务正在运行；新批量任务暂不能同时启动。'));
    item.append(actions); legacyBox.append(item);
  }
}
async function syncGlobal() {
  if (S.syncing || S.operations || active(S.scan) || active(S.job)) return; S.syncing = true;
  try {
    S.status = await api('/status');
    const runningJobs = S.status.running_jobs;
    const jobs = Array.isArray(runningJobs) ? [...runningJobs,...S.jobs.filter(job => !active(job) && !runningJobs.some(running => running.id === job.id))] : await api('/jobs');
    S.jobs = jobs;
    const running = jobs.find(job => active(job) && batchJob(job));
    if (scanJob(running)) await followScan(await api(`/batch/scans/${running.id}`),{syncForm:true});
    else if (running && !active(S.job)) {
      trackJob(running);
      if (!S.plan) acceptPlan(await api(`/batch/plans/${running.plan_id}`),{syncForm:true,collapse:true,preserveSelection:false});
    }
    controls();
  } catch (_) { /* Foreground operations display actionable connection errors. */ }
  finally { S.syncing = false; }
}
async function browseDirectory(path) {
  $('folder-error').hidden = true; $('confirm-folder').disabled = true;
  try {
    const result = await api('/fs' + (path ? '?path=' + encodeURIComponent(path) : ''));
    S.browser.path = result.path; S.browser.parent = result.parent; S.browser.selected = null;
    $('folder-path').value = prettyPath(result.path); $('folder-selection').textContent = prettyPath(result.path); $('folder-up').disabled = !result.parent;
    const box = $('folder-entries'); box.replaceChildren();
    const entries = result.entries.filter(entry => entry.is_dir || S.browser.kind === 'index');
    for (const entry of entries) {
      const button = el('button','folder-entry'); button.type = 'button';
      button.append(el('span','',entry.is_dir ? '▣' : '▤'),el('span','',entry.name));
      button.addEventListener('click',() => {
        if (entry.is_dir) browseDirectory(entry.path);
        else {
          S.browser.selected = entry.path; $('folder-selection').textContent = prettyPath(entry.path); $('confirm-folder').disabled = false;
          box.querySelectorAll('.selected').forEach(node => node.classList.remove('selected')); button.classList.add('selected');
        }
      });
      box.append(button);
    }
    if (!entries.length) box.append(el('div','empty-pane',S.browser.kind === 'index' ? '本目录没有表格文件，可进入其他目录或手动填写路径。' : '没有下级文件夹，可以选择当前目录。'));
    $('confirm-folder').disabled = S.browser.kind === 'index';
  } catch (error) { $('folder-error').textContent = error.message; $('folder-error').hidden = false; }
}
async function choosePath(kind, target) {
  if (busy()) return; const invoke = nativeInvoke(); const initial = $(target).value.trim() || $('root-path').value.trim() || null;
  if (invoke) {
    const value = await invoke(kind === 'index' ? 'select_index_file' : 'select_folder',{initialPath:initial});
    if (!value) return; $(target).value = prettyPath(value);
    if (target === 'root-path') { S.renameOverrides.clear(); changedSource(); await refreshPreparation(); }
    else if (target === 'index-path') { changedSource(); await inspectIndex(); }
    else updateOutputDraft();
    return;
  }
  S.browser = {kind,target,path:'',parent:null,selected:null}; $('folder-browser-title').textContent = kind === 'index' ? '选择重命名索引文件' : target === 'output-path' ? '选择输出文件夹' : '选择目标文件夹';
  $('confirm-folder').textContent = kind === 'index' ? '选择此索引文件' : '选择此文件夹'; $('folder-browser').showModal();
  let directory = initial;
  if (kind === 'index' && directory && /\.xlsx?$/i.test(directory)) directory = directory.replace(/[\\/][^\\/]+$/, '');
  await browseDirectory(directory);
}
async function confirmBrowserSelection() {
  const value = S.browser.kind === 'index' ? S.browser.selected : S.browser.path; if (!value) return;
  const target = S.browser.target; $(target).value = prettyPath(value); $('folder-browser').close();
  if (target === 'root-path') { S.renameOverrides.clear(); changedSource(); await refreshPreparation(); }
  else if (target === 'index-path') { changedSource(); await inspectIndex(); }
  else updateOutputDraft();
}
function threadOptions(select) {
  select.replaceChildren(...Array.from({length:10},(_, index) => { const option = el('option','',`${index + 1} 线程`); option.value = index + 1; return option; })); select.value = '4';
}
function applyThreadPreferences(preferences) {
  const orientation = Number(preferences.orientation_threads); const merge = Number(preferences.merge_threads);
  if (![orientation,merge].every(value => Number.isInteger(value) && value >= 1 && value <= 10)) throw new Error('线程偏好无效，应为 1–10 的整数。');
  S.orientationThreads = orientation; S.mergeThreads = merge;
}
async function saveThreadPreferences(changes) {
  const preferences = {orientation_threads:S.orientationThreads,merge_threads:S.mergeThreads,...changes};
  const saved = await api('/batch/preferences',{method:'PATCH',body:preferences}); applyThreadPreferences(saved);
}
async function initialize() {
  threadOptions($('orientation-threads')); threadOptions($('merge-threads')); controls();
  if (!token) { $('connection').classList.add('offline'); $('connection').replaceChildren(el('i'),el('span','','缺少会话')); showError(new Error('请从 PDF工作台应用或开发启动入口打开此页面，以取得本机访问会话。')); return; }
  try {
    const status = await api('/status'); S.status = status; S.ready = true;
    $('app-version').textContent = `v${status.version}`; $('connection').replaceChildren(el('i'),el('span','','本机已连接'));
    try { applyThreadPreferences(await api('/batch/preferences')); }
    catch (error) { showError(new Error(`无法读取线程偏好，当前使用默认 4 线程。${error.message}`)); }
    await refreshHistory(false);
    const running = S.jobs.find(job => active(job) && batchJob(job));
    if (scanJob(running)) await followScan(await api(`/batch/scans/${running.id}`),{syncForm:true});
    else if (running) { acceptPlan(await api(`/batch/plans/${running.plan_id}`),{syncForm:true,collapse:true,preserveSelection:false}); trackJob(running); }
    setInterval(syncGlobal,4000); controls();
  } catch (error) { $('connection').classList.add('offline'); $('connection').replaceChildren(el('i'),el('span','','连接失败')); showError(error); }
}

$('tab-batch').addEventListener('click',() => setView('batch'));
$('tab-history').addEventListener('click',() => setView('history'));
document.querySelector('.brand').addEventListener('click',event => { event.preventDefault(); setView('batch'); });
$('toggle-preparation').addEventListener('click',() => setPreparationCollapsed(!S.preparationCollapsed));
$('source-form').addEventListener('submit',action(async event => { event.preventDefault(); await scan(); }));
$('root-path').addEventListener('input',changedSource);
$('root-path').addEventListener('change',action(async () => { S.renameOverrides.clear(); changedSource(); if ($('root-path').value.trim()) await refreshPreparation({level:0}); }));
$('choose-root').addEventListener('click',action(() => choosePath('folder','root-path')));
$('merge-level').addEventListener('change',action(async () => { S.renameOverrides.clear(); changedSource(); await refreshPreparation({level:Number($('merge-level').value)}); }));
$('exclude-rules').addEventListener('input',changedSource);
$('exclude-rules').addEventListener('change',action(async () => { if ($('root-path').value.trim()) await refreshPreparation({level:Number($('merge-level').value)}); }));
$('reset-rules').addEventListener('click',action(async () => { $('exclude-rules').value = DEFAULT_RULES; changedSource(); if ($('root-path').value.trim()) await refreshPreparation({level:Number($('merge-level').value)}); }));
$('rename-enabled').addEventListener('change',action(async () => { S.renameOverrides.clear(); changedSource(); if ($('rename-enabled').checked && $('index-path').value.trim()) await inspectIndex(); else { S.renamePreview = null; renderNamingPreview(); } }));
$('index-path').addEventListener('input',changedSource);
$('index-path').addEventListener('change',action(async () => { changedSource(); await inspectIndex(); }));
$('choose-index').addEventListener('click',action(() => choosePath('index','index-path')));
$('index-sheet').addEventListener('change',action(() => inspectIndex({sheet:$('index-sheet').value})));
$('header-row').addEventListener('change',action(() => inspectIndex({sheet:$('index-sheet').value,headerRow:Number($('header-row').value)})));
for (const id of ['index-column','name-column','match-rule','naming-mode']) $(id).addEventListener('change',scheduleNamingPreview);
$('refresh-names').addEventListener('click',action(async () => { if (!S.indexInspection?.columns?.length) await inspectIndex({sheet:$('index-sheet').value,headerRow:Number($('header-row').value) || 1}); else await refreshNamingPreview(); }));
$('show-issues').addEventListener('click',() => { $('scan-details').open = !$('scan-details').open; if ($('scan-details').open) $('scan-details').scrollIntoView({block:'nearest',behavior:'smooth'}); });
for (const level of ['all','warning','error','info']) $(`log-filter-${level}`).addEventListener('click',() => { S.logFilter = level; renderLogs(); });
$('cancel-scan').addEventListener('click',action(cancelScan));
$('group-search').addEventListener('input',() => renderGroups());
$('select-all-groups').addEventListener('change',() => {
  S.selected = new Set($('select-all-groups').checked ? S.plan.groups.filter(group => !group.empty && group.files.length).map(group => group.id) : []); renderGroups(); controls();
});
$('deselect-errors').addEventListener('click',() => { S.selected = new Set(selectedGroups().filter(group => !hasErrors(group)).map(group => group.id)); renderGroups(); controls(); });
$('save-edits').addEventListener('click',action(saveEdits));
document.addEventListener('pointermove',moveSortPointer,true);
document.addEventListener('pointerup',finishSortPointer,true);
for (const type of ['pointercancel','lostpointercapture']) document.addEventListener(type,cancelSortPointer,true);
document.addEventListener('keydown',event => { if (event.key === 'Escape') clearDragState(); },true);
document.addEventListener('visibilitychange',() => { if (document.hidden) clearDragState(); });
window.addEventListener('blur',clearDragState);
$('file-sort').addEventListener('change',renderSortHint);
$('apply-sort').addEventListener('click',action(applySort));
$('rotation-scope').addEventListener('change',renderPreviewMeta);
$('previous-source-page').addEventListener('click',action(() => navigateSourcePage(S.sourcePage - 1)));
$('next-source-page').addEventListener('click',action(() => navigateSourcePage(S.sourcePage + 1)));
$('source-page-number').addEventListener('change',action(() => navigateSourcePage($('source-page-number').value)));
$('zoom-out').addEventListener('click',action(async () => { S.fit = false; S.zoom = Math.max(.15,S.zoom / 1.25); await drawPreview(); }));
$('zoom-in').addEventListener('click',action(async () => { S.fit = false; S.zoom = Math.min(5,S.zoom * 1.25); await drawPreview(); }));
$('fit-preview').addEventListener('click',action(async () => { S.fit = true; await drawPreview(); }));
$('rotate-left').addEventListener('click',action(() => rotateFile(270)));
$('rotate-right').addEventListener('click',action(() => rotateFile(90)));
$('rotate-half').addEventListener('click',action(() => rotateFile(180)));
$('reset-rotation').addEventListener('click',action(() => rotateFile(0,true)));
$('auto-orient').addEventListener('click',action(() => openOrientation()));
$('orientation-consent').addEventListener('change',controls);
$('start-orientation').addEventListener('click',action(startOrientation));
$('close-orientation').addEventListener('click',() => $('orientation-dialog').close());
$('orientation-dialog').addEventListener('close',() => { S.orientationRetry = null; controls(); });
$('merge-button').addEventListener('click',action(() => openMerge()));
$('start-merge').addEventListener('click',action(startMerge));
$('close-merge').addEventListener('click',() => $('merge-dialog').close());
$('merge-dialog').addEventListener('close',() => { S.mergeRetry = null; controls(); });
$('output-conflict-skip').addEventListener('click',action(() => resolveOutputConflict('skip')));
$('output-conflict-overwrite').addEventListener('click',action(() => resolveOutputConflict('overwrite')));
$('output-conflict-cancel').addEventListener('click',action(cancelJob));
$('output-conflict-dialog').addEventListener('cancel',event => { event.preventDefault(); $('output-conflict-hint').textContent = '此任务仍在等待选择。请点击跳过、覆盖或取消任务；Esc 不会关闭此提示。'; });
$('output-conflict-dialog').addEventListener('close',() => { if (S.conflictKey && S.conflictKey === conflictIdentity()) renderOutputConflict(); });
$('output-path').addEventListener('input',updateOutputDraft);
$('choose-output').addEventListener('click',action(() => choosePath('folder','output-path')));
$('check-output').addEventListener('click',action(checkOutput));
$('cancel-job').addEventListener('click',action(cancelJob));
$('retry-job').addEventListener('click',action(retryJob));
$('save-report').addEventListener('click',action(() => saveReport()));
$('open-output').addEventListener('click',action(async () => { if (S.job) { await api(`/batch/plans/${S.job.plan_id}/open-output`,{method:'POST',body:{}}); toast('已打开输出文件夹。'); } }));
$('refresh-history').addEventListener('click',action(() => refreshHistory()));
$('close-folder-browser').addEventListener('click',() => $('folder-browser').close());
$('folder-up').addEventListener('click',() => browseDirectory(S.browser.parent));
$('folder-path-form').addEventListener('submit',event => { event.preventDefault(); browseDirectory($('folder-path').value.trim()); });
$('confirm-folder').addEventListener('click',action(confirmBrowserSelection));
let resizeTimer;
new ResizeObserver(() => { if (S.fit && (S.image || S.pdf)) { clearTimeout(resizeTimer); resizeTimer = setTimeout(() => drawPreview().catch(showError),100); } }).observe($('preview-surface'));
window.addEventListener('beforeunload',() => { clearTimeout(S.polling); clearDragState(); clearPreview(); });
initialize();
