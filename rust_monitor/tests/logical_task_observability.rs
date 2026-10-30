// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use fod_rust_monitor::{
    LogicalTaskClass, LogicalTaskLane, LogicalTaskObservabilitySampler, LogicalTaskOperation,
    LogicalTaskQueueObservability,
};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

fn tracker() -> Arc<LogicalTaskQueueObservability> {
    Arc::new(LogicalTaskQueueObservability::new(
        LogicalTaskClass::new(LogicalTaskLane::Write, LogicalTaskOperation::FileWrite),
        0,
        0,
        4096,
    ))
}

fn tracker_for(
    lane: LogicalTaskLane,
    operation: LogicalTaskOperation,
) -> Arc<LogicalTaskQueueObservability> {
    Arc::new(LogicalTaskQueueObservability::new(
        LogicalTaskClass::new(lane, operation),
        0,
        0,
        4096,
    ))
}

#[test]
fn completed_observation_balances_active_payload_and_throughput() {
    let tracker = tracker();
    let observation = tracker.observe_task(4096, true);

    let active = tracker.queue_snapshot().unwrap();
    assert_eq!(active.admitted_tasks, 1);
    assert_eq!(active.queued_tasks, 0);
    assert_eq!(active.peak_queued_tasks, 1);
    assert_eq!(active.active_tasks, 1);
    assert_eq!(active.peak_active_tasks, 1);
    assert_eq!(active.active_transactions, 1);
    assert_eq!(active.payload_in_flight_bytes, 4096);

    observation.complete(1, 4096);

    let finished = tracker.queue_snapshot().unwrap();
    assert_eq!(finished.completed_tasks, 1);
    assert_eq!(finished.failed_tasks, 0);
    assert_eq!(finished.active_tasks, 0);
    assert_eq!(finished.active_transactions, 0);
    assert_eq!(finished.payload_in_flight_bytes, 0);
    assert_eq!(finished.accounting_errors, 0);

    let throughput = tracker.throughput_snapshot().unwrap();
    assert_eq!(throughput.completed_files, 1);
    assert_eq!(throughput.completed_bytes, 4096);
}

#[test]
fn concurrent_tasks_keep_queue_and_throughput_accounting_balanced() {
    const WORKERS: usize = 8;
    const PAYLOAD_BYTES: u64 = 1024;

    let tracker = tracker();
    let all_started = Arc::new(Barrier::new(WORKERS + 1));
    let release_tasks = Arc::new(Barrier::new(WORKERS + 1));
    let workers = (0..WORKERS)
        .map(|_| {
            let tracker = Arc::clone(&tracker);
            let all_started = Arc::clone(&all_started);
            let release_tasks = Arc::clone(&release_tasks);
            thread::spawn(move || {
                let observation = tracker.observe_task(PAYLOAD_BYTES, true);
                all_started.wait();
                release_tasks.wait();
                observation.complete(1, PAYLOAD_BYTES);
            })
        })
        .collect::<Vec<_>>();

    all_started.wait();
    let active = tracker.queue_snapshot().unwrap();
    assert_eq!(active.admitted_tasks, WORKERS as u64);
    assert_eq!(active.queued_tasks, 0);
    assert_eq!(active.active_tasks, WORKERS as u64);
    assert_eq!(active.peak_active_tasks, WORKERS as u64);
    assert_eq!(active.active_transactions, WORKERS as u64);
    assert_eq!(
        active.payload_in_flight_bytes,
        WORKERS as u64 * PAYLOAD_BYTES
    );
    assert_eq!(active.completed_tasks, 0);

    release_tasks.wait();
    for worker in workers {
        worker.join().expect("concurrent task worker panicked");
    }

    let finished = tracker.queue_snapshot().unwrap();
    assert_eq!(finished.admitted_tasks, WORKERS as u64);
    assert_eq!(finished.completed_tasks, WORKERS as u64);
    assert_eq!(finished.failed_tasks, 0);
    assert_eq!(finished.active_tasks, 0);
    assert_eq!(finished.active_transactions, 0);
    assert_eq!(finished.payload_in_flight_bytes, 0);
    assert_eq!(finished.accounting_errors, 0);

    let throughput = tracker.throughput_snapshot().unwrap();
    assert_eq!(throughput.completed_files, WORKERS as u64);
    assert_eq!(throughput.completed_bytes, WORKERS as u64 * PAYLOAD_BYTES);
}

#[test]
fn concurrent_read_write_and_copy_tasks_keep_class_accounting_isolated() {
    const WORKERS_PER_CLASS: usize = 4;
    const PAYLOAD_BYTES: u64 = 512;

    let classes = [
        (
            tracker_for(LogicalTaskLane::Read, LogicalTaskOperation::FileRead),
            false,
        ),
        (
            tracker_for(LogicalTaskLane::Write, LogicalTaskOperation::FileWrite),
            false,
        ),
        (
            tracker_for(LogicalTaskLane::Write, LogicalTaskOperation::FileCopy),
            true,
        ),
    ];
    let worker_count = classes.len() * WORKERS_PER_CLASS;
    let all_active = Arc::new(Barrier::new(worker_count + 1));
    let release_tasks = Arc::new(Barrier::new(worker_count + 1));
    let workers = classes
        .iter()
        .flat_map(|(tracker, fail_first)| {
            (0..WORKERS_PER_CLASS).map({
                let tracker = Arc::clone(tracker);
                let all_active = Arc::clone(&all_active);
                let release_tasks = Arc::clone(&release_tasks);
                let fail_first = *fail_first;
                move |worker_id| {
                    let tracker = Arc::clone(&tracker);
                    let all_active = Arc::clone(&all_active);
                    let release_tasks = Arc::clone(&release_tasks);
                    thread::spawn(move || {
                        let observation = tracker.observe_task(PAYLOAD_BYTES, true);
                        all_active.wait();
                        release_tasks.wait();
                        if fail_first && worker_id == 0 {
                            observation.fail();
                        } else {
                            observation.complete(1, PAYLOAD_BYTES);
                        }
                    })
                }
            })
        })
        .collect::<Vec<_>>();

    all_active.wait();
    for (tracker, _) in &classes {
        let active = tracker.queue_snapshot().unwrap();
        assert_eq!(active.admitted_tasks, WORKERS_PER_CLASS as u64);
        assert_eq!(active.queued_tasks, 0);
        assert_eq!(active.active_tasks, WORKERS_PER_CLASS as u64);
        assert_eq!(active.peak_active_tasks, WORKERS_PER_CLASS as u64);
        assert_eq!(active.active_transactions, WORKERS_PER_CLASS as u64);
        assert_eq!(
            active.payload_in_flight_bytes,
            WORKERS_PER_CLASS as u64 * PAYLOAD_BYTES
        );
    }

    release_tasks.wait();
    for worker in workers {
        worker.join().expect("mixed task worker panicked");
    }

    for (tracker, fail_first) in &classes {
        let finished = tracker.queue_snapshot().unwrap();
        let expected_failures = if *fail_first { 1 } else { 0 };
        assert_eq!(finished.admitted_tasks, WORKERS_PER_CLASS as u64);
        assert_eq!(finished.completed_tasks, WORKERS_PER_CLASS as u64);
        assert_eq!(finished.failed_tasks, expected_failures);
        assert_eq!(finished.queued_tasks, 0);
        assert_eq!(finished.active_tasks, 0);
        assert_eq!(finished.active_transactions, 0);
        assert_eq!(finished.payload_in_flight_bytes, 0);
        assert_eq!(finished.accounting_errors, 0);

        let throughput = tracker.throughput_snapshot().unwrap();
        assert_eq!(
            throughput.completed_files,
            WORKERS_PER_CLASS as u64 - expected_failures
        );
        assert_eq!(
            throughput.completed_bytes,
            (WORKERS_PER_CLASS as u64 - expected_failures) * PAYLOAD_BYTES
        );
    }

    assert_eq!(
        classes[0].0.queue_snapshot().unwrap().class,
        LogicalTaskClass::new(LogicalTaskLane::Read, LogicalTaskOperation::FileRead)
    );
    assert_eq!(
        classes[1].0.queue_snapshot().unwrap().class,
        LogicalTaskClass::new(LogicalTaskLane::Write, LogicalTaskOperation::FileWrite)
    );
    assert_eq!(
        classes[2].0.queue_snapshot().unwrap().class,
        LogicalTaskClass::new(LogicalTaskLane::Write, LogicalTaskOperation::FileCopy)
    );
}

#[test]
fn dropped_observation_is_recorded_as_failure_and_releases_counters() {
    let tracker = tracker();

    {
        let _observation = tracker.observe_task(2048, false);
        let active = tracker.queue_snapshot().unwrap();
        assert_eq!(active.active_tasks, 1);
        assert_eq!(active.payload_in_flight_bytes, 2048);
    }

    let finished = tracker.queue_snapshot().unwrap();
    assert_eq!(finished.completed_tasks, 1);
    assert_eq!(finished.failed_tasks, 1);
    assert_eq!(finished.active_tasks, 0);
    assert_eq!(finished.payload_in_flight_bytes, 0);
    assert_eq!(finished.accounting_errors, 0);
}

#[test]
fn explicit_failure_is_counted_once() {
    let tracker = tracker();

    tracker.observe_task(512, false).fail();

    let finished = tracker.queue_snapshot().unwrap();
    assert_eq!(finished.completed_tasks, 1);
    assert_eq!(finished.failed_tasks, 1);
    assert_eq!(finished.active_tasks, 0);
    assert_eq!(finished.payload_in_flight_bytes, 0);
}

#[test]
fn combined_snapshot_contains_queue_and_throughput_from_one_state() {
    let tracker = tracker();
    tracker.observe_task(1024, false).complete(1, 1024);

    let snapshot = tracker.snapshot().unwrap();
    assert_eq!(snapshot.queue.admitted_tasks, 1);
    assert_eq!(snapshot.queue.completed_tasks, 1);
    assert_eq!(snapshot.queue.active_tasks, 0);
    assert_eq!(snapshot.throughput.completed_files, 1);
    assert_eq!(snapshot.throughput.completed_bytes, 1024);
    assert_eq!(snapshot.queue.class, snapshot.throughput.class);
}

#[test]
fn logical_task_sampler_samples_now_and_stops_without_sleep() {
    let tracker = tracker();
    tracker.observe_task(256, false).complete(0, 256);

    let mut sampler =
        LogicalTaskObservabilitySampler::spawn(vec![tracker], Duration::from_secs(60)).unwrap();
    sampler.sample_now();
    sampler.stop();
}
