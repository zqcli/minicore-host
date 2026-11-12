use super::*;
use crate::time::DeadlineSource;

fn deadline_with_retry(
    model_timeout: Duration,
    retry_policy: RetryPolicySnapshot,
) -> ModelDriverConfig {
    ModelDriverConfig {
        model_call_timeout: model_timeout,
        retry_policy,
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn pending_stream_reports_exact_turn_or_port_deadline_source() {
    for (turn_after, model_timeout, expected) in [
        (
            Duration::from_secs(5),
            Duration::from_secs(30),
            DeadlineSource::Turn,
        ),
        (
            Duration::from_secs(30),
            Duration::from_secs(5),
            DeadlineSource::Port,
        ),
    ] {
        let probe = StreamProbe::shared();
        let model = ScriptModel::new(
            descriptor(),
            vec![Behavior::ProbedStream {
                events: Vec::new(),
                terminal: StreamTerminal::Pending,
                probe: Arc::clone(&probe),
            }],
        );
        let driver = model.driver(&deadline_with_retry(
            model_timeout,
            RetryPolicySnapshot::new(1, Duration::ZERO),
        ));
        let (progress, _receiver) = progress_channel();
        let run = tokio::spawn(async move {
            driver
                .run_detailed(
                    request(),
                    context(CancellationToken::new(), turn_after),
                    &progress,
                )
                .await
        });
        probe.wait_for_terminal().await;
        tokio::time::advance(Duration::from_secs(6)).await;
        let failure = run.await.unwrap().unwrap_err();
        assert_eq!(failure.error().kind(), ModelErrorKind::Timeout);
        assert_eq!(failure.deadline_source(), Some(expected));
        assert!(probe.dropped());
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn absent_outer_deadline_uses_the_configured_maximum_model_budget_and_port_source() {
    let probe = StreamProbe::shared();
    let model = ScriptModel::new(
        descriptor(),
        vec![Behavior::ProbedStream {
            events: Vec::new(),
            terminal: StreamTerminal::Pending,
            probe: Arc::clone(&probe),
        }],
    );
    let started = TokioInstant::now();
    let call_model = Arc::clone(&model);
    let run = tokio::spawn(async move {
        let driver = ModelDriver::new(
            call_model.as_ref(),
            ModelDriverConfig::new(MAX_MODEL_CALL_TIMEOUT, 1, Duration::from_millis(100)),
        )
        .unwrap();
        driver
            .call(
                request(),
                ModelCallParams {
                    loop_id: loop_id(),
                    request_index: 0,
                    cancellation: CancellationToken::new(),
                    deadline: None,
                },
                &mut |_| {},
            )
            .await
    });
    probe.wait_for_terminal().await;
    assert_eq!(
        *lock(&model.deadlines),
        [(started + MAX_MODEL_CALL_TIMEOUT).into_std()]
    );
    tokio::time::advance(MAX_MODEL_CALL_TIMEOUT - Duration::from_secs(1)).await;
    assert!(!run.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    let report = run.await.unwrap();
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::Timeout);
    assert_eq!(failure.deadline_source(), Some(DeadlineSource::Port));
    assert_eq!(report.attempts, 1);
    assert!(probe.dropped());
    assert!(lock(&model.tokens)[0].is_cancelled());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn retries_without_an_outer_deadline_share_the_original_model_deadline() {
    let probe = StreamProbe::shared();
    let model = ScriptModel::new(
        descriptor(),
        vec![
            Behavior::StartError(retryable_error(None)),
            Behavior::ProbedStream {
                events: Vec::new(),
                terminal: StreamTerminal::Pending,
                probe: Arc::clone(&probe),
            },
        ],
    );
    let started = TokioInstant::now();
    let budget = Duration::from_secs(10);
    let delay = Duration::from_secs(2);
    let call_model = Arc::clone(&model);
    let run = tokio::spawn(async move {
        let driver = ModelDriver::new(
            call_model.as_ref(),
            ModelDriverConfig::new(budget, 2, delay),
        )
        .unwrap();
        driver
            .call(
                request(),
                ModelCallParams {
                    loop_id: loop_id(),
                    request_index: 0,
                    cancellation: CancellationToken::new(),
                    deadline: None,
                },
                &mut |_| {},
            )
            .await
    });
    model.wait_for_completions(1).await;
    tokio::time::advance(delay).await;
    probe.wait_for_terminal().await;
    assert_eq!(*lock(&model.deadlines), [(started + budget).into_std(); 2]);
    tokio::time::advance(budget - delay - Duration::from_secs(1)).await;
    assert!(!run.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    let report = run.await.unwrap();
    let failure = report.outcome.unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::Timeout);
    assert_eq!(failure.deadline_source(), Some(DeadlineSource::Port));
    assert_eq!(report.attempts, 2);
    assert_eq!(report.retry_wait_elapsed, delay);
    assert!(probe.dropped());
    assert!(
        lock(&model.tokens)
            .iter()
            .all(CancellationToken::is_cancelled)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn adapter_timeout_has_no_core_deadline_provenance() {
    let model = ScriptModel::new(
        descriptor(),
        vec![Behavior::StartError(test_error(ModelErrorKind::Timeout))],
    );
    let driver = model.driver(&deadline_with_retry(
        Duration::from_secs(5),
        RetryPolicySnapshot::new(1, Duration::ZERO),
    ));
    let (progress, _receiver) = progress_channel();
    let failure = driver
        .run_detailed(
            request(),
            context(CancellationToken::new(), Duration::from_secs(30)),
            &progress,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::Timeout);
    assert_eq!(failure.deadline_source(), None);
}

#[tokio::test(flavor = "current_thread")]
async fn expired_turn_deadline_is_reported_before_model_invocation() {
    let model = ScriptModel::new(descriptor(), Vec::new());
    let driver = model.driver(&deadline_with_retry(
        Duration::from_secs(30),
        RetryPolicySnapshot::new(1, Duration::ZERO),
    ));
    let mut call_context = context(CancellationToken::new(), Duration::from_secs(30));
    call_context.deadline = Instant::now().checked_sub(Duration::from_secs(1));
    let (progress, _receiver) = progress_channel();
    let failure = driver
        .run_detailed(request(), call_context, &progress)
        .await
        .unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::Timeout);
    assert_eq!(failure.deadline_source(), Some(DeadlineSource::Turn));
    assert_eq!(model.starts(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn cancellation_and_panic_have_no_deadline_provenance() {
    let model = ScriptModel::new(descriptor(), Vec::new());
    let driver = model.driver(&deadline_with_retry(
        Duration::from_secs(5),
        RetryPolicySnapshot::new(1, Duration::ZERO),
    ));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let (progress, _receiver) = progress_channel();
    let failure = driver
        .run_detailed(
            request(),
            context(cancellation, Duration::from_secs(30)),
            &progress,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::Cancelled);
    assert_eq!(failure.deadline_source(), None);

    let model = ScriptModel::new(descriptor(), vec![Behavior::StartPanic]);
    let driver = model.driver(&deadline_with_retry(
        Duration::from_secs(5),
        RetryPolicySnapshot::new(1, Duration::ZERO),
    ));
    let failure = driver
        .run_detailed(
            request(),
            context(CancellationToken::new(), Duration::from_secs(30)),
            &progress,
        )
        .await
        .unwrap_err();
    assert_eq!(failure.error().kind(), ModelErrorKind::Panicked);
    assert_eq!(failure.deadline_source(), None);
}

async fn retry_timeout_source(turn_after: Duration, model_timeout: Duration) -> ModelDriverFailure {
    let model = ScriptModel::new(
        descriptor(),
        vec![
            Behavior::StartError(retryable_error(None)),
            Behavior::Events(text_success("unexpected")),
        ],
    );
    let driver = model.driver(&deadline_with_retry(
        model_timeout,
        RetryPolicySnapshot::new(2, Duration::from_secs(5)),
    ));
    let (progress, _receiver) = progress_channel();
    let run = tokio::spawn(async move {
        driver
            .run_detailed(
                request(),
                context(CancellationToken::new(), turn_after),
                &progress,
            )
            .await
    });
    model.wait_for_completions(1).await;
    tokio::time::advance(Duration::from_secs(10)).await;
    let failure = run.await.unwrap().unwrap_err();
    assert_eq!(model.starts(), 1);
    failure
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn retry_sleep_timeout_reports_turn_source() {
    let failure = retry_timeout_source(Duration::from_secs(9), Duration::from_secs(30)).await;
    assert_eq!(failure.error(), &retryable_error(None));
    assert_eq!(failure.deadline_source(), Some(DeadlineSource::Turn));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn retry_sleep_timeout_reports_port_source() {
    let failure = retry_timeout_source(Duration::from_secs(30), Duration::from_secs(9)).await;
    assert_eq!(failure.error(), &retryable_error(None));
    assert_eq!(failure.deadline_source(), Some(DeadlineSource::Port));
}
