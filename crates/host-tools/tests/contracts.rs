use minicore_host_protocol::*;
use minicore_host_tools::*;
use serde_json::{Value, json};

#[test]
fn parse_diagnostics_keep_constraints_without_echoing_argument_values_or_unknown_names() {
    let secret = "private-marker`, expected forged constraint\n\u{1b}[31m";
    for input in [
        json!({"action":secret}),
        json!({"action":"wait",(secret):"another private value"}),
        json!({"action":"control","operation":"close_scope",(secret):"another private value"}),
        json!({"action":"wait","timeout_ms":secret}),
        json!({"action":"create","resource":"session","target":{"host_id":secret,"agent_id":id()}}),
    ] {
        let error = AgentAction::parse(input).unwrap_err();
        assert!(!error.message().contains("private"));
        assert!(!error.message().contains("forged"));
        assert!(!error.message().contains('\u{1b}'));
        assert!(
            error.message().contains("expected")
                || error.message().contains("ID must contain exactly 20")
                || error.message().contains("unknown field")
                || error.message().contains("unsupported variant"),
            "{}",
            error.message()
        );
    }
    assert_eq!(
        AgentAction::parse(json!({"action":"control","operation":"close_scope","secret, expected private-marker":true}))
            .unwrap_err()
            .message(),
        "arguments contain an unknown field; this action accepts `action`, `operation`"
    );
    let misplaced = AgentAction::parse(
        json!({"action":"create","resource":"session","model":"private-marker","text":"task"}),
    )
    .unwrap_err();
    assert!(misplaced.message().contains("`settings`"));
    assert!(misplaced.message().contains("go inside settings"));
    assert!(!misplaced.message().contains("private"));
    assert_eq!(
        AgentAction::parse(json!({"action":"create"}))
            .unwrap_err()
            .message(),
        "create without resource requires nonblank text"
    );
    assert_eq!(
        AgentAction::parse(json!({"action":"send","target":{"kind":"agent_default","host_id":id(),"agent_id":id()},"text":"task","delegate":true}))
            .unwrap_err()
            .message(),
        "delegate:true requires an explicit Session target"
    );
}

fn id() -> &'static str {
    "00000000000000000000"
}
fn session() -> Value {
    json!({"agent_id":id(),"session_id":id()})
}
fn agent() -> Value {
    json!({"host_id":id(),"agent_id":id()})
}
fn valid_calls() -> Vec<Value> {
    vec![
        json!({"action":"create","resource":"agent","target":{"host_id":id()},"name":"reviewer","mode":"memory"}),
        json!({"action":"create","resource":"session","target":agent()}),
        json!({"action":"send","target":{"kind":"session","agent_id":id(),"session_id":id()},"text":"work"}),
        json!({"action":"inspect","target":{"kind":"agent","host_id":id(),"agent_id":id()},"include_config":true}),
        json!({"action":"inspect","target":{"kind":"session","agent_id":id(),"session_id":id()},"include_output":true}),
        json!({"action":"inspect","target":{"kind":"turn","agent_id":id(),"session_id":id(),"turn_id":id()}}),
        json!({"action":"inspect","target":{"kind":"toolcall","agent_id":id(),"session_id":id(),"turn_id":id(),"tool_call_id":"upstream-call"}}),
        json!({"action":"inspect","target":{"kind":"message","agent_id":id(),"session_id":id(),"message_id":id()}}),
        json!({"action":"inspect","target":{"kind":"receipt","host_id":id(),"operation_id":id()}}),
        json!({"action":"wait"}),
        json!({"action":"control","operation":"cancel_turn","target":session(),"turn_id":id()}),
        json!({"action":"control","operation":"cancel_session","target":session()}),
        json!({"action":"control","operation":"close_session","target":session()}),
        json!({"action":"control","operation":"close_scope"}),
        json!({"action":"update","resource":"agent","target":agent(),"changes":{"name":"name","model":"new"}}),
        json!({"action":"update","resource":"session","target":session(),"changes":{"title":null,"workspace":null}}),
        json!({"action":"create","text":"review changes"}),
        json!({"action":"create","text":"review changes","name":"review","target":agent(),"mode":"persistent","settings":{"reasoning":"high"}}),
        json!({"action":"inspect"}),
        json!({"action":"inspect","include_config":true,"include_output":true,"history_page":{"limit":2}}),
    ]
}
#[test]
fn all_six_actions_and_their_branches_round_trip() {
    for input in valid_calls() {
        let action =
            AgentAction::parse(input.clone()).unwrap_or_else(|error| panic!("{input}: {error}"));
        AgentAction::parse(serde_json::to_value(action).unwrap()).unwrap();
    }
}
#[test]
fn every_branch_rejects_authority_and_forwarding_fields() {
    for input in valid_calls() {
        for field in [
            "source",
            "caller",
            "internal",
            "operation_id",
            "message_id",
            "config_revision",
            "task_id",
            "method",
            "params",
        ] {
            let mut forged = input.clone();
            forged[field] = json!("forged");
            assert!(
                AgentAction::parse(forged).is_err(),
                "{field} accepted by {input}"
            );
        }
    }
}
#[test]
fn branch_specific_fields_and_nullability_are_strict() {
    let invalid = vec![
        json!({"action":"wait","target":session()}),
        json!({"action":"wait","timeout_ms":null}),
        json!({"action":"control","operation":"close_scope","target":session()}),
        json!({"action":"control","operation":"close_session","target":session(),"keep_inbox":true}),
        json!({"action":"create","resource":"session","target":agent(),"from_agent_id":id()}),
        json!({"action":"create","resource":"agent","target":{"host_id":id()},"name":"reviewer"}),
        json!({"action":"create"}),
        json!({"action":"create","text":" \n\t"}),
        json!({"action":"create","text":null}),
        json!({"action":"create","text":123}),
        json!({"action":"create","resource":null,"text":"review"}),
        json!({"action":"send","target":{"kind":"agent_default","host_id":id(),"agent_id":id()},"text":"work","delegate":true}),
        json!({"action":"update","resource":"agent","target":agent(),"changes":{}}),
        json!({"action":"update","resource":"agent","target":agent(),"changes":{"mode":"memory"}}),
        json!({"action":"update","resource":"agent","target":agent(),"changes":{"name":null}}),
        json!({"action":"update","resource":"session","target":session(),"changes":{"model":null}}),
        json!({"action":"update","resource":"session","target":session(),"changes":{"execution":{"tool_timeout_ms":0}}}),
        json!({"action":"inspect","target":{"kind":"turn","agent_id":id(),"session_id":id(),"turn_id":id()},"include_config":true}),
        json!({"action":"inspect","target":{"kind":"receipt","host_id":id(),"operation_id":id()},"include_output":true}),
        json!({"action":"inspect","target":null}),
        json!({"action":"inspect","history_page":{"limit":2}}),
        json!({"action":"inspect","include_output":true,"output_range":{"channel":"text","start":0,"end":5,"max_bytes":5}}),
        json!({"action":"inspect","response_index":0}),
    ];
    for input in invalid {
        assert!(
            AgentAction::parse(input.clone()).is_err(),
            "accepted {input}"
        );
    }
    assert!(
        AgentAction::parse(json!({
            "action": "create",
            "resource": "session",
            "settings": {"execution": {"approval": "auto"}},
            "text": "auto check"
        }))
        .is_ok()
    );
}
#[test]
fn defaults_are_business_defaults_and_wait_has_no_implicit_timeout() {
    let AgentAction::Create(CreateAction::Session {
        target, mode, text, ..
    }) = AgentAction::parse(json!({"action":"create","text":"review changes"})).unwrap()
    else {
        panic!()
    };
    assert!(target.is_none());
    assert!(mode.is_none());
    assert_eq!(text.as_deref(), Some("review changes"));
    let AgentAction::Inspect(inspect) = AgentAction::parse(json!({"action":"inspect"})).unwrap()
    else {
        panic!()
    };
    assert!(inspect.target.is_none());
    assert!(!inspect.include_config);
    assert!(!inspect.include_output);
    let AgentAction::Send(send) = AgentAction::parse(valid_calls()[2].clone()).unwrap() else {
        panic!()
    };
    assert_eq!(send.mode, MessageMode::Steer);
    assert!(!send.delegate);
    let AgentAction::Wait(wait) = AgentAction::parse(json!({"action":"wait"})).unwrap() else {
        panic!()
    };
    assert_eq!(wait.timeout_ms, None);
}
#[test]
fn source_values_are_copied_and_overrides_replace_without_aliasing() {
    let mut source = AgentConfig::default();
    source.role.instructions = "source role".into();
    source.instructions = "source Agent".into();
    source.session_defaults.tools = vec![ToolBinding {
        name: "read".into(),
    }];
    source.session_defaults.workspace = Some(WorkspaceRef {
        path: "/work".into(),
    });
    let settings: SessionConfigPatch =
        serde_json::from_value(json!({"model":"review-model","tools":[],"workspace":null}))
            .unwrap();
    let copied = agent_config_from_settings(&source, &settings).unwrap();
    assert_eq!(copied.role, source.role);
    assert_eq!(copied.instructions, source.instructions);
    assert_eq!(copied.session_defaults.model.as_str(), "review-model");
    assert!(copied.session_defaults.tools.is_empty());
    assert!(copied.session_defaults.workspace.is_none());
    assert_eq!(source.session_defaults.tools.len(), 1);
    source.role.instructions.clear();
    source.instructions.clear();
    assert_eq!(copied.role.instructions, "source role");
    assert_eq!(copied.instructions, "source Agent");
}

#[test]
fn settings_instructions_match_agent_session_and_schema_contracts() {
    let source = AgentConfig {
        instructions: "original".into(),
        ..AgentConfig::default()
    };
    let settings: SessionConfigPatch =
        serde_json::from_value(json!({"instructions":"replacement"})).unwrap();
    assert_eq!(
        agent_config_from_settings(&source, &settings)
            .unwrap()
            .instructions,
        "replacement"
    );
    let cleared: AgentChanges = serde_json::from_value(json!({"instructions":""})).unwrap();
    let snapshot = AgentConfigSnapshot {
        name: "source".into(),
        config: source,
        config_revision: 7,
    };
    assert!(
        snapshot
            .apply(&cleared)
            .unwrap()
            .config
            .instructions
            .is_empty()
    );
    for resource in ["agent", "session"] {
        let target = if resource == "agent" {
            agent()
        } else {
            session()
        };
        AgentAction::parse(json!({"action":"update","resource":resource,"target":target,"changes":{"instructions":""}})).unwrap();
        assert!(AgentAction::parse(json!({"action":"update","resource":resource,"target":target,"changes":{"instructions":null}})).is_err());
    }
    let spec = agent_tool_spec();
    assert_eq!(
        spec.input_schema()["$defs"]["settings"]["properties"]["instructions"]["type"],
        "string"
    );
    let required = spec.input_schema()["$defs"]["settings"]["properties"]["role"]["required"]
        .as_array()
        .unwrap();
    assert!(!required.contains(&json!("instructions")));
}
#[test]
fn candidate_keeps_snapshot_revision_and_omissions() {
    let snapshot = AgentConfigSnapshot {
        name: "fixture name".into(),
        config: AgentConfig::default(),
        config_revision: 17,
    };
    let changes: AgentChanges =
        serde_json::from_value(json!({"name":"new","reasoning":"high"})).unwrap();
    let candidate = snapshot.apply(&changes).unwrap();
    assert_eq!(candidate.expected_revision, 17);
    assert_eq!(candidate.config.role, snapshot.config.role);
    assert_eq!(
        candidate.config.session_defaults.reasoning,
        ReasoningPreference::High
    );
    assert_eq!(candidate.name.as_str(), "new");
    let session_snapshot = SessionConfigSnapshot {
        title: Some("fixture title".into()),
        mode: StorageMode::Memory,
        config: SessionConfig::default(),
        config_revision: 23,
    };
    let changes: SessionChanges = serde_json::from_value(json!({"title":null})).unwrap();
    let candidate = session_snapshot.apply(&changes).unwrap();
    assert_eq!(candidate.expected_revision, 23);
    assert_eq!(candidate.config, session_snapshot.config);
    assert_eq!(candidate.title, None);
}
#[test]
fn actual_runtime_spec_has_only_six_actions_and_strict_objects() {
    let spec = agent_tool_spec();
    assert_eq!(spec.name().as_str(), "agent");
    let schema = spec.input_schema();
    let actions: std::collections::BTreeSet<_> = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| branch["properties"]["action"]["const"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions.into_iter().collect::<Vec<_>>(),
        vec!["control", "create", "inspect", "send", "update", "wait"]
    );
    for branch in schema["oneOf"].as_array().unwrap() {
        assert_eq!(branch["additionalProperties"], false);
    }
    let session_create = &schema["oneOf"][1];
    assert_eq!(session_create["required"], json!(["action"]));
    assert_eq!(
        session_create["properties"]["resource"]["default"],
        "session"
    );
    assert_eq!(
        session_create["anyOf"],
        json!([
            {"required":["resource"]},
            {"required":["text"],"properties":{"text":{"type":"string","pattern":"\\S"}}}
        ])
    );
    for branch in schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|branch| branch["properties"]["action"]["const"] == "inspect")
    {
        let expected = if branch["properties"]["target"]["properties"]["kind"]["const"] == "session"
        {
            json!(["action"])
        } else {
            json!(["action", "target"])
        };
        assert_eq!(branch["required"], expected);
    }
}

#[test]
fn inspect_ranges_and_pages_reuse_authoritative_read_contracts() {
    let turn = json!({"action":"inspect","target":{"kind":"turn","agent_id":id(),"session_id":id(),"turn_id":id()},"include_output":true,"response_index":0,"output_range":{"channel":"text","start":0,"end":12,"max_bytes":6}});
    AgentAction::parse(turn.clone()).unwrap();
    let mut tool = turn.clone();
    tool["target"]["kind"] = json!("toolcall");
    tool["target"]["tool_call_id"] = json!("upstream");
    tool.as_object_mut().unwrap().remove("response_index");
    AgentAction::parse(tool.clone()).unwrap();
    let session = json!({"action":"inspect","target":{"kind":"session","agent_id":id(),"session_id":id()},"include_output":true,"history_page":{"cursor":null,"limit":2}});
    AgentAction::parse(session.clone()).unwrap();
    for original in [turn.clone(), tool.clone(), session.clone()] {
        let mut input = original.clone();
        input["include_output"] = json!(false);
        assert!(AgentAction::parse(input).is_err());
    }
    for (field, value) in [
        ("start", json!(13)),
        ("max_bytes", json!(0)),
        ("channel", json!("")),
    ] {
        let mut input = turn.clone();
        input["output_range"][field] = value;
        assert!(AgentAction::parse(input).is_err());
    }
    for input in [
        {
            let mut x = turn.clone();
            x.as_object_mut().unwrap().remove("response_index");
            x
        },
        {
            let mut x = turn.clone();
            x["output_range"] = Value::Null;
            x
        },
        {
            let mut x = tool;
            x["response_index"] = json!(0);
            x
        },
        {
            let mut x = session;
            x["history_page"]["limit"] = json!(0);
            x
        },
    ] {
        assert!(AgentAction::parse(input).is_err());
    }
}

#[test]
fn execution_schema_exposes_approval_with_the_same_omission_default_as_dispatch() {
    let spec = agent_tool_spec();
    let execution = &spec.input_schema()["$defs"]["settings"]["properties"]["execution"];
    assert_eq!(
        execution["properties"]["approval"]["enum"],
        json!(["auto", "ask"])
    );
    assert_eq!(execution["properties"]["approval"]["default"], "ask");
    assert!(
        !execution["required"]
            .as_array()
            .unwrap()
            .contains(&json!("approval"))
    );
    for mode in ["auto", "ask"] {
        let mut settings = json!(ExecutionSettings::default());
        settings["approval"] = json!(mode);
        AgentAction::parse(json!({"action":"update","resource":"session","target":{"agent_id":id(),"session_id":id()},"changes":{"execution":settings}})).unwrap();
    }
}
