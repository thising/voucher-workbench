use crate::{model::*, orientation::Engine, pdf, workers};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

pub const MAX_THREADS: usize = 10;
fn default_threads() -> usize {
    1
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessingSettings {
    pub threads: usize,
}
impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            threads: default_threads(),
        }
    }
}
impl ProcessingSettings {
    fn validate(&self) -> Result<()> {
        if !(1..=MAX_THREADS).contains(&self.threads) {
            bail!("并行线程数必须是1到10之间的整数");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub voucher_ids: Option<Vec<String>>,
    #[serde(default)]
    pub allow_unreviewed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobResult {
    pub voucher_id: String,
    pub folder: String,
    pub status: String,
    pub output_path: Option<String>,
    pub pages: u32,
    pub error: Option<String>,
    pub needs_review: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub plan_id: String,
    pub kind: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub total: u32,
    pub done: u32,
    pub current: String,
    pub error: Option<String>,
    pub results: Vec<JobResult>,
    pub request: GenerateRequest,
    #[serde(default = "default_threads")]
    pub threads: usize,
}
impl Job {
    pub fn active(&self) -> bool {
        matches!(self.status.as_str(), "queued" | "running")
    }
}
pub struct Data {
    pub plans: HashMap<String, Plan>,
    pub jobs: HashMap<String, Job>,
    pub cancels: HashMap<String, Arc<AtomicBool>>,
    pub settings: ProcessingSettings,
}
pub struct AppState {
    pub data: Mutex<Data>,
    pub dir: PathBuf,
    pub token: String,
    pub engine: Mutex<Option<Engine>>,
    pub model_error: Option<String>,
    model_path: PathBuf,
    runtime_path: Option<PathBuf>,
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn recover_json_directory(dir: &Path) -> Result<()> {
    // A crash during the two-step Windows-compatible replacement may leave only .previous.
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|x| x.to_str()) == Some("previous") {
            let original = path.with_extension("");
            if !original.exists() {
                fs::rename(path, original)?;
            }
        }
    }
    Ok(())
}

impl AppState {
    pub fn load(dir: PathBuf, model: &Path, runtime: Option<&Path>) -> Result<Arc<Self>> {
        fs::create_dir_all(dir.join("plans"))?;
        fs::create_dir_all(dir.join("jobs"))?;
        fs::create_dir_all(dir.join("reports"))?;
        recover_json_directory(&dir.join("plans"))?;
        recover_json_directory(&dir.join("jobs"))?;
        recover_json_directory(&dir)?;
        let settings_path = dir.join("settings.json");
        let settings: ProcessingSettings = if settings_path.exists() {
            serde_json::from_slice(&fs::read(&settings_path)?)
                .context("并行设置文件损坏，请检查状态目录中的settings.json")?
        } else {
            ProcessingSettings::default()
        };
        settings.validate()?;
        let mut data = Data {
            plans: HashMap::new(),
            jobs: HashMap::new(),
            cancels: HashMap::new(),
            settings,
        };
        for entry in fs::read_dir(dir.join("plans"))? {
            let path = entry?.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let p: Plan = serde_json::from_slice(&fs::read(&path)?)
                .with_context(|| format!("计划文件损坏：{}", path.display()))?;
            data.plans.insert(p.id.clone(), p);
        }
        for entry in fs::read_dir(dir.join("jobs"))? {
            let path = entry?.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let mut j: Job = serde_json::from_slice(&fs::read(&path)?)
                .with_context(|| format!("任务文件损坏：{}", path.display()))?;
            ProcessingSettings { threads: j.threads }.validate()?;
            if j.active() {
                if j.kind == "analyze" {
                    if let Some(plan) = data.plans.get_mut(&j.plan_id) {
                        plan.orientation_workflow.analysis_completed = false;
                        pdf::atomic_json(
                            &dir.join("plans").join(format!("{}.json", plan.id)),
                            plan,
                        )?;
                    }
                }
                j.status = "interrupted".into();
                j.current = "服务上次退出时任务尚未完成，可点击重试".into();
                j.updated_at = now();
                pdf::atomic_json(&path, &j)?;
            }
            data.jobs.insert(j.id.clone(), j);
        }
        let (engine, error) = match Engine::new(model, runtime) {
            Ok(e) => (Some(e), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        Ok(Arc::new(Self {
            data: Mutex::new(data),
            dir,
            token: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            engine: Mutex::new(engine),
            model_error: error,
            model_path: model.to_owned(),
            runtime_path: runtime.map(Path::to_owned),
        }))
    }
    pub fn settings(&self) -> ProcessingSettings {
        self.data.lock().unwrap().settings
    }
    pub fn set_settings(&self, settings: ProcessingSettings) -> Result<ProcessingSettings> {
        settings.validate()?;
        let mut data = self.data.lock().unwrap();
        if data.jobs.values().any(Job::active) {
            bail!("任务运行中不能更改并行线程数，请先取消或等待完成");
        }
        pdf::atomic_json(&self.dir.join("settings.json"), &settings)?;
        data.settings = settings;
        Ok(settings)
    }
    pub fn save_plan(&self, p: &Plan) -> Result<()> {
        pdf::atomic_json(&self.dir.join("plans").join(format!("{}.json", p.id)), p)
    }
    fn save_job(&self, j: &Job) -> Result<()> {
        pdf::atomic_json(&self.dir.join("jobs").join(format!("{}.json", j.id)), j)
    }
    pub fn insert_plan(&self, p: Plan) -> Result<Plan> {
        self.save_plan(&p)?;
        self.data
            .lock()
            .unwrap()
            .plans
            .insert(p.id.clone(), p.clone());
        Ok(p)
    }
    pub fn plan(&self, id: &str) -> Result<Plan> {
        self.data
            .lock()
            .unwrap()
            .plans
            .get(id)
            .cloned()
            .context("计划不存在，请重新扫描")
    }
    pub fn job(&self, id: &str) -> Result<Job> {
        self.data
            .lock()
            .unwrap()
            .jobs
            .get(id)
            .cloned()
            .context("任务不存在")
    }
    fn update_job(&self, id: &str, update: impl FnOnce(&mut Job)) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        let j = data.jobs.get_mut(id).context("任务不存在")?;
        update(j);
        j.updated_at = now();
        self.save_job(j)
    }
    pub fn set_orientation_workflow(
        &self,
        id: &str,
        enabled: bool,
        accept_disclaimer: bool,
    ) -> Result<Plan> {
        let mut data = self.data.lock().unwrap();
        if data.jobs.values().any(|j| j.plan_id == id && j.active()) {
            bail!("任务运行中不能更改方向识别模式，请先取消或等待完成");
        }
        if enabled && !accept_disclaimer {
            bail!("启用方向识别须明确同意当前免责声明");
        }
        let mut plan = data.plans.get(id).cloned().context("计划不存在")?;
        let was_enabled = plan.orientation_workflow.enabled;
        let had_consent = plan.orientation_workflow.has_current_consent();
        if enabled {
            if !was_enabled || !had_consent {
                plan.orientation_workflow.analysis_completed = false;
                plan.orientation_workflow.consent_version =
                    Some(ORIENTATION_DISCLAIMER_VERSION.into());
                plan.orientation_workflow.consented_at = Some(now());
            }
        } else {
            plan.orientation_workflow.analysis_completed = false;
        }
        plan.orientation_workflow.enabled = enabled;
        for file in plan.vouchers.iter_mut().flat_map(|v| &mut v.files) {
            if file.kind == "image" && !file.reviewed {
                file.rotation_cw = if enabled {
                    file.orientation
                        .as_ref()
                        .filter(|o| crate::orientation::is_current(o))
                        .map(|o| o.rotation_cw)
                        .unwrap_or(0)
                } else {
                    0
                };
            }
        }
        self.save_plan(&plan)?;
        data.plans.insert(id.into(), plan.clone());
        Ok(plan)
    }
    pub fn edit_files(&self, id: &str, ids: &[String], rotation: Option<u16>) -> Result<Plan> {
        if rotation.is_some_and(|r| ![0, 90, 180, 270].contains(&r)) {
            bail!("旋转角度必须为0/90/180/270");
        }
        let mut data = self.data.lock().unwrap();
        if data.jobs.values().any(|j| j.plan_id == id && j.active()) {
            bail!("任务运行中不能修改或确认方向，请先取消或等待完成");
        }
        let mut plan = data.plans.get(id).cloned().context("计划不存在")?;
        if plan.orientation_workflow.enabled
            && (!plan.orientation_workflow.has_current_consent()
                || !plan.orientation_workflow.analysis_completed)
        {
            bail!("请先同意免责声明并完成方向识别，再人工核对修正和确认");
        }
        if ids.is_empty() {
            bail!("请明确选择要人工确认的页面");
        }
        for id in ids {
            if !plan
                .vouchers
                .iter()
                .flat_map(|v| &v.files)
                .any(|f| &f.id == id)
            {
                bail!("文件不属于此计划");
            }
        }
        let effective: HashMap<_, _> = plan
            .vouchers
            .iter()
            .flat_map(|v| &v.files)
            .filter(|file| ids.contains(&file.id))
            .map(|file| (file.id.clone(), pdf::effective_rotation(&plan, file)))
            .collect();
        for file in plan.vouchers.iter_mut().flat_map(|v| &mut v.files) {
            if ids.contains(&file.id) {
                file.rotation_cw = rotation.unwrap_or(effective[&file.id]);
                file.reviewed = true;
                if file
                    .error
                    .as_ref()
                    .is_some_and(|e| e.starts_with("方向识别失败："))
                {
                    file.error = None;
                }
            }
        }
        self.save_plan(&plan)?;
        data.plans.insert(id.into(), plan.clone());
        Ok(plan)
    }
    pub fn start(
        self: &Arc<Self>,
        plan_id: &str,
        kind: &str,
        request: GenerateRequest,
    ) -> Result<Job> {
        self.start_with_threads(plan_id, kind, request, None)
    }
    fn start_with_threads(
        self: &Arc<Self>,
        plan_id: &str,
        kind: &str,
        request: GenerateRequest,
        threads: Option<usize>,
    ) -> Result<Job> {
        let mut data = self.data.lock().unwrap();
        let threads = threads.unwrap_or(data.settings.threads);
        ProcessingSettings { threads }.validate()?;
        if !matches!(kind, "analyze" | "generate") {
            bail!("未知任务类型");
        }
        if request.allow_unreviewed {
            bail!("不再支持跳过人工确认生成，请先核对方向并确认");
        }
        if data.jobs.values().any(Job::active) {
            bail!("已有任务运行，请完成或取消后再启动");
        }
        let plan = data.plans.get(plan_id).context("计划不存在")?;
        if plan.issues.iter().any(|i| i.severity == "error") && kind == "generate" {
            bail!("资料目录存在扫描错误，请修正后重新扫描");
        }
        if kind == "analyze" {
            if !plan.orientation_workflow.enabled
                || !plan.orientation_workflow.has_current_consent()
            {
                bail!("请先启用标题方向识别并同意当前免责声明");
            }
            if plan
                .vouchers
                .iter()
                .flat_map(|v| &v.files)
                .any(pdf::needs_analysis)
                && self.model_error.is_some()
            {
                bail!(
                    "方向模型未就绪：{}",
                    self.model_error.as_deref().unwrap_or_default()
                );
            }
        } else {
            pdf::validate_generation_workflow(plan)?;
            if plan
                .vouchers
                .iter()
                .filter(|v| {
                    request
                        .voucher_ids
                        .as_ref()
                        .map(|ids| ids.contains(&v.id))
                        .unwrap_or(true)
                })
                .flat_map(|v| &v.files)
                .any(|f| pdf::requires_manual_review(plan, f))
            {
                bail!("所选凭证的所有图片均须在识别完成后人工核对修正并确认");
            }
        }
        if let Some(ids) = &request.voucher_ids {
            if ids.is_empty()
                || ids
                    .iter()
                    .any(|id| !plan.vouchers.iter().any(|v| &v.id == id))
            {
                bail!("请选择本计划中的有效凭证");
            }
        }
        let total = if kind == "analyze" {
            plan.vouchers
                .iter()
                .flat_map(|v| &v.files)
                .filter(|f| pdf::needs_analysis(f))
                .count() as u32
        } else {
            plan.vouchers
                .iter()
                .filter(|v| {
                    request
                        .voucher_ids
                        .as_ref()
                        .map(|ids| ids.contains(&v.id))
                        .unwrap_or(true)
                })
                .flat_map(|v| &v.files)
                .map(|f| f.pages)
                .sum()
        };
        let j = Job {
            id: uuid::Uuid::new_v4().to_string(),
            plan_id: plan_id.into(),
            kind: kind.into(),
            status: "queued".into(),
            created_at: now(),
            updated_at: now(),
            total,
            done: 0,
            current: "准备中".into(),
            error: None,
            results: vec![],
            request,
            threads,
        };
        if kind == "analyze" {
            let mut updated = plan.clone();
            updated.orientation_workflow.analysis_completed = false;
            self.save_plan(&updated)?;
            data.plans.insert(plan_id.into(), updated);
        }
        self.save_job(&j)?;
        let cancel = Arc::new(AtomicBool::new(false));
        data.cancels.insert(j.id.clone(), cancel.clone());
        data.jobs.insert(j.id.clone(), j.clone());
        drop(data);
        let state = self.clone();
        let task = j.clone();
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                state.execute(&task, &cancel)
            }));
            let error = match outcome {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(format!("{e:#}")),
                Err(_) => Some("任务内部异常；源文件未改动，可检查后重试".into()),
            };
            let _ = state.update_job(&task.id, |j| {
                if cancel.load(Ordering::Relaxed) {
                    j.status = "cancelled".into();
                    j.current = "已取消，完成的凭证保留，可重试继续".into();
                } else if let Some(err) = error {
                    j.status = "failed".into();
                    j.error = Some(err);
                    j.current = "任务失败".into();
                } else if j.results.iter().any(|r| r.status == "failed") {
                    j.status = "completed_with_errors".into();
                    j.current = "处理结束，部分凭证需要处理后重试".into();
                } else {
                    j.status = "completed".into();
                    j.current = "处理完成".into();
                }
            });
            state.data.lock().unwrap().cancels.remove(&task.id);
        });
        Ok(j)
    }
    pub fn cancel(&self, id: &str) -> Result<Job> {
        let mut data = self.data.lock().unwrap();
        let j = data.jobs.get(id).context("任务不存在")?.clone();
        if j.active() {
            if j.kind == "analyze" {
                let mut plan = data.plans.get(&j.plan_id).cloned().context("计划不存在")?;
                plan.orientation_workflow.analysis_completed = false;
                self.save_plan(&plan)?;
                data.plans.insert(j.plan_id.clone(), plan);
            }
            if let Some(c) = data.cancels.get(id) {
                c.store(true, Ordering::Relaxed);
            }
        }
        Ok(j)
    }
    pub fn retry(self: &Arc<Self>, id: &str) -> Result<Job> {
        let old = self.job(id)?;
        if old.active() {
            bail!("任务仍在运行");
        }
        self.start_with_threads(&old.plan_id, &old.kind, old.request, Some(old.threads))
    }
    fn execute(&self, job: &Job, cancel: &AtomicBool) -> Result<()> {
        self.update_job(&job.id, |j| j.status = "running".into())?;
        let plan = self.plan(&job.plan_id)?;
        if job.kind == "analyze" {
            self.execute_analysis(job, &plan, cancel)
        } else {
            self.execute_generation(job, &plan, cancel)
        }
    }

    fn execute_analysis(&self, job: &Job, plan: &Plan, cancel: &AtomicBool) -> Result<()> {
        let work: Vec<_> = plan
            .vouchers
            .iter()
            .flat_map(|v| &v.files)
            .filter(|f| pdf::needs_analysis(f))
            .collect();
        let mut engines = Vec::with_capacity(job.threads.min(work.len()));
        let outcome = (|| -> Result<usize> {
            if work.is_empty() || cancel.load(Ordering::Acquire) {
                return Ok(0);
            }
            self.update_job(&job.id, |j| j.current = "正在准备识别工作线程".into())?;
            let warm_engine = self.engine.lock().unwrap().take();
            engines.push(match warm_engine {
                Some(engine) => engine,
                None => Engine::new(&self.model_path, self.runtime_path.as_deref())?,
            });
            for _ in 1..job.threads.min(work.len()) {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                engines.push(
                    Engine::new(&self.model_path, self.runtime_path.as_deref())
                        .context("无法准备并行识别模型，可降低线程数后新建任务")?,
                );
            }
            self.analyze_work(job, plan, &work, &mut engines, cancel)
        })();
        // Keep one warm engine, releasing all additional sessions after the job.
        // After a worker panic or scheduler failure the next job rebuilds it.
        if outcome.is_ok() {
            if let Some(engine) = engines.into_iter().next() {
                *self.engine.lock().unwrap() = Some(engine);
            }
        }
        {
            let mut data = self.data.lock().unwrap();
            let mut current = data.plans.get(&plan.id).cloned().context("计划不存在")?;
            current.orientation_workflow.analysis_completed =
                outcome.is_ok() && !cancel.load(Ordering::Acquire);
            self.save_plan(&current)?;
            data.plans.insert(plan.id.clone(), current);
        }
        let failures = outcome?;
        if failures > 0 {
            bail!("{} 个文件方向识别失败，详情见页面列表；可重试", failures);
        }
        Ok(())
    }

    fn analyze_work(
        &self,
        job: &Job,
        plan: &Plan,
        work: &[&SourceFile],
        engines: &mut [Engine],
        cancel: &AtomicBool,
    ) -> Result<usize> {
        let mut failures = 0;
        let mut completed = 0;
        workers::run(
            work,
            engines,
            cancel,
            |engine, file, _| {
                // Fingerprint failures are source errors, not optional OCR errors.
                pdf::check_source(Path::new(&plan.root), file)
                    .map_err(|error| (false, error))
                    .and_then(|path| {
                        let result = engine.predict(&path).map_err(|error| (true, error))?;
                        pdf::check_source(Path::new(&plan.root), file)
                            .map_err(|error| (false, error))?;
                        Ok(result)
                    })
            },
            |event| {
                match event {
                    workers::Event::Started { index } => {
                        self.update_job(&job.id, |j| {
                            j.current = format!("最近开始：{}", work[index].relative_path)
                        })?;
                    }
                    workers::Event::Finished { index, result } => {
                        let mut data = self.data.lock().unwrap();
                        let current = data.plans.get_mut(&plan.id).context("计划不存在")?;
                        let file = current
                            .vouchers
                            .iter_mut()
                            .flat_map(|v| &mut v.files)
                            .find(|f| f.id == work[index].id)
                            .context("图片不属于计划")?;
                        match result {
                            Ok(orientation) => {
                                if !file.reviewed {
                                    file.rotation_cw = orientation.rotation_cw;
                                }
                                file.orientation = Some(orientation);
                                if file.error.as_deref().is_some_and(pdf::is_orientation_error) {
                                    file.error = None;
                                }
                            }
                            Err((orientation_error, error)) => {
                                // A model error cannot erase a real source error.
                                if !orientation_error
                                    || file.error.as_deref().is_none_or(pdf::is_orientation_error)
                                {
                                    file.error = Some(format!(
                                        "{}：{error:#}",
                                        if orientation_error {
                                            "方向识别失败"
                                        } else {
                                            "源文件校验失败"
                                        }
                                    ));
                                }
                                failures += 1;
                            }
                        }
                        completed += 1;
                        if completed % 20 == 0 {
                            self.save_plan(current)?;
                        }
                        drop(data);
                        self.update_job(&job.id, |j| j.done += 1)?;
                    }
                    workers::Event::Panicked => bail!("识别工作线程异常，可重试继续"),
                    workers::Event::Progress { .. } => {}
                }
                Ok(())
            },
        )?;
        Ok(failures)
    }

    fn execute_generation(&self, job: &Job, plan: &Plan, cancel: &AtomicBool) -> Result<()> {
        let work: Vec<_> = plan
            .vouchers
            .iter()
            .filter(|voucher| {
                job.request
                    .voucher_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&voucher.id))
            })
            .collect();
        let report_dir = self.dir.join("reports").join(&job.id);
        fs::create_dir_all(&report_dir)?;
        let positions: HashMap<_, _> = work
            .iter()
            .enumerate()
            .map(|(index, voucher)| (voucher.id.clone(), index))
            .collect();
        let mut accounted = vec![0u32; work.len()];
        let mut contexts = vec![(); job.threads.min(work.len())];
        workers::run(
            &work,
            &mut contexts,
            cancel,
            |_, voucher, progress| {
                pdf::build_voucher(
                    &self.dir,
                    plan,
                    voucher,
                    job.request.allow_unreviewed,
                    cancel,
                    progress,
                )
            },
            |event| {
                match event {
                    workers::Event::Started { index } => {
                        self.update_job(&job.id, |j| {
                            j.current = format!("最近开始：{}", work[index].folder)
                        })?;
                    }
                    workers::Event::Progress { index, pages } => {
                        accounted[index] += pages;
                        self.update_job(&job.id, |j| j.done += pages)?;
                    }
                    workers::Event::Finished { index, result } => {
                        let voucher = work[index];
                        if cancel.load(Ordering::Acquire) && result.is_err() {
                            return Ok(());
                        }
                        let result = match result {
                            Ok(output) => {
                                pdf::atomic_json(
                                    &report_dir.join(format!("{}.json", voucher.id)),
                                    &output.manifest,
                                )?;
                                JobResult {
                                    voucher_id: voucher.id.clone(),
                                    folder: voucher.folder.clone(),
                                    status: if output.skipped {
                                        "skipped"
                                    } else {
                                        "completed"
                                    }
                                    .into(),
                                    output_path: Some(output.path.to_string_lossy().into()),
                                    pages: output.manifest.pages,
                                    error: None,
                                    needs_review: output.manifest.needs_review,
                                }
                            }
                            Err(error) => JobResult {
                                voucher_id: voucher.id.clone(),
                                folder: voucher.folder.clone(),
                                status: "failed".into(),
                                output_path: None,
                                pages: 0,
                                error: Some(format!("{error:#}")),
                                needs_review: voucher
                                    .files
                                    .iter()
                                    .any(|f| pdf::requires_manual_review(plan, f)),
                            },
                        };
                        let pages = voucher.files.iter().map(|f| f.pages).sum::<u32>();
                        self.update_job(&job.id, |j| {
                            // Other vouchers may have progressed since this worker started.
                            j.done += pages.saturating_sub(accounted[index]);
                            j.results.push(result);
                            j.results
                                .sort_by_key(|result| positions[&result.voucher_id]);
                        })?;
                    }
                    workers::Event::Panicked => {
                        bail!("PDF工作线程异常，已提交的成品保留，可重试继续")
                    }
                }
                Ok(())
            },
        )
    }
}
