use super::*;

#[tokio::test]
async fn selected_patch_uses_the_real_workspace_and_session_edit_owner() {
    let mut server = Server::start(vec![
        (
            tool_answer(
                "patch-original",
                "apply_patch",
                json!({
                    "path": "file.txt",
                    "patch": "*** Begin Patch\n*** Update File: file.txt\n@@\n-old\n+实际\n second\n*** End Patch\n"
                }),
            ),
            None,
        ),
        (answer("Actual patch completed"), None),
    ])
    .await;
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file.txt"), b"old\r\nsecond\r\n").unwrap();
    let builder = HostResourceBuilder::builtins().unwrap();
    let mut config = config(&server.endpoint);
    config.tools = selection(&["apply_patch"]);
    let host = Host::memory_configured(prepare(&builder, config).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(
        &host,
        &caller,
        selection(&["apply_patch"]),
        workspace(root.path()),
    )
    .await;
    send(&host, &caller, &target, "patch the existing file");
    let turn = settled(&host, &caller, &target, 1).await.remove(0);
    assert_eq!(turn.outcome, Some(TurnOutcome::Completed));
    assert!(turn.cleanup_faults.is_empty());
    let expected = "实际\r\nsecond\r\n".as_bytes();
    assert_eq!(
        std::fs::read(root.path().join("file.txt")).unwrap(),
        expected
    );
    let history = host.history(&caller, &target).unwrap();
    assert_eq!(result_json(&history, "patch-original")["status"], "applied");
    let edits = changes(&host, &caller, &target, ChangeScope::Session);
    assert_eq!(edits.len(), 1);
    let edit = host
        .session_change_record(&caller, &target, &edits[0].change_ref)
        .unwrap();
    assert_eq!(
        edit.before,
        FileSnapshot::Content(b"old\r\nsecond\r\n".to_vec())
    );
    assert_eq!(edit.after, FileSnapshot::Content(expected.to_vec()));
    assert_eq!(edit.tool_call, call(&turn.id, "patch-original"));
    assert_eq!(edit.path, "file.txt");
    for _ in 0..2 {
        let request = server.request().await;
        let tools = request["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"], "apply_patch");
        let required = tools[0]["function"]["parameters"]["required"]
            .as_array()
            .unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("patch")));
    }
}
