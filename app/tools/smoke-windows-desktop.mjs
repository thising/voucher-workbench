// W3C WebDriver against the actual shipped Tauri EXE and WebView2. No browser
// stand-in, mock IPC, embedded test server, or test-only application build.
import assert from 'node:assert/strict';
import {spawn, execFile} from 'node:child_process';
import {promisify} from 'node:util';
import fs from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import crypto from 'node:crypto';

const exec = promisify(execFile);
const [exe, fixtureTool, tools, work, evidence] = process.argv.slice(2);
assert(exe && fixtureTool && tools && work && evidence, 'Desktop smoke arguments are required.');
const expectedVersion=JSON.parse(await fs.readFile(path.join(path.dirname(exe),'package-manifest.json'),'utf8')).version;
const nativeScript = path.join(import.meta.dirname, 'windows-desktop-native.ps1');
const report = {schema_version:1, synthetic_only:true, native_window:true, external_browser:false,
  version:expectedVersion, checks:[], artifacts:['desktop-window.png'], customer_windows10:'pending'};
const owned = new Set();
const childProcesses = new Set();
let driver, session, driverPort;
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn, label, timeout=40000) {
  const end = Date.now()+timeout;
  while (Date.now()<end) {
    try { const value=await fn(); if(value) return value; } catch (_) { /* bounded startup/poll race */ }
    await wait(150);
  }
  throw new Error(`Timed out: ${label}`);
}
async function freePort() {
  const listener=net.createServer(); await new Promise(resolve=>listener.listen(0,'127.0.0.1',resolve));
  const port=listener.address().port; await new Promise(resolve=>listener.close(resolve)); return port;
}
async function wd(method, endpoint, body) {
  const response=await fetch(`http://127.0.0.1:${driverPort}${endpoint}`,{
    method,headers:{'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),
    signal:AbortSignal.timeout(180000)});
  const result=await response.json();
  if(!response.ok || result.value?.error) {
    // Never echo native driver messages or a document URL: a failing WebView
    // attach may include an authentication fragment in diagnostic text.
    throw new Error(`Native WebDriver ${method} failed (${response.status}, ${String(result.value?.error||'unknown').replace(/[^a-z -]/gi,'')}).`);
  }
  return result.value;
}
async function js(script,...args) { return wd('POST',`/session/${session}/execute/sync`,{script,args}); }
async function asyncJs(script,...args) { return wd('POST',`/session/${session}/execute/async`,{script,args}); }
async function api(endpoint,body,method=body===undefined?'GET':'POST') {
  const result=await asyncJs(`const done=arguments[arguments.length-1];api(arguments[0],{method:arguments[2],body:arguments[1]??undefined}).then(value=>done({ok:true,value}),()=>done({ok:false}));`,endpoint,body??null,method);
  assert(result.ok,`Authenticated workbench API rejected ${endpoint.replace(/[a-z0-9]{20,}/gi,'<id>')}.`);
  return result.value;
}
async function click(selector) { await js(`const n=document.querySelector(arguments[0]);if(!n||n.disabled)throw Error('control unavailable');n.click();`,selector); }
async function input(selector,value,event='input') {
  await js(`const n=document.querySelector(arguments[0]);n.value=arguments[1];n.dispatchEvent(new Event(arguments[2],{bubbles:true}));`,selector,String(value),event);
}
async function probe(directory) { return JSON.parse(await fs.readFile(path.join(directory,'desktop-probe.json'),'utf8')); }
async function native(pid,action,extra=[]) {
  const result=await exec('pwsh',['-NoProfile','-File',nativeScript,'-OwnedProcessId',String(pid),'-Action',action,...extra],{windowsHide:true,timeout:35000,maxBuffer:100000});
  assert(JSON.parse(result.stdout).controlled,'Owned native control did not confirm its action.');
}
function start(args) {
  const child=spawn(exe,args,{stdio:['ignore','pipe','pipe'],windowsHide:false});
  childProcesses.add(child);
  // Drain without persisting/printing. A headless legacy instance intentionally
  // prints a session fragment; it remains only in this anonymous pipe.
  child.stdout.resume();child.stderr.resume();
  return child;
}
async function ended(child,expected=0,timeout=30000) {
  await until(()=>child.exitCode!==null || child.signalCode!==null,'owned child exit',timeout);
  assert.equal(child.exitCode,expected,'Owned child returned an unexpected exit code.');
  childProcesses.delete(child);
}
async function exited(pid) {
  await until(()=>{try{process.kill(pid,0);return false;}catch{return true;}},'safe desktop exit',60000);
  owned.delete(pid);
}
async function open(state,probeDirectory,extra=[]) {
  const result=await wd('POST','/session',{capabilities:{alwaysMatch:{'tauri:options':{
    application:exe,args:['--state-dir',state,'--test-probe-dir',probeDirectory,...extra]}}}});
  session=result.sessionId;
  assert(session,'Native WebDriver did not return a session.');
  await wd('POST',`/session/${session}/timeouts`,{script:90000,pageLoad:90000,implicit:0});
  const status=await until(async()=>{const p=await probe(probeDirectory);return p.webview_loaded && p.window_visible && p;},'real workbench window loaded',90000);
  owned.add(status.pid);
  await until(()=>js(`return document.querySelector('#app-version')?.textContent.includes(arguments[0]) && typeof window.__TAURI__?.core?.invoke==='function';`,expectedVersion),'authenticated native UI connected',90000);
  return status.pid;
}
async function endSession() { if(session){try{await wd('DELETE',`/session/${session}`);}catch{}session=null;} }
function hash(bytes) { return crypto.createHash('sha256').update(bytes).digest('hex'); }
async function screenshot(name) {
  const png=await wd('GET',`/session/${session}/screenshot`);
  await fs.writeFile(path.join(evidence,name),Buffer.from(png,'base64'));
}
async function nativeSave(pid,command,args,destination) {
  const pending=asyncJs(`const done=arguments[arguments.length-1];window.__TAURI__.core.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value}),()=>done({ok:false}));`,command,args);
  await native(pid,'save',['-SavePath',destination,'-ExpectedText','另存处理结果']);
  return pending;
}

await fs.mkdir(evidence,{recursive:true});
await fs.mkdir(work,{recursive:true});
try {
  driverPort=await freePort(); let nativePort=await freePort();while(nativePort===driverPort)nativePort=await freePort();
  driver=spawn(path.join(tools,'bin/tauri-driver.exe'),['--port',String(driverPort),'--native-port',String(nativePort),'--native-driver',path.join(tools,'edge/msedgedriver.exe')],{stdio:'ignore',windowsHide:true});
  await until(async()=>{const r=await fetch(`http://127.0.0.1:${driverPort}/status`);return r.ok;},'native driver listening',30000);
  const fixtures=path.join(work,'合成资料 含空格');
  const generated=JSON.parse((await exec(fixtureTool,['create',fixtures],{timeout:30000})).stdout);
  assert(generated.synthetic,'Only a synthetic fixture may be used.');
  const state=path.join(work,'处理记录 含空格'); const probes=path.join(work,'原生窗口探针');
  const pid=await open(state,probes);
  report.checks.push('shipped-gui-exe-chinese-space-path-native-webview2');

  // Exact remote origin capability and native pickers, with cancellation through
  // the real Windows file dialogs owned by this process.
  for(const command of ['select_folder','select_index_file']) {
    const pending=asyncJs(`const done=arguments[arguments.length-1];window.__TAURI__.core.invoke(arguments[0],{}).then(value=>done({ok:true,value}),()=>done({ok:false}));`,command);
    await native(pid,'cancel'); const result=await pending;
    assert(result.ok && result.value===null,'A real native picker did not cancel cleanly.');
  }
  report.checks.push('real-native-folder-and-index-picker-cancellation');
  const origin=await js(`return location.origin;`);
  await js(`location.href='https://example.invalid/blocked-navigation';`);
  await wait(300); assert.equal(await js(`return location.origin;`),origin,'Remote navigation was not blocked.');
  assert.equal(await js(`window.open('https://example.invalid/blocked-popup');return true;`),true);
  assert.equal((await wd('GET',`/session/${session}/window/handles`)).length,1,'Unexpected additional desktop window.');
  report.checks.push('remote-navigation-and-new-window-blocked');

  await input('#root-path',path.join(fixtures,'合成分项'));
  await click('#initialize-folder');
  await until(()=>js(`return !document.querySelector('#merge-level').disabled && document.querySelector('#merge-level').options.length>=2;`),'actual folder levels');
  await input('#merge-level','1','change');
  await click('#scan-button');
  await until(()=>js(`return document.querySelector('#summary-groups').textContent==='2' && document.querySelectorAll('#file-list .file-row').length===3;`),'three-pane scan');
  assert.equal(await js(`return document.querySelector('#summary-pages').textContent;`),'8');
  report.checks.push('real-ui-folder-depth-scan-image-and-pdf-three-pane');
  await js(`Array.from(document.querySelectorAll('#file-list .file-select')).find(n=>n.textContent.includes('.pdf')).click();`);
  await until(()=>js(`const c=document.querySelector('#preview-surface canvas');return !!c && c.width>0 && c.height>0;`),'offline PDF.js renders native PDF',60000);
  await input('#source-page-number',2,'change');
  await click('#rotate-right');
  await until(async()=>{const plans=await api('/batch/plans');const p=plans.find(p=>p.groups.some(g=>g.files.some(f=>f.kind==='pdf'&&f.page_rotations?.[1]===90)));return p;},'PDF manual page-two rotation');
  assert.equal(await js(`return document.querySelector('#rotation-scope').value;`),'page');
  assert.equal(await js(`return document.querySelector('#reviewed-file').checked;`),false);
  await screenshot('desktop-window.png');
  report.checks.push('offline-pdfjs-real-canvas-and-per-page-manual-rotation');

  // Reorder and rename through the actual UI, then merge without a review gate.
  await js(`document.querySelectorAll('#file-list .to-top')[2].click();`);
  await input('#group-list .group-rename input','桌面合成成品.pdf');
  await click('#save-edits');
  await until(()=>js(`return document.querySelector('#save-edits').hidden;`),'manual edits saved');
  await click('#merge-button');
  await until(()=>js(`return document.querySelector('#merge-dialog').open;`),'merge dialog');
  await input('#merge-threads',3,'change');
  await click('#start-merge');
  const completed=await until(async()=>{const jobs=await api('/jobs');const j=jobs.find(j=>j.kind==='batch_generate');return j && j.status==='completed' && j;},'desktop batch generation',90000);
  assert.equal(completed.threads,3);assert.equal(completed.results.length,2);
  assert.equal(completed.results.reduce((n,r)=>n+r.pages,0),8);
  const delivered=completed.results.find(r=>path.basename(r.output_path||'')==='桌面合成成品.pdf');assert(delivered,'UI rename did not reach generated output.');
  const outputBytes=await fs.readFile(delivered.output_path);assert(outputBytes.subarray(0,4).equals(Buffer.from('%PDF')));
  report.checks.push('real-ui-manual-order-rename-threads-merge-without-review-gate');

  const savedPDF=path.join(work,'原生另存 副本.pdf');
  const copy=await nativeSave(pid,'save_job_output',{jobId:completed.id,groupId:delivered.voucher_id},savedPDF);
  assert(copy.ok && copy.value.saved);assert.equal(hash(await fs.readFile(savedPDF)),hash(outputBytes));
  const savedReport=path.join(work,'原生另存 报告.json');
  const saved=await nativeSave(pid,'save_job_report',{jobId:completed.id},savedReport);
  assert(saved.ok && saved.value.saved);assert.equal(JSON.parse(await fs.readFile(savedReport,'utf8')).job.id,completed.id);
  report.checks.push('real-native-save-pdf-and-report-verified-bytes');

  await native(pid,'minimize');
  const before=(await probe(probes)).activations;
  await ended(start(['--state-dir',state]));
  await until(async()=>{const p=await probe(probes);return p.activations>before && p.window_visible && p.window_minimized===false;},'duplicate activation restores existing window');
  assert.equal((await probe(probes)).pid,pid);
  report.checks.push('duplicate-launch-restores-original-window-and-exits-zero');

  // A bounded long OCR task allows proving activation/close semantics against
  // real work; each image is a copy of our generated bitmap, never customer data.
  const busyRoot=path.join(work,'合成后台任务');await fs.mkdir(busyRoot);
  const syntheticImage=path.join(fixtures,'合成分项','260101-00001','2601_00001_1.jpg');
  for(let i=0;i<256;i++) await fs.copyFile(syntheticImage,path.join(busyRoot,`image_${String(i).padStart(4,'0')}.jpg`));
  const busyPlan=await api('/batch/scan',{root:busyRoot,level:0});
  const busyJob=await api(`/batch/plans/${busyPlan.id}/analyze`,{accept_disclaimer:true,threads:1});
  await until(async()=>['queued','running'].includes((await api(`/jobs/${busyJob.id}`)).status),'actual OCR task active');
  const busyBefore=await api(`/jobs/${busyJob.id}`);const count=(await probe(probes)).activations;
  await ended(start(['--state-dir',state]));
  await until(async()=>(await probe(probes)).activations>count,'task-running activation');
  const busyAfter=await api(`/jobs/${busyJob.id}`);
  assert(['queued','running'].includes(busyAfter.status),'Synthetic OCR finished before the task-running activation could be verified.');
  assert(busyAfter.done>=busyBefore.done,'Repeated startup reset task progress.');
  await native(pid,'close');
  await native(pid,'cancel',['-ExpectedText','任务或文件保存仍在进行']);
  await until(async()=>(await probe(probes)).event==='close_cancelled_task_continues','native continue choice');
  assert(['queued','running'].includes((await api(`/jobs/${busyJob.id}`)).status),'Continue choice stopped the task.');
  await native(pid,'close');await native(pid,'accept',['-ExpectedText','安全停止并退出']);
  await exited(pid);await endSession();
  const stopped=JSON.parse(await fs.readFile(path.join(state,'jobs',`${busyJob.id}.json`),'utf8'));
  assert.equal(stopped.status,'cancelled','Safe close did not persist a cancelled task.');
  report.checks.push('real-ocr-task-activation-progress-continue-and-safe-close');

  let restarted=await open(state,probes);
  assert((await api('/jobs')).some(j=>j.id===completed.id),'Prior output history was lost.');
  assert.equal((await api('/batch/preferences')).merge_threads,3,'Batch thread preference was lost.');
  await native(restarted,'close');await exited(restarted);await endSession();
  report.checks.push('normal-restart-history-results-and-thread-preferences');
  restarted=await open(state,probes);
  process.kill(restarted,'SIGKILL');await exited(restarted);await endSession();
  restarted=await open(state,probes);await native(restarted,'close');await exited(restarted);await endSession();
  report.checks.push('abnormal-exit-restart-without-lock-file-deletion');

  // Startup races require no WebDriver session; the actual WebView still emits
  // only bounded readiness/visibility evidence, with no discovery URL or token.
  for(const [name,delay] of [['starting-instance',1800],['simultaneous-launch',0]]) {
    const d=path.join(work,name); const p=path.join(d,'probe');
    const one=start(['--state-dir',path.join(d,'state'),'--test-probe-dir',p,'--startup-delay-ms',String(delay)]);
    if(delay) await wait(150);
    const two=start(['--state-dir',path.join(d,'state'),'--test-probe-dir',p]);
    const ready=await until(async()=>{const s=await probe(p);return s.webview_loaded&&s.activations>=2&&s;},name,90000);
    owned.add(ready.pid);const secondary=one.pid===ready.pid?two:one;
    await ended(secondary);await native(ready.pid,'close');await exited(ready.pid);
    const primary=one.pid===ready.pid?one:two;await ended(primary);
    report.checks.push(name+'-one-real-window');
  }

  // A legacy HTTP instance uses protocol 1. Desktop protocol 2 must explain the
  // mismatch and must never ask that legacy process to open an external browser.
  const legacyState=path.join(work,'legacy-state');
  const legacy=start(['--headless','--no-browser','--state-dir',legacyState]);
  await until(async()=>{try{return (await fs.stat(path.join(legacyState,'instance.lock'))).isFile();}catch{return false;}},'legacy lock created');
  const blocked=start(['--state-dir',legacyState]);
  await native(blocked.pid,'error',['-ExpectedText','不同界面版本']);await ended(blocked,1);
  assert(legacy.exitCode===null,'The desktop ended the legacy process.');legacy.kill();await until(()=>legacy.exitCode!==null||legacy.signalCode!==null,'owned legacy cleanup');childProcesses.delete(legacy);
  report.checks.push('legacy-protocol-instance-explicit-error-without-activation');
  const missing=start(['--state-dir',path.join(work,'missing-webview-state'),'--test-force-missing-webview2']);
  await native(missing.pid,'error',['-ExpectedText','MicrosoftEdgeWebView2RuntimeInstallerX64.exe']);await ended(missing,1);
  report.checks.push('missing-webview2-same-native-error-path-and-instructions');
  const invalid=path.join(work,'state-is-a-file');await fs.writeFile(invalid,'synthetic');
  const failed=start(['--state-dir',invalid]);
  await native(failed.pid,'error',['-ExpectedText','无法启动']);await ended(failed,1);
  report.checks.push('state-directory-failure-visible-native-dialog');
  report.passed=true;
} catch(error) {
  report.passed=false;
  report.failure=String(error.message).replace(/#token=[^\s"<>]+/gi,'#token=[redacted]');
  throw new Error(report.failure);
} finally {
  await endSession();
  for(const child of childProcesses) if(child.exitCode===null && child.signalCode===null) child.kill();
  for(const pid of owned) {try{process.kill(pid,'SIGKILL');}catch{}}
  driver?.kill();
  await fs.writeFile(path.join(evidence,'desktop-smoke-report.json'),JSON.stringify(report,null,2));
}
console.log(`Native desktop verification passed (${report.checks.length} checks, synthetic inputs only).`);
