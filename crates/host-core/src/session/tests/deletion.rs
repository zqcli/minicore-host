use super::*;

struct DeleteGateJournal {
    inner: Arc<JournalHistory>,
    persisted: Arc<Semaphore>,
    acknowledgement: Arc<Semaphore>,
    release_entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    first: AtomicBool,
}
impl HistoryBackend for DeleteGateJournal {
    fn mode(&self) -> StorageMode {
        StorageMode::Persistent
    }
    fn load(&self) -> LoadFuture<'_> {
        self.inner.load()
    }
    fn append<'a>(&'a self, entries: Vec<HistoryEntry>) -> SaveFuture<'a> {
        Box::pin(async move {
            let result = self.inner.append(entries).await?;
            if self.first.swap(false, Ordering::SeqCst) {
                self.persisted.add_permits(1);
                self.acknowledgement.acquire().await.unwrap().forget();
            }
            Ok(result)
        })
    }
    fn release(&self) -> Pin<Box<dyn Future<Output = Result<(), Fault>> + Send + '_>> {
        Box::pin(async move {
            self.release_entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            self.inner.release().await
        })
    }
}
#[tokio::test]
async fn task051_confirmed_file_bytes_do_not_bypass_save_callback_or_resource_release() {
    let persisted = Arc::new(Semaphore::new(0));
    let acknowledgement = Arc::new(Semaphore::new(0));
    let release_entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let mut fixture = setup(
        Arc::new(Resources {
            model: model(vec![final_events("gated actual Runtime response")]),
            tools: ToolSet::default(),
        }),
        |inner| {
            Arc::new(DeleteGateJournal {
                inner,
                persisted: persisted.clone(),
                acknowledgement: acknowledgement.clone(),
                release_entered: release_entered.clone(),
                release: release.clone(),
                first: AtomicBool::new(true),
            })
        },
    );
    // The standard history fixture supplies real JournalHistory and files;
    // additionally install its actual persistent object boundary for deletion.
    let record = fixture.store.load().unwrap();
    Arc::get_mut(&mut fixture.session)
        .expect("unpublished fixture owner")
        .provenance = Some(record.clone());
    *fixture.session.store.lock().unwrap() = Some(fixture.store.clone());
    fixture.session.state.lock().unwrap().record_base = Some(record);
    send(&fixture, "accepted input");
    persisted.acquire().await.unwrap().forget();
    assert!(!records(&fixture).is_empty());
    let operation = OperationId::new().unwrap();
    let (accepted, records) = fixture
        .session
        .claim_delete_owned(true, &operation)
        .unwrap();
    diagnostic::emit(records);
    assert_eq!(accepted.lifecycle, SessionCloseLifecycle::Closing);
    fixture.session.begin_delete_close_owned();
    let owner = fixture.session.clone();
    let task = tokio::spawn(async move { owner.delete_data_owned(Some(&operation)).await });
    assert!(fixture.session.state.lock().unwrap().saving);
    assert!(!task.is_finished());
    assert!(fixture.store.canonical_path().join("session.json").exists());
    assert_eq!(release_entered.available_permits(), 0);
    acknowledgement.add_permits(1);
    tokio::time::timeout(std::time::Duration::from_secs(3), release_entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert!(!task.is_finished());
    assert!(
        fixture
            .store
            .canonical_path()
            .join("history.jsonl")
            .exists()
    );
    release.add_permits(1);
    assert_eq!(task.await.unwrap().unwrap(), DataDeletion::Deleted {});
    assert!(!fixture.store.canonical_path().exists());
    assert_eq!(
        fixture
            .session
            .snapshot(&fixture.connection.caller())
            .unwrap_err()
            .kind(),
        Some(FaultKind::AgentSessionNotFound)
    );
}
