//! Bounded native workers with one coordinator for progress and durable state.
use anyhow::{bail, Result};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::sync_channel,
};

pub(crate) enum Event<R> {
    Started { index: usize },
    Progress { index: usize, pages: u32 },
    Finished { index: usize, result: R },
    Panicked,
}

/// Each worker owns a separate mutable context (an OCR engine, or () for PDF).
/// Results are streamed to the caller, which alone updates plans/job records.
/// All running workers are joined and their results drained even after cancel,
/// a worker panic, or a persistence error, so committed PDFs are not discarded.
pub(crate) fn run<T: Sync, W: Send, R: Send>(
    items: &[T],
    contexts: &mut [W],
    cancel: &AtomicBool,
    execute: impl Fn(&mut W, &T, &mut dyn FnMut(u32)) -> R + Sync,
    mut on_event: impl FnMut(Event<R>) -> Result<()>,
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    if contexts.is_empty() {
        bail!("没有可用工作线程");
    }
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (sender, receiver) = sync_channel(contexts.len() * 2);
    std::thread::scope(|scope| {
        let mut error = None;
        for (worker_id, context) in contexts.iter_mut().enumerate() {
            let sender = sender.clone();
            let (next, stop, execute) = (&next, &stop, &execute);
            let spawned = std::thread::Builder::new()
                .name(format!("voucher-worker-{}", worker_id + 1))
                .spawn_scoped(scope, move || {
                    while !cancel.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index >= items.len()
                            || cancel.load(Ordering::Acquire)
                            || stop.load(Ordering::Acquire)
                        {
                            break;
                        }
                        if sender.send(Event::Started { index }).is_err() {
                            break;
                        }
                        let mut progress = |pages| {
                            let _ = sender.send(Event::Progress { index, pages });
                        };
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                execute(context, &items[index], &mut progress)
                            }));
                        match outcome {
                            Ok(result) => {
                                if sender.send(Event::Finished { index, result }).is_err() {
                                    break;
                                }
                            }
                            Err(_) => {
                                stop.store(true, Ordering::Release);
                                let _ = sender.send(Event::Panicked);
                                break;
                            }
                        }
                    }
                });
            if let Err(failure) = spawned {
                stop.store(true, Ordering::Release);
                error = Some(anyhow::Error::from(failure).context("无法创建工作线程"));
                break;
            }
        }
        drop(sender);
        for event in receiver {
            if let Err(failure) = on_event(event) {
                stop.store(true, Ordering::Release);
                if error.is_none() {
                    error = Some(failure);
                }
            }
        }
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};

    #[test]
    fn one_to_ten_workers_overlap_without_exceeding_the_limit_or_repeating_items() {
        for threads in 1..=10 {
            let work: Vec<_> = (0..threads * 3).collect();
            let barrier = Barrier::new(threads);
            let active = AtomicUsize::new(0);
            let peak = AtomicUsize::new(0);
            let mut contexts = vec![(); threads];
            let mut completed = vec![];
            run(
                &work,
                &mut contexts,
                &AtomicBool::new(false),
                |_, index, progress| {
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(count, Ordering::SeqCst);
                    // The first wave cannot finish until every worker is live.
                    if *index < threads {
                        barrier.wait();
                    }
                    progress(1);
                    active.fetch_sub(1, Ordering::SeqCst);
                    *index
                },
                |event| {
                    if let Event::Finished { index, result } = event {
                        assert_eq!(index, result);
                        completed.push(index);
                    }
                    Ok(())
                },
            )
            .unwrap();
            completed.sort_unstable();
            assert_eq!(completed, work);
            assert_eq!(peak.load(Ordering::SeqCst), threads);
        }
    }

    #[test]
    fn cancellation_stops_dispatch_but_drains_the_running_results() {
        let cancel = AtomicBool::new(false);
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let mut contexts = vec![(); 4];
        let mut started = 0;
        let mut finished = 0;
        run(
            &(0..100).collect::<Vec<_>>(),
            &mut contexts,
            &cancel,
            |_, item, _| {
                let (open, ready) = &*gate;
                drop(
                    ready
                        .wait_while(open.lock().unwrap(), |open| !*open)
                        .unwrap(),
                );
                *item
            },
            |event| {
                match event {
                    Event::Started { .. } => {
                        started += 1;
                        if started == 4 {
                            cancel.store(true, Ordering::Release);
                            *gate.0.lock().unwrap() = true;
                            gate.1.notify_all();
                        }
                    }
                    Event::Finished { .. } => finished += 1,
                    _ => {}
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!((started, finished), (4, 4));
    }

    #[test]
    fn a_worker_panic_or_coordinator_failure_does_not_lose_other_finished_results() {
        for panic_worker in [false, true] {
            let barrier = Barrier::new(3);
            let mut contexts = vec![(); 3];
            let mut finished = vec![];
            let result = run(
                &[0, 1, 2],
                &mut contexts,
                &AtomicBool::new(false),
                |_, item, _| {
                    barrier.wait();
                    assert!(!(panic_worker && *item == 0), "synthetic worker panic");
                    *item
                },
                |event| {
                    match event {
                        Event::Finished { result, .. } => {
                            finished.push(result);
                            if !panic_worker && result == 0 {
                                bail!("synthetic persistence failure");
                            }
                        }
                        Event::Panicked => bail!("worker failed"),
                        _ => {}
                    }
                    Ok(())
                },
            );
            assert!(result.is_err());
            finished.sort_unstable();
            assert_eq!(
                finished,
                if panic_worker {
                    vec![1, 2]
                } else {
                    vec![0, 1, 2]
                }
            );
        }
    }
}
