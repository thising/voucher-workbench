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
const active = job => job && ['queued', 'running'].includes(job.status);
const labels = {queued:'等待处理',running:'正在处理',completed:'已完成',completed_with_errors:'部分完成',cancelled:'已取消',interrupted:'意外中断',failed:'处理失败',skipped:'已核验，复用结果'};
const fmt = value => Number(value || 0).toLocaleString('zh-CN');
const basename = path => String(path || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop();
const timestamp = value => value ? new Date(value).toLocaleString('zh-CN', {hour12:false}) : '';
const lines = id => $(id).value.split(/\r?\n/).map(x => x.trim()).filter(Boolean);
const S = {
  plan:null, plans:[], jobs:[], job:null, groupId:null, selected:new Set(), drafts:new Map(),
  operation:false, ready:false, settings:{threads:1}, sourceDirty:false, orderDirty:false,
  previewUrl:null, previewRequest:0, polling:null, syncing:false, toastTimer:null,
  browser:{target:'root-path', path:'', parent:null}, generate:null
};
const busy = () => S.operation || active(S.job) || S.jobs.some(active);
const group = () => S.plan?.groups.find(g => g.id === S.groupId);
const blocked = g => g.empty || !g.files.length || g.issues.some(i => i.severity === 'error');
const planBlocked = () => S.plan?.issues.some(i => i.severity === 'error');
const chosen = () => S.plan?.groups.filter(g => S.selected.has(g.id) && !blocked(g)) || [];
function draft(g) {
  if (!S.drafts.has(g.id)) S.drafts.set(g.id, {file_ids:g.files.map(f => f.id), output_name:g.output_name});
  return S.drafts.get(g.id);
}
function ordered(g) {
  const lookup = new Map(g.files.map(f => [f.id, f]));
  return draft(g).file_ids.map(id => lookup.get(id));
}
async function api(path, options = {}) {
  const headers = new Headers(options.headers || {});
  headers.set('X-Session-Token', token);
  if (options.body && typeof options.body !== 'string') {
    options.body = JSON.stringify(options.body);
    headers.set('Content-Type', 'application/json');
  }
  let response;
  try { response = await fetch('/api' + path, {...options, headers, cache:'no-store'}); }
  catch (_) { throw new Error('无法连接本地服务，请确认程序窗口仍在运行。'); }
  if (!response.ok) {
    let message;
    try { message = (await response.json()).error; } catch (_) { /* non-JSON error */ }
    throw new Error(message || `请求失败（${response.status}），请从程序启动入口重新打开页面。`);
  }
  return options.blob ? response.blob() : response.json();
}
function showError(error) {
  $('global-alert').textContent = error.message || String(error);
  $('global-alert').hidden = false;
}
function action(fn) {
  return async event => {
    $('global-alert').hidden = true;
    try { await fn(event); } catch (error) { showError(error); } finally { controls(); }
  };
}
function toast(message) {
  clearTimeout(S.toastTimer);
  $('toast').textContent = message;
  $('toast').hidden = false;
  S.toastTimer = setTimeout(() => { $('toast').hidden = true; }, 4000);
}
function setView(view) {
  $('work-view').hidden = view !== 'work';
  $('history-view').hidden = view !== 'history';
  $('nav-work').classList.toggle('active', view === 'work');
  $('nav-history').classList.toggle('active', view === 'history');
  if (view === 'history') refreshHistory().catch(showError);
}
function controls() {
  const locked = busy() || !S.ready;
  $('source-form').querySelectorAll('input,select,textarea,button').forEach(node => { node.disabled = locked; });
  $('scan-button').textContent = S.operation ? '正在处理…' : '扫描并预览';
  $('processing-threads').disabled = locked;
  $('settings-status').textContent = !S.ready ? '正在读取设置…' : busy() ? '任务期间保持当前设置' : '已保存在本机';
  $('processing-threads').value = String(S.settings.threads);
  $('save-order').hidden = !S.orderDirty;
  $('save-order').disabled = locked || S.sourceDirty;
  const count = chosen().length;
  $('generate-button').textContent = count ? `生成所选 ${fmt(count)} 组` : '生成所选组';
  $('generate-button').disabled = locked || !count || S.sourceDirty || S.orderDirty || planBlocked();
  $('start-generate').disabled = locked;
  $('select-all').disabled = locked || S.sourceDirty;
  $('group-list').querySelectorAll('input').forEach(node => { const g = S.plan.groups.find(g => g.id === node.value); node.disabled = locked || S.sourceDirty || blocked(g); });
  $('group-detail').querySelectorAll('[data-edit]').forEach(node => { node.disabled = locked || S.sourceDirty || node.dataset.limit === 'true'; });
  $('group-detail').querySelectorAll('[draggable]').forEach(node => { node.draggable = !locked && !S.sourceDirty; });
  $('retry-job').disabled = locked || S.sourceDirty || S.orderDirty;
  const external = S.jobs.find(j => active(j) && j.kind !== 'generic_generate');
  $('external-job').hidden = !external;
  if (external) {
    const link = el('a', '', '前往凭证整理查看'); link.href = '/';
    $('external-job').replaceChildren(document.createTextNode('凭证任务正在运行，完成或取消后才能开始新任务和更改设置。'), link);
  }
  if (S.plan) {
    $('draft-alert').hidden = !S.sourceDirty && !S.orderDirty;
    $('draft-alert').textContent = S.sourceDirty ? '合并范围已修改。请重新扫描，核对新的分组后再生成。' : '顺序或输出名称尚未保存。保存后将生成新计划，历史计划保持原样。';
    $('stat-selected').textContent = `已选择 ${fmt(count)} 组`;
    const allowed = S.plan.groups.filter(g => !blocked(g));
    $('select-all').checked = allowed.length > 0 && allowed.every(g => S.selected.has(g.id));
    $('select-all').indeterminate = count > 0 && count < allowed.length;
  }
}
function issuesList(issues) {
  const list = el('ul', 'generic-issues');
  for (const issue of issues) {
    const item = el('li', issue.severity === 'error' ? 'error' : '', issue.message);
    if (issue.path) item.append(el('small', '', ` · ${issue.path}`));
    list.append(item);
  }
  return list;
}
function acceptPlan(plan, preserveSelection = false) {
  const selectedPaths = new Set(S.plan?.groups.filter(g => S.selected.has(g.id)).map(g => g.relative_path) || []);
  const oldPath = group()?.relative_path;
  S.plan = plan; S.drafts.clear(); S.orderDirty = false; S.sourceDirty = false;
  S.selected = new Set(plan.groups.filter(g => !blocked(g) && (!preserveSelection || selectedPaths.has(g.relative_path))).map(g => g.id));
  S.groupId = plan.groups.find(g => g.relative_path === oldPath)?.id || plan.groups[0]?.id || null;
  $('root-path').value = plan.root; $('output-path').value = plan.output_dir;
  $('merge-level').value = plan.level; $('merge-sort').value = plan.sort;
  $('exclude-paths').value = plan.exclude_paths.join('\n'); $('exclude-globs').value = plan.exclude_globs.join('\n');
  $('welcome').hidden = true; $('workspace').hidden = false;
  renderPlan(); controls();
}
function renderPlan() {
  const p = S.plan;
  const files = p.groups.flatMap(g => g.files);
  $('stat-groups').textContent = fmt(p.groups.length);
  $('stat-files').textContent = fmt(files.length);
  $('stat-pages').textContent = `${fmt(files.reduce((sum, f) => sum + f.pages, 0))} 页`;
  $('stat-issues').textContent = fmt(p.issues.length + p.groups.reduce((sum, g) => sum + g.issues.length, 0));
  const excluded = new Set(p.exclusions.flatMap(x => x.matches));
  $('stat-excluded').textContent = fmt(excluded.size + p.ignored_files);
  $('group-count').textContent = fmt(p.groups.length);
  $('plan-output').textContent = `输出到：${p.output_dir}`;
  const allIssues = [...p.issues, ...p.groups.flatMap(g => g.issues.map(i => ({...i, path:i.path || g.relative_path || g.name})))];
  $('scan-issues').replaceChildren(allIssues.length ? issuesList(allIssues) : el('p', 'generic-hint', '未发现扫描问题。生成前仍会重新核对文件指纹和输出归属。'));
  const details = $('scan-exclusions'); details.replaceChildren();
  for (const exclusion of p.exclusions) {
    const section = el('details'); section.append(el('summary', '', `${exclusion.kind === 'output' ? '输出目录自动排除' : '排除规则'}：${exclusion.rule} · ${fmt(exclusion.matches.length)} 个匹配项`));
    const paths = el('ul'); paths.append(...exclusion.matches.map(path => el('li', '', path))); section.append(paths); details.append(section);
  }
  if (p.ignored_files) details.append(el('p', 'generic-hint', `跳过 ${fmt(p.ignored_files)} 个非 PDF 或辅助文件。`));
  for (const [title, paths] of [['空目录', p.empty_directories]]) {
    if (!paths.length) continue;
    const section = el('details'); section.append(el('summary', '', `${title} · ${fmt(paths.length)}`));
    const list = el('ul'); list.append(...paths.map(path => el('li', '', typeof path === 'string' ? path : JSON.stringify(path)))); section.append(list); details.append(section);
  }
  renderGroups(); renderGroup();
}
function renderGroups() {
  const search = $('group-search').value.trim().toLocaleLowerCase();
  const list = $('group-list'); list.replaceChildren();
  for (const g of S.plan.groups.filter(g => `${g.relative_path} ${g.name}`.toLocaleLowerCase().includes(search))) {
    const row = el('div', 'generic-group-row' + (g.id === S.groupId ? ' active' : ''));
    const check = document.createElement('input'); check.type = 'checkbox'; check.value = g.id; check.checked = S.selected.has(g.id); check.setAttribute('aria-label', `选择分组 ${g.relative_path || g.name}`);
    check.addEventListener('change', () => { if (check.checked) S.selected.add(g.id); else S.selected.delete(g.id); controls(); });
    const button = el('button'); button.type = 'button'; button.append(el('strong', '', g.relative_path || '资料目录'), el('small', '', `${fmt(g.files.length)} 个 PDF · ${fmt(g.pages)} 页${g.shallow ? ' · 浅层分组' : ''}`));
    if (blocked(g)) button.append(el('small', 'error', g.empty ? '空分组' : '需处理问题'));
    button.addEventListener('click', () => { S.groupId = g.id; renderGroups(); renderGroup(); controls(); });
    row.append(check, button); list.append(row);
  }
  if (!list.children.length) list.append(el('p', 'empty-inline', S.plan.groups.length ? '没有匹配的分组' : '未找到可合并的 PDF，请检查范围与排除规则。'));
  controls();
}
function markOrder() { S.orderDirty = true; controls(); }
function moveFile(from, to) {
  if (busy() || S.sourceDirty) return;
  const d = draft(group());
  if (from === to || from < 0 || to < 0 || from >= d.file_ids.length || to >= d.file_ids.length) return;
  const [id] = d.file_ids.splice(from, 1); d.file_ids.splice(to, 0, id);
  markOrder(); renderGroup(); controls();
}
function renderGroup() {
  const box = $('group-detail'); box.replaceChildren(); const g = group();
  if (!g) { box.append(el('div', 'empty-inline', '选择分组查看文件')); return; }
  const heading = el('div', 'generic-detail-heading');
  const info = el('div'); info.append(el('div', 'overline', g.shallow ? '浅层分组 · 未达到指定层级' : '合并分组'), el('h2', '', g.relative_path || basename(S.plan.root)), el('p', 'generic-hint', `${fmt(g.files.length)} 个 PDF · ${fmt(g.pages)} 页`)); heading.append(info);
  box.append(heading);
  const nameLabel = el('label', 'generic-output-name'); nameLabel.append(el('span', '', '输出文件名（含 .pdf）'));
  const input = document.createElement('input'); input.value = draft(g).output_name; input.dataset.edit = 'name'; input.setAttribute('aria-label', '输出文件名');
  input.addEventListener('input', () => { draft(g).output_name = input.value; markOrder(); });
  nameLabel.append(input); box.append(nameLabel);
  if (g.issues.length) box.append(issuesList(g.issues));
  box.append(el('p', 'generic-detail-note', '按下列顺序合并；源 PDF 内部页序保持不变。可拖动文件，或使用 ↑ / ↓ 调整，修改后请保存。'));
  const list = el('ol', 'generic-file-list'); let start = 1;
  const files = ordered(g);
  files.forEach((file, index) => {
    const row = el('li', 'generic-file-row'); row.draggable = !busy();
    row.append(el('span', 'generic-file-index', index + 1));
    const details = el('div', 'generic-file-info', file.relative_path);
    details.append(el('small', '', file.pages ? `合并后第 ${fmt(start)}–${fmt(start + file.pages - 1)} 页 · ${fmt(file.pages)} 页 · ${(file.bytes / 1024 / 1024).toFixed(2)} MB` : '无法确定页数'));
    start += file.pages;
    if (file.error) details.append(el('small', 'error', file.error));
    const buttons = el('div', 'generic-file-actions');
    const up = el('button', 'icon-button', '↑'); up.type = 'button'; up.dataset.edit = 'up'; up.dataset.limit = String(index === 0); up.setAttribute('aria-label', `上移 ${file.name}`); up.addEventListener('click', () => moveFile(index, index - 1));
    const down = el('button', 'icon-button', '↓'); down.type = 'button'; down.dataset.edit = 'down'; down.dataset.limit = String(index === files.length - 1); down.setAttribute('aria-label', `下移 ${file.name}`); down.addEventListener('click', () => moveFile(index, index + 1));
    const preview = el('button', 'text-button', '预览'); preview.type = 'button'; preview.disabled = Boolean(file.error); preview.addEventListener('click', action(() => previewFile(file)));
    const exclude = el('button', 'text-button', '排除'); exclude.type = 'button'; exclude.dataset.edit = 'exclude'; exclude.addEventListener('click', () => {
      $('exclude-paths').value = [...new Set([...lines('exclude-paths'), file.relative_path])].join('\n');
      S.sourceDirty = true; controls(); toast('已加入排除路径，请重新扫描以更新分组。');
    });
    buttons.append(up, down, preview, exclude); row.append(details, buttons);
    row.addEventListener('dragstart', event => {
      if (busy() || S.sourceDirty) { event.preventDefault(); return; }
      event.dataTransfer.setData('text/plain', `${g.id}:${file.id}`); event.dataTransfer.effectAllowed = 'move'; row.classList.add('dragging');
    });
    row.addEventListener('dragend', () => { row.classList.remove('dragging'); list.querySelectorAll('.drop-target').forEach(n => n.classList.remove('drop-target')); });
    row.addEventListener('dragover', event => { if (!busy() && !S.sourceDirty) { event.preventDefault(); event.dataTransfer.dropEffect = 'move'; row.classList.add('drop-target'); } });
    row.addEventListener('dragleave', () => row.classList.remove('drop-target'));
    row.addEventListener('drop', event => {
      event.preventDefault(); row.classList.remove('drop-target');
      const value = event.dataTransfer.getData('text/plain');
      if (!value.startsWith(`${g.id}:`)) return;
      moveFile(draft(g).file_ids.indexOf(value.slice(g.id.length + 1)), index);
    });
    list.append(row);
  });
  box.append(list); controls();
}
async function scan() {
  if (busy()) return;
  const level = Number($('merge-level').value);
  if (!Number.isInteger(level) || level < 0 || level > 32) throw new Error('合并层级应为 0–32 的整数。');
  S.operation = true; controls();
  try {
    const sort = $('merge-sort').value === 'manual' ? 'natural_asc' : $('merge-sort').value;
    const plan = await api('/generic/scan', {method:'POST', body:{root:$('root-path').value.trim(), output_dir:$('output-path').value.trim() || null, level, exclude_paths:lines('exclude-paths'), exclude_globs:lines('exclude-globs'), sort}});
    acceptPlan(plan); S.job = null; $('job-panel').hidden = true;
    await refreshHistory(false); toast('扫描完成，请核对分组和文件顺序。');
  } finally { S.operation = false; controls(); }
}
async function saveOrder() {
  if (!S.plan || busy() || S.sourceDirty) return;
  S.operation = true; controls();
  try {
    const plan = await api(`/generic/plans/${S.plan.id}/order`, {method:'POST', body:{groups:S.plan.groups.map(g => ({group_id:g.id, ...draft(g)}))}});
    acceptPlan(plan, true); S.job = null; $('job-panel').hidden = true;
    await refreshHistory(false); toast('已保存新的合并计划，请核对后生成。');
  } finally { S.operation = false; controls(); }
}
function confirmGenerate(retry = null) {
  if (busy() || !S.plan || S.sourceDirty || S.orderDirty) return;
  const ids = retry ? retry.request?.voucher_ids || S.plan.groups.filter(g => !blocked(g)).map(g => g.id) : chosen().map(g => g.id);
  const groups = S.plan.groups.filter(g => ids.includes(g.id));
  if (!groups.length || planBlocked() || groups.some(blocked)) throw new Error('所选分组存在错误，请先处理问题并重新扫描。');
  const threads = retry?.threads || S.settings.threads;
  S.generate = {jobId:retry?.id, ids};
  const summary = $('generate-summary'); summary.replaceChildren();
  summary.append(el('p', 'summary-line', `本次合并 ${fmt(groups.length)} 组，共 ${fmt(groups.reduce((sum, g) => sum + g.pages, 0))} 页；并行上限 ${threads} 线程${retry ? '（沿用原任务）' : ''}。`), el('p', 'summary-line', `输出到：${S.plan.output_dir}`));
  const names = el('ul'); names.append(...groups.map(g => el('li', '', `${g.output_name} · ${fmt(g.pages)} 页`))); summary.append(names, el('p', 'summary-line', '请核对名称与顺序。来源不明或已被外部修改的同名文件不会覆盖；工具拥有的结果更新前会保留历史。'));
  $('generate-dialog').showModal(); controls();
}
async function startGenerate() {
  if (busy() || !S.generate || S.sourceDirty || S.orderDirty) return;
  S.operation = true; controls();
  try {
    const path = S.generate.jobId ? `/jobs/${S.generate.jobId}/retry` : `/generic/plans/${S.plan.id}/generate`;
    const job = await api(path, {method:'POST', body:{group_ids:S.generate.ids}});
    $('generate-dialog').close(); trackJob(job); toast(`合并任务已开始 · ${job.threads} 线程`);
  } finally { S.operation = false; controls(); }
}
function trackJob(job) {
  clearTimeout(S.polling); S.job = job;
  S.jobs = [job, ...S.jobs.filter(j => j.id !== job.id)];
  renderJob(); controls();
  if (active(job)) S.polling = setTimeout(pollJob, 700);
}
async function pollJob() {
  if (!S.job) return;
  try {
    const job = await api(`/jobs/${S.job.id}`); S.job = job;
    S.jobs = [job, ...S.jobs.filter(j => j.id !== job.id)]; renderJob(); controls();
    if (!active(job)) { S.polling = null; await refreshHistory(false); toast(labels[job.status] || '处理已结束'); return; }
  } catch (error) { showError(error); }
  S.polling = setTimeout(pollJob, 1200);
}
function renderJob() {
  const job = S.job; if (!job) return;
  $('job-panel').hidden = false; $('job-status').textContent = labels[job.status] || job.status;
  $('job-threads').textContent = `并行上限 ${job.threads || 1} 线程`;
  $('job-current').textContent = job.current || labels[job.status];
  $('job-progress-text').textContent = `${fmt(job.done)} / ${fmt(job.total)}`;
  $('job-progress').style.width = `${job.total ? Math.min(100, job.done / job.total * 100) : 0}%`;
  $('cancel-job').hidden = !active(job); $('retry-job').hidden = active(job) || job.status === 'completed';
  $('download-report').hidden = active(job); $('job-error').hidden = !job.error; $('job-error').textContent = job.error || '';
  const results = $('job-results'); results.replaceChildren();
  for (const result of job.results) {
    const row = el('div', 'result-row'); row.append(el('span', 'result-name', result.folder), el('span', '', `${fmt(result.pages)} 页`), el('span', result.status === 'failed' ? 'result-review' : '', labels[result.status] || result.status));
    if (result.output_path && result.status !== 'failed') {
      const download = el('button', 'text-button', '下载 PDF'); download.type = 'button';
      download.addEventListener('click', action(() => downloadBlob(`/jobs/${job.id}/outputs/${result.voucher_id}`, basename(result.output_path)))); row.append(download);
    } else row.append(el('span'));
    if (result.error) row.append(el('div', 'result-error', result.error)); results.append(row);
  }
  $('open-output').hidden = !job.results.some(r => r.output_path && r.status !== 'failed');
}
async function downloadBlob(path, name) {
  const blob = await api(path, {blob:true}); const url = URL.createObjectURL(blob);
  const link = el('a'); link.href = url; link.download = name; document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30000);
}
async function previewFile(file) {
  const request = ++S.previewRequest;
  $('preview-name').textContent = file.name; $('preview-stage').replaceChildren(el('p', 'empty-inline', '正在读取原件…')); $('preview-dialog').showModal();
  const blob = await api(`/generic/plans/${S.plan.id}/files/${file.id}`, {blob:true});
  if (request !== S.previewRequest || !$('preview-dialog').open) return;
  if (S.previewUrl) URL.revokeObjectURL(S.previewUrl); S.previewUrl = URL.createObjectURL(blob);
  const frame = document.createElement('iframe'); frame.title = file.name; frame.src = S.previewUrl;
  $('preview-stage').replaceChildren(frame);
}
async function refreshHistory(render = true) {
  const [plans, jobs] = await Promise.all([api('/generic/plans'), api('/jobs')]);
  S.plans = plans; S.jobs = jobs; $('history-count').textContent = fmt(plans.length);
  if (render) renderHistory(); controls();
}
async function loadPlan(plan, job = null) {
  if (busy() && (!job || S.job?.id !== job.id)) throw new Error('任务正在运行，请先等待完成或取消后再切换计划。');
  acceptPlan(await api(`/generic/plans/${plan.id}`)); setView('work');
  if (job) trackJob(await api(`/jobs/${job.id}`));
  else { S.job = null; $('job-panel').hidden = true; controls(); }
}
function renderHistory() {
  const list = $('history-list'); list.replaceChildren();
  if (!S.plans.length) list.append(el('div', 'card empty-inline', '还没有通用合并记录。扫描完成后，计划会保存在本机。'));
  for (const plan of S.plans) {
    const card = el('article', 'card history-card'); const info = el('div');
    info.append(el('h3', '', basename(plan.root)), el('p', '', plan.root), el('small', '', `${timestamp(plan.created_at)} · 第 ${plan.level} 层 · ${fmt(plan.groups.length)} 组 · ${fmt(plan.groups.reduce((n, g) => n + g.pages, 0))} 页`), el('p', 'generic-history-output', `输出：${plan.output_dir}`));
    const open = el('button', 'secondary-button compact', '查看计划'); open.type = 'button'; open.addEventListener('click', action(() => loadPlan(plan)));
    const jobs = S.jobs.filter(j => j.kind === 'generic_generate' && j.plan_id === plan.id);
    if (jobs.length) {
      const block = el('div', 'generic-history-jobs');
      for (const job of jobs) {
        const row = el('div', 'history-job'); row.append(el('span', '', `${labels[job.status]} · ${fmt(job.done)} / ${fmt(job.total)} · ${job.threads || 1} 线程`));
        const button = el('button', 'text-button', '查看任务'); button.type = 'button'; button.addEventListener('click', action(() => loadPlan(plan, job))); row.append(button); block.append(row);
      }
      info.append(block);
    }
    card.append(info, open); list.append(card);
  }
}
async function sync() {
  if (S.syncing || S.operation) return; S.syncing = true;
  try {
    const [jobs, settings] = await Promise.all([api('/jobs'), api('/settings')]);
    S.jobs = jobs; S.settings = settings;
    const running = jobs.find(j => active(j) && j.kind === 'generic_generate');
    if (running && !active(S.job)) {
      // An open draft stays visible; the background task remains independently tracked.
      trackJob(running);
      if (!S.plan) acceptPlan(await api(`/generic/plans/${running.plan_id}`));
    }
    controls();
  } catch (_) { /* foreground requests report connection errors */ }
  finally { S.syncing = false; }
}
async function browseDirectory(path) {
  $('browser-error').hidden = true;
  try {
    const result = await api('/fs' + (path ? '?path=' + encodeURIComponent(path) : ''));
    S.browser.path = result.path; S.browser.parent = result.parent;
    $('browser-path-input').value = result.path; $('browser-selected').textContent = result.path; $('browser-up').disabled = !result.parent;
    const entries = $('browser-entries'); entries.replaceChildren();
    for (const entry of result.entries.filter(e => e.is_dir)) {
      const button = el('button', 'browser-entry', entry.name); button.type = 'button'; button.addEventListener('click', () => browseDirectory(entry.path)); entries.append(button);
    }
    if (!entries.children.length) entries.append(el('p', 'empty-inline', '没有下级文件夹，可选择当前目录。'));
    $('browser-choose').disabled = false;
  } catch (error) { $('browser-error').textContent = error.message; $('browser-error').hidden = false; $('browser-choose').disabled = true; }
}
async function browse(target) {
  S.browser.target = target; $('browser-title').textContent = target === 'root-path' ? '选择资料文件夹' : '选择输出文件夹';
  $('browser-dialog').showModal(); await browseDirectory($(target).value.trim() || $('root-path').value.trim());
}
async function initialize() {
  $('processing-threads').replaceChildren(...Array.from({length:10}, (_, i) => { const option = el('option', '', `${i + 1} 线程`); option.value = i + 1; return option; }));
  controls();
  if (!token) { showError(new Error('请从程序启动入口重新打开工作台，以取得本机访问会话。')); return; }
  try {
    const [status, settings] = await Promise.all([api('/status'), api('/settings')]);
    S.settings = settings; S.ready = true;
    $('connection').replaceChildren(el('i'), document.createTextNode('本地服务已连接')); $('server-version').textContent = `v${status.version} · 离线工作台`;
    await refreshHistory(false);
    const running = S.jobs.find(j => active(j) && j.kind === 'generic_generate');
    if (running) { acceptPlan(await api(`/generic/plans/${running.plan_id}`)); trackJob(running); }
    setInterval(sync, 3500); controls();
  } catch (error) { $('connection').classList.add('offline'); $('connection').textContent = '连接失败'; showError(error); }
}
$('nav-work').addEventListener('click', () => setView('work'));
$('nav-history').addEventListener('click', () => setView('history'));
$('source-form').addEventListener('submit', action(async event => { event.preventDefault(); await scan(); }));
$('source-form').querySelectorAll('input,select,textarea').forEach(node => node.addEventListener('input', () => { if (S.plan) S.sourceDirty = true; controls(); }));
$('processing-threads').addEventListener('change', action(async () => {
  const threads = Number($('processing-threads').value); if (busy()) return;
  S.operation = true; controls();
  try { S.settings = await api('/settings', {method:'PATCH', body:{threads}}); toast('并行设置已保存'); }
  finally { S.operation = false; controls(); }
}));
$('select-all').addEventListener('change', () => { S.selected = new Set($('select-all').checked ? S.plan.groups.filter(g => !blocked(g)).map(g => g.id) : []); renderGroups(); controls(); });
$('group-search').addEventListener('input', () => { if (S.plan) renderGroups(); });
$('save-order').addEventListener('click', action(saveOrder));
$('generate-button').addEventListener('click', action(() => confirmGenerate()));
$('start-generate').addEventListener('click', action(startGenerate));
$('close-generate').addEventListener('click', () => $('generate-dialog').close());
$('cancel-job').addEventListener('click', action(async () => { if (S.job) { const job = await api(`/jobs/${S.job.id}/cancel`, {method:'POST'}); trackJob(job); toast('已请求取消，正在保存进度'); } }));
$('retry-job').addEventListener('click', action(async () => {
  if (!S.job || busy() || S.sourceDirty || S.orderDirty) return;
  if (S.plan?.id !== S.job.plan_id) acceptPlan(await api(`/generic/plans/${S.job.plan_id}`));
  confirmGenerate(S.job);
}));
$('download-report').addEventListener('click', action(() => downloadBlob(`/jobs/${S.job.id}/report`, `合并报告-${S.job.id}.json`)));
$('open-output').addEventListener('click', action(async () => { await api(`/generic/plans/${S.job.plan_id}/open-output`, {method:'POST', body:{}}); toast('已打开输出文件夹'); }));
$('refresh-history').addEventListener('click', action(() => refreshHistory()));
$('browse-root').addEventListener('click', action(() => browse('root-path')));
$('browse-output').addEventListener('click', action(() => browse('output-path')));
$('close-browser').addEventListener('click', () => $('browser-dialog').close());
$('browser-up').addEventListener('click', () => browseDirectory(S.browser.parent));
$('browser-path-form').addEventListener('submit', event => { event.preventDefault(); browseDirectory($('browser-path-input').value.trim()); });
$('browser-choose').addEventListener('click', () => { $(S.browser.target).value = S.browser.path; if (S.plan) S.sourceDirty = true; $('browser-dialog').close(); controls(); });
$('close-preview').addEventListener('click', () => $('preview-dialog').close());
$('preview-dialog').addEventListener('close', () => { S.previewRequest++; if (S.previewUrl) URL.revokeObjectURL(S.previewUrl); S.previewUrl = null; $('preview-stage').replaceChildren(); });
initialize();
