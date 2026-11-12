use super::*;
use minicore_host_runtime::{
    Text, ToolCallId,
    history::SummaryHistory,
    model::{ModelPartial, ToolCall},
};

fn projection(position: u64, covered: u64, text: &str) -> CompactionProjection {
    CompactionProjection::new(
        position,
        covered,
        SummaryHistory {
            content: Text::new(text),
        },
    )
}
fn raw(entries: &[HistoryEntry]) -> Vec<HistoryItem> {
    entries
        .iter()
        .filter_map(HistoryEntry::runtime_item)
        .collect()
}
fn complete_turn() -> Vec<HistoryEntry> {
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    vec![
        input(turn.clone(), loop_id),
        response(turn.clone(), loop_id),
        HistoryEntry::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Completed,
            reason: None,
        },
    ]
}
fn partial_turn() -> Vec<HistoryEntry> {
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    vec![
        input(turn.clone(), loop_id),
        HistoryEntry::Partial {
            turn_id: turn.clone(),
            loop_id,
            response_index: 0,
            partial: ModelPartial {
                request_index: 0,
                content: vec![AssistantPart::Text("actual unfinished output".into())],
                incomplete_tool_calls: vec![],
                usage: None,
                provider_replay: None,
            },
        },
        HistoryEntry::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Cancelled,
            reason: None,
        },
    ]
}
fn tool_exchange() -> Vec<HistoryEntry> {
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    vec![
        input(turn.clone(), loop_id),
        tool_response(turn.clone(), loop_id),
        tool_result(turn, loop_id),
    ]
}
fn append(journal: &mut SessionJournal, entries: Vec<HistoryEntry>) -> SaveRange {
    let range = journal.accept(entries).unwrap();
    assert_eq!(journal.persist_pending().unwrap().range, range);
    range
}
fn assert_invalid_file(f: &Fixture, bytes: &[u8]) {
    fs::write(f.history_path(), bytes).unwrap();
    assert!(f.session.read_history().is_err());
    assert!(f.session.open_journal().is_err());
    assert_eq!(read_history_bytes(f.history_path()).unwrap(), bytes);
}

#[test]
fn repeated_compaction_reload_keeps_raw_bytes_and_projects_only_latest_summary() {
    let f = Fixture::new();
    let mut original = tool_exchange();
    original.extend(complete_turn());
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, original.clone());
    let first_prefix = read_history_bytes(f.history_path()).unwrap();
    let first = projection(6, 3, "first actual summary fixture");
    append(&mut journal, vec![first.clone().into_entry()]);
    let first_compacted = read_history_bytes(f.history_path()).unwrap();
    assert!(first_compacted.starts_with(&first_prefix));
    drop(journal);

    let first_loaded = f.session.read_history().unwrap();
    assert_eq!(first_loaded.runtime_history(), raw(&original));
    assert_eq!(first_loaded.compaction_projection(), Some(first));
    assert_eq!(first_loaded.model_history().unwrap().len(), 3);
    let mut journal = f.session.open_journal().unwrap();
    let next = complete_turn();
    original.extend(next.clone());
    append(&mut journal, next);
    let second_prefix = read_history_bytes(f.history_path()).unwrap();
    let second = projection(10, 5, "second summary includes the earlier summary");
    let range = append(&mut journal, vec![second.clone().into_entry()]);
    assert_eq!(range, SaveRange { start: 10, end: 11 });
    let second_compacted = read_history_bytes(f.history_path()).unwrap();
    assert!(second_compacted.starts_with(&second_prefix));
    assert!(second_prefix.starts_with(&first_compacted));
    // A successful save has no marker to replay or append on a later check.
    assert_eq!(
        journal.reconcile_pending().unwrap().range,
        SaveRange { start: 11, end: 11 }
    );
    assert_eq!(
        read_history_bytes(f.history_path()).unwrap(),
        second_compacted
    );
    drop(journal);

    let loaded = f.session.read_history().unwrap();
    assert_eq!(loaded.records.len(), 11);
    assert_eq!(loaded.runtime_history(), raw(&original));
    assert_eq!(loaded.compaction_projection(), Some(second.clone()));
    let mut expected = vec![HistoryItem::Summary(second.summary)];
    expected.extend_from_slice(&raw(&original)[5..]);
    assert_eq!(loaded.model_history().unwrap(), expected);
    assert_eq!(
        loaded
            .records
            .iter()
            .filter(|r| matches!(r.entry, HistoryEntry::Compaction { .. }))
            .count(),
        2
    );
    assert!(loaded.records[6].entry.runtime_item().is_none());
    assert!(loaded.records[6].entry.turn_id().is_none());
    assert_eq!(
        read_history_bytes(f.history_path()).unwrap(),
        second_compacted
    );
}

#[test]
fn original_v1_shapes_and_literal_summary_remain_exact_when_v2_is_appended() {
    let f = Fixture::new();
    let mut entries = tool_exchange();
    entries.push(HistoryEntry::Summary {
        summary: SummaryHistory {
            content: Text::new("literal summary, not a replacement"),
        },
    });
    entries.extend(partial_turn());
    entries.extend(complete_turn());
    let mut bytes = Vec::new();
    for (position, entry) in entries.iter().cloned().enumerate() {
        let mut record = HistoryRecord::new(f.session.id().clone(), position as u64, entry);
        record.schema_version = 1;
        serde_json::to_writer(&mut bytes, &record).unwrap();
        bytes.push(b'\n');
    }
    // Existing v1 without a trailing newline is supported without normalization.
    bytes.pop();
    fs::write(f.history_path(), &bytes).unwrap();
    let loaded = f.session.read_history().unwrap();
    assert_eq!(loaded.runtime_history(), raw(&entries));
    assert!(loaded.compaction_projection().is_none());
    assert_eq!(loaded.model_history().unwrap(), raw(&entries));
    let mut journal = f.session.open_journal().unwrap();
    append(
        &mut journal,
        vec![projection(10, 5, "replacement with explicit cumulative coverage").into_entry()],
    );
    drop(journal);
    let written = read_history_bytes(f.history_path()).unwrap();
    assert!(written.starts_with(&bytes));
    let loaded = f.session.read_history().unwrap();
    assert!(
        loaded.records[..10]
            .iter()
            .all(|record| record.schema_version == 1)
    );
    assert_eq!(loaded.records[10].schema_version, HISTORY_SCHEMA_VERSION);
    assert_eq!(loaded.runtime_history(), raw(&entries));
    assert_eq!(loaded.model_history().unwrap().len(), 3);
    assert_eq!(f.session.load().unwrap().schema_version, 1);
}

#[test]
fn v1_compaction_and_unknown_history_versions_fail_without_changes() {
    let f = Fixture::new();
    let prior = complete_turn();
    let prefix = record_bytes(&f.session, prior);
    for version in [0, 1, 3, 999] {
        let mut record = HistoryRecord::new(
            f.session.id().clone(),
            3,
            projection(3, 1, "summary").into_entry(),
        );
        record.schema_version = version;
        let mut bytes = prefix.clone();
        serde_json::to_writer(&mut bytes, &record).unwrap();
        bytes.push(b'\n');
        assert_invalid_file(&f, &bytes);
    }
}

#[test]
fn marker_fields_are_required_typed_and_strict_without_guessing_coverage() {
    let f = Fixture::new();
    let prefix = record_bytes(&f.session, complete_turn());
    let marker = HistoryRecord::new(
        f.session.id().clone(),
        3,
        projection(3, 1, "summary").into_entry(),
    );
    let valid = serde_json::to_value(marker).unwrap();
    let mut invalid = Vec::new();
    for field in ["source_position", "covered_items", "summary"] {
        let mut value = valid.clone();
        value["entry"]["data"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        invalid.push(value);
    }
    for coverage in [
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::Value::Null,
        serde_json::json!("1"),
    ] {
        let mut value = valid.clone();
        value["entry"]["data"]["covered_items"] = coverage;
        invalid.push(value);
    }
    let mut extra = valid;
    extra["entry"]["data"]["invented_anchor"] = serde_json::json!("none");
    invalid.push(extra);
    for value in invalid {
        let mut bytes = prefix.clone();
        serde_json::to_writer(&mut bytes, &value).unwrap();
        bytes.push(b'\n');
        assert_invalid_file(&f, &bytes);
    }
}

#[test]
fn invalid_source_empty_out_of_range_and_nonadvancing_markers_preserve_prior_state() {
    let f = Fixture::new();
    let entries = complete_turn();
    let original = raw(&entries);
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, entries.clone());
    let bytes = read_history_bytes(f.history_path()).unwrap();
    for candidate in [
        projection(2, 1, "stale"),
        projection(4, 1, "future"),
        projection(3, 0, "zero"),
        projection(3, 3, "out of native range"),
        projection(3, 1, " \n\t"),
    ] {
        assert!(candidate.validate(&original, 3, 0).is_err());
        assert!(journal.accept(vec![candidate.into_entry()]).is_err());
        assert_eq!(
            journal.positions(),
            SavePositions {
                accepted: 3,
                persisted: Some(3)
            }
        );
        assert_eq!(read_history_bytes(f.history_path()).unwrap(), bytes);
    }
    append(&mut journal, vec![projection(3, 1, "first").into_entry()]);
    let first = read_history_bytes(f.history_path()).unwrap();
    for covered in [0, 1] {
        let candidate = projection(4, covered, "nonadvancing");
        assert!(candidate.validate(&original, 4, 1).is_err());
        assert!(journal.accept(vec![candidate.into_entry()]).is_err());
        assert_eq!(read_history_bytes(f.history_path()).unwrap(), first);
    }
    append(
        &mut journal,
        vec![projection(4, 2, "advancing").into_entry()],
    );
    drop(journal);
    assert_eq!(
        f.session
            .read_history()
            .unwrap()
            .compaction_projection()
            .unwrap()
            .covered_items,
        2
    );

    for candidate in [
        projection(2, 1, "stale"),
        projection(3, 0, "zero"),
        projection(3, 3, "out of range"),
        projection(3, 1, " "),
    ] {
        let mut bad = entries.clone();
        bad.push(candidate.into_entry());
        assert_invalid_file(&f, &record_bytes(&f.session, bad));
    }
    let mut bad = entries;
    bad.extend([
        projection(3, 1, "first").into_entry(),
        projection(4, 1, "duplicate coverage").into_entry(),
    ]);
    assert_invalid_file(&f, &record_bytes(&f.session, bad));
}

#[test]
fn cuts_cannot_split_tool_call_results_or_cover_an_unclosed_tail() {
    let f = Fixture::new();
    let mut entries = tool_exchange();
    let original = raw(&entries);
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, entries.clone());
    let before = read_history_bytes(f.history_path()).unwrap();
    let split = projection(3, 2, "would orphan result");
    assert!(split.validate(&original, 3, 0).is_err());
    assert!(journal.accept(vec![split.clone().into_entry()]).is_err());
    assert_eq!(read_history_bytes(f.history_path()).unwrap(), before);
    drop(journal);
    entries.push(split.into_entry());
    assert_invalid_file(&f, &record_bytes(&f.session, entries));

    let mut unclosed = tool_exchange();
    unclosed.pop();
    let missing = projection(2, 2, "cannot assume pending call is old");
    assert!(missing.validate(&raw(&unclosed), 2, 0).is_err());
    unclosed.push(missing.into_entry());
    assert_invalid_file(&f, &record_bytes(&f.session, unclosed));
}

#[test]
fn historical_missing_and_mixed_results_remain_raw_without_fabricated_outcomes() {
    let f = Fixture::new();
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let mut assistant = tool_response(turn.clone(), loop_id);
    if let HistoryEntry::Runtime {
        item: HistoryItem::Assistant(item),
        ..
    } = &mut assistant
    {
        item.content.push(AssistantPart::ToolCall(
            ToolCall::new(
                ToolCallId::new("missing-call").unwrap(),
                "read_file".parse().unwrap(),
                serde_json::json!({"path":"unknown.txt"}),
                1,
            )
            .unwrap(),
        ));
    }
    let mut entries = vec![
        input(turn.clone(), loop_id),
        assistant,
        tool_result(turn.clone(), loop_id),
        HistoryEntry::TurnFinished {
            turn_id: turn,
            outcome: TurnOutcome::Cancelled,
            reason: Some("historical cancellation, tool outcome unknown".into()),
        },
    ];
    // A new native group establishes the end of the historical incomplete one.
    // A reused ToolCallId belongs to the actual new Loop, not the missing group.
    entries.extend(tool_exchange());
    let original = raw(&entries);
    let bytes = record_bytes(&f.session, entries.clone());
    fs::write(f.history_path(), &bytes).unwrap();
    assert_eq!(
        f.session.read_history().unwrap().runtime_history(),
        original
    );
    let candidate = projection(7, 3, "historical missing call is unknown");
    candidate.validate(&original, 7, 0).unwrap();
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, vec![candidate.clone().into_entry()]);
    drop(journal);
    let loaded = f.session.read_history().unwrap();
    assert_eq!(loaded.runtime_history(), original);
    assert!(
        read_history_bytes(f.history_path())
            .unwrap()
            .starts_with(&bytes)
    );
    let projected = loaded.model_history().unwrap();
    assert_eq!(&projected[1..], &original[3..]);
    assert_eq!(
        projected
            .iter()
            .filter(|item| matches!(item, HistoryItem::ToolResult(_)))
            .count(),
        1
    );
    assert_eq!(
        loaded
            .runtime_history()
            .iter()
            .filter(|item| matches!(item, HistoryItem::ToolResult(_)))
            .count(),
        2
    );
}

#[test]
fn partial_finish_and_markers_never_count_as_native_coverage() {
    let f = Fixture::new();
    let mut entries = partial_turn();
    let partial = entries[1].clone();
    entries.extend(complete_turn());
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, entries.clone());
    assert!(
        journal
            .accept(vec![
                projection(6, 4, "physical rows are not native items").into_entry()
            ])
            .is_err()
    );
    append(
        &mut journal,
        vec![projection(6, 1, "first summary").into_entry()],
    );
    assert!(
        journal
            .accept(vec![
                projection(7, 4, "prior marker adds no raw item").into_entry()
            ])
            .is_err()
    );
    append(
        &mut journal,
        vec![projection(7, 2, "second summary").into_entry()],
    );
    drop(journal);
    let loaded = f.session.read_history().unwrap();
    assert_eq!(loaded.runtime_history(), raw(&entries));
    assert_eq!(loaded.runtime_history().len(), 3);
    assert_eq!(loaded.records[1].entry, partial);
    assert_eq!(loaded.model_history().unwrap().len(), 2);
}

#[test]
fn a_later_result_cannot_cross_already_committed_coverage() {
    let f = Fixture::new();
    let turn = TurnId::new().unwrap();
    let loop_id = LoopId::new().unwrap();
    let entries = vec![
        tool_response(turn.clone(), loop_id),
        input(TurnId::new().unwrap(), LoopId::new().unwrap()),
    ];
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, entries.clone());
    let candidate = projection(2, 1, "old call with no recorded result");
    candidate.validate(&raw(&entries), 2, 0).unwrap();
    append(&mut journal, vec![candidate.clone().into_entry()]);
    let original = read_history_bytes(f.history_path()).unwrap();
    let result = tool_result(turn, loop_id);
    assert!(journal.accept(vec![result.clone()]).is_err());
    assert_eq!(read_history_bytes(f.history_path()).unwrap(), original);
    drop(journal);
    let mut crossing = entries;
    crossing.push(result.clone());
    assert!(candidate.project(&raw(&crossing)).is_err());
    let mut file_entries = crossing[..2].to_vec();
    file_entries.extend([candidate.into_entry(), result]);
    assert_invalid_file(&f, &record_bytes(&f.session, file_entries));
}

#[test]
fn invalid_batch_does_not_install_compaction_coverage_before_acceptance() {
    let f = Fixture::new();
    let entries = complete_turn();
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, entries);
    let before = read_history_bytes(f.history_path()).unwrap();
    assert!(
        journal
            .accept(vec![
                projection(3, 1, "must remain unaccepted").into_entry(),
                projection(4, 1, "invalid second marker").into_entry()
            ])
            .is_err()
    );
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 3,
            persisted: Some(3)
        }
    );
    assert_eq!(read_history_bytes(f.history_path()).unwrap(), before);
    append(
        &mut journal,
        vec![projection(3, 1, "valid first marker").into_entry()],
    );
}

#[test]
fn memory_compaction_uses_same_validation_without_creating_files() {
    let f = Fixture::new();
    let session = f
        .agent
        .session(SessionId::new().unwrap(), StorageMode::Memory)
        .unwrap();
    let mut journal = session.open_journal().unwrap();
    let entries = complete_turn();
    append(&mut journal, entries.clone());
    let candidate = projection(3, 1, "memory summary");
    candidate.validate(&raw(&entries), 3, 0).unwrap();
    let range = journal
        .accept(vec![candidate.clone().into_entry()])
        .unwrap();
    let confirmation = journal.persist_pending().unwrap();
    assert_eq!(confirmation.range, range);
    assert_eq!(confirmation.persistence, PersistenceState::NotApplicable);
    assert_eq!(
        journal.positions(),
        SavePositions {
            accepted: 4,
            persisted: None
        }
    );
    assert_eq!(candidate.project(&raw(&entries)).unwrap().len(), 2);
    assert!(!session.canonical_path().exists());
}

#[test]
fn pure_helper_and_journal_agree_at_every_actual_exchange_cut() {
    for complete in [false, true] {
        let f = Fixture::new();
        let mut entries = tool_exchange();
        if !complete {
            entries.pop();
        }
        entries.extend(complete_turn());
        let position = entries.len() as u64;
        let native = raw(&entries);
        for cut in 0..=native.len() as u64 + 1 {
            let mut journal = SessionJournal::memory(SessionId::new().unwrap());
            journal.accept(entries.clone()).unwrap();
            let candidate = projection(position, cut, "fixture summary");
            let valid = candidate.validate(&native, position, 0).is_ok();
            assert_eq!(
                journal.accept(vec![candidate.into_entry()]).is_ok(),
                valid,
                "complete={complete}, cut={cut}"
            );
        }
        // The same ordinary history is readable even with missing old results.
        let bytes = record_bytes(&f.session, entries);
        fs::write(f.history_path(), &bytes).unwrap();
        assert_eq!(f.session.read_history().unwrap().runtime_history(), native);
    }
}

#[test]
fn coverage_cannot_stop_between_results_in_one_multi_tool_batch() {
    let f = Fixture::new();
    let mut entries = tool_exchange();
    if let HistoryEntry::Runtime {
        item: HistoryItem::Assistant(item),
        ..
    } = &mut entries[1]
    {
        item.content.push(AssistantPart::ToolCall(
            ToolCall::new(
                ToolCallId::new("second-call").unwrap(),
                "read_file".parse().unwrap(),
                serde_json::json!({"path":"second.txt"}),
                1,
            )
            .unwrap(),
        ));
    }
    let mut result = entries[2].clone();
    if let HistoryEntry::Runtime {
        item: HistoryItem::ToolResult(item),
        ..
    } = &mut result
    {
        item.call_id = ToolCallId::new("second-call").unwrap();
    }
    entries.push(result);
    let native = raw(&entries);
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, entries.clone());
    let before = read_history_bytes(f.history_path()).unwrap();
    for cut in [2, 3] {
        let candidate = projection(4, cut, "must keep both actual results with their calls");
        assert!(candidate.validate(&native, 4, 0).is_err());
        assert!(journal.accept(vec![candidate.into_entry()]).is_err());
        assert_eq!(read_history_bytes(f.history_path()).unwrap(), before);
    }
    let valid = projection(4, 4, "whole exchange");
    valid.validate(&native, 4, 0).unwrap();
    append(&mut journal, vec![valid.into_entry()]);
    drop(journal);
    let loaded = f.session.read_history().unwrap();
    assert_eq!(loaded.runtime_history(), native);
    assert_eq!(loaded.model_history().unwrap().len(), 1);
}

#[test]
fn nonadjacent_recorded_result_still_cannot_cross_covered_prefix() {
    let f = Fixture::new();
    let mut entries = tool_exchange();
    entries.insert(2, input(TurnId::new().unwrap(), LoopId::new().unwrap()));
    let native = raw(&entries);
    // This pre-existing native association is valid history, not task recovery.
    let mut journal = f.session.open_journal().unwrap();
    append(&mut journal, entries.clone());
    let before = read_history_bytes(f.history_path()).unwrap();
    let cut = projection(4, 2, "would leave the actual result without its call");
    assert!(cut.validate(&native, 4, 0).is_err());
    assert!(journal.accept(vec![cut.clone().into_entry()]).is_err());
    assert_eq!(read_history_bytes(f.history_path()).unwrap(), before);
    drop(journal);
    entries.push(cut.into_entry());
    assert_invalid_file(&f, &record_bytes(&f.session, entries));
}
