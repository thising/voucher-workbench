//! Per-job decisions. No AppState.data lock is held while a worker waits.
use super::*;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputConflict {
    pub id: String,
    pub group_id: String,
    pub group_name: String,
    pub output_path: String,
    pub reason: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Skip,
    Overwrite,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputAuthorization {
    pub job_id: String,
    pub conflict_id: String,
    pub output_path: String,
    pub previous_sha256: String,
    pub approved_at: String,
    pub apply_to_batch: bool,
    pub policy_applied: bool,
}
#[derive(Clone, Debug)]
pub(super) enum Decision {
    Skip,
    Overwrite(OutputAuthorization),
}
#[derive(Clone)]
struct Policy {
    action: Action,
    id: String,
    approved_at: String,
}
struct Pending {
    request: OutputConflict,
    hash: String,
    reply: Option<(Action, bool, String)>,
}
#[derive(Default)]
struct Inner {
    pending: Option<Pending>,
    policy: Option<Policy>,
    closed: bool,
}
#[derive(Default)]
pub(crate) struct ConflictCoordinator {
    inner: Mutex<Inner>,
    wake: Condvar,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveRequest {
    pub conflict_id: String,
    pub action: Action,
    #[serde(default)]
    pub apply_to_batch: bool,
}

pub(super) struct RequestContext<'a> {
    pub state: &'a AppState,
    pub job: &'a Job,
    pub cancel: &'a AtomicBool,
    pub stop: &'a AtomicBool,
}

impl ConflictCoordinator {
    fn stopped(state: &AppState, cancel: &AtomicBool, stop: &AtomicBool) -> bool {
        cancel.load(Ordering::Acquire)
            || stop.load(Ordering::Acquire)
            || state.shutting_down.load(Ordering::Acquire)
    }
    pub(super) fn request(
        &self,
        context: &RequestContext<'_>,
        group: &Group,
        path: &Path,
        hash: &str,
    ) -> Result<Decision> {
        let result = self.request_inner(context, group, path, hash);
        if result.is_err() {
            self.close();
            let _ = context
                .state
                .update_job(&context.job.id, |job| job.output_conflict = None);
        }
        result
    }
    fn request_inner(
        &self,
        context: &RequestContext<'_>,
        group: &Group,
        path: &Path,
        hash: &str,
    ) -> Result<Decision> {
        let (state, job, cancel, stop) = (context.state, context.job, context.cancel, context.stop);
        let mut inner = self.inner.lock().unwrap();
        loop {
            if inner.closed || Self::stopped(state, cancel, stop) {
                bail!("任务已取消或停止，未覆盖已有文件");
            }
            if let Some(policy) = &inner.policy {
                return Ok(match policy.action {
                    Action::Skip => Decision::Skip,
                    Action::Overwrite => Decision::Overwrite(OutputAuthorization {
                        job_id: job.id.clone(),
                        conflict_id: policy.id.clone(),
                        output_path: path_text(path)?,
                        previous_sha256: hash.into(),
                        approved_at: policy.approved_at.clone(),
                        apply_to_batch: true,
                        policy_applied: true,
                    }),
                });
            }
            if inner.pending.is_none() {
                break;
            }
            inner = self
                .wake
                .wait_timeout(inner, Duration::from_millis(100))
                .unwrap()
                .0;
        }
        let conflict = OutputConflict {
            id: uuid::Uuid::new_v4().to_string(),
            group_id: group.id.clone(),
            group_name: group.name.clone(),
            output_path: path_text(path)?,
            reason:
                "此位置已有同名文件。覆盖将直接替换原文件，不保留备份，无法恢复；也可以选择跳过。"
                    .into(),
        };
        inner.pending = Some(Pending {
            request: conflict.clone(),
            hash: hash.into(),
            reply: None,
        });
        state.update_job(&job.id, |job| job.output_conflict = Some(conflict.clone()))?;
        loop {
            if inner.closed || Self::stopped(state, cancel, stop) {
                bail!("任务已取消或停止，未覆盖已有文件");
            }
            if let Some((action, apply, approved_at)) = inner
                .pending
                .as_ref()
                .and_then(|pending| pending.reply.clone())
            {
                let pending = inner.pending.take().context("同名文件决策已过期")?;
                if apply {
                    inner.policy = Some(Policy {
                        action,
                        id: pending.request.id.clone(),
                        approved_at: approved_at.clone(),
                    });
                }
                self.wake.notify_all();
                return Ok(match action {
                    Action::Skip => Decision::Skip,
                    Action::Overwrite => Decision::Overwrite(OutputAuthorization {
                        job_id: job.id.clone(),
                        conflict_id: pending.request.id,
                        output_path: pending.request.output_path,
                        previous_sha256: pending.hash,
                        approved_at,
                        apply_to_batch: apply,
                        policy_applied: false,
                    }),
                });
            }
            inner = self
                .wake
                .wait_timeout(inner, Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }
    pub(super) fn resolve(
        &self,
        state: &AppState,
        id: &str,
        request: ResolveRequest,
    ) -> Result<Job> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closed {
            bail!("同名文件决策已结束或已取消，请刷新任务状态");
        }
        let pending = inner
            .pending
            .as_mut()
            .context("当前没有等待处理的同名文件")?;
        if pending.request.id != request.conflict_id || pending.reply.is_some() {
            bail!("同名文件决策已过期，请刷新当前提示");
        }
        let mut data = state.data.lock().unwrap();
        let cancelled = data
            .cancels
            .get(id)
            .is_some_and(|flag| flag.load(Ordering::Acquire));
        let job = data.jobs.get_mut(id).context("任务不存在")?;
        if job.kind != "batch_generate"
            || !job.active()
            || cancelled
            || state.shutting_down.load(Ordering::Acquire)
        {
            bail!("任务已结束、取消或工作台正在退出");
        }
        let approved_at = now();
        let mut resolved = job.clone();
        resolved.output_conflict = None;
        resolved.updated_at = approved_at.clone();
        state
            .save_job(&resolved)
            .context("无法保存同名文件决策，尚未执行；请检查处理记录目录后重试")?;
        // A failed save leaves both the visible prompt and the reply untouched.
        // Workers cannot consume an action whose API response was a failure.
        *job = resolved.clone();
        pending.reply = Some((request.action, request.apply_to_batch, approved_at));
        let result = resolved;
        drop(data);
        self.wake.notify_all();
        Ok(result)
    }
    pub(super) fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        inner.pending = None;
        self.wake.notify_all();
    }
}
pub(crate) fn close_output_conflicts(state: &AppState, id: &str) {
    let coordinator = state.output_conflicts.lock().unwrap().remove(id);
    if let Some(coordinator) = coordinator {
        coordinator.close();
    }
}
pub(super) async fn api_resolve(
    State(state): State<Arc<AppState>>,
    ApiPath(id): ApiPath<String>,
    Json(request): Json<ResolveRequest>,
) -> ApiResult<Json<Job>> {
    let coordinator = state
        .output_conflicts
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .context("该任务当前没有等待处理的同名文件")?;
    let job = coordinator.resolve(&state, &id, request)?;
    Ok(Json(job))
}
