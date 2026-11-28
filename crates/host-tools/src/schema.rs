use minicore_host_runtime::tools::ToolSpec;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

/// One declaration is reused for discovery and the selected Runtime ToolSet.
pub fn agent_tool_spec() -> ToolSpec {
    static SPEC: LazyLock<ToolSpec> = LazyLock::new(|| {
        ToolSpec::new("agent".parse().expect("static tool name"),
        concat!(
            "Manage Agents and Sessions with create, send, inspect, wait, update and control. ",
            "create with text and no resource starts a child Session asynchronously; text must be nonblank. Defaults are your own Agent and memory mode; target and mode may override them. Use a short descriptive name. ",
            "Explicit resource:session may omit text to create an empty Session. resource:agent requires name and mode, copying your Agent unless from_agent_id selects another. ",
            "Only a root Session can set allow_delegation:true on a new create with text or send with delegate:true. Tell the recipient about this grant in the task text. It shares max_children active direct slots across create and send (default 2); pending starts count until released. Grants are not inherited or retained for later delegations. ",
            "Same-Agent children inherit your model, reasoning, approval policy and workspace. Overrides belong in settings (model, reasoning, instructions, role, tools, workspace, execution); choose model IDs from available_models in session context. ",
            "For ordinary communication, send to the exact agent_id/session_id with kind:session and omit delegate (false). Use kind:agent_default only to address an Agent's default Session. Set delegate:true to enroll an existing exact Session into delegated work. A delegated child without a grant can still communicate and append to active relations in its scope, but cannot start new delegated work. Do not supply grant fields on an append. ",
            "send defaults to mode:steer: next step when busy, new Turn when idle. Explicit mode:follow_up queues for the next Turn, including delegated sends; it cannot answer a wait in the current Turn, so use steer for active coordination. Creation with text and delegated sends return acceptance, not completion. Final results automatically go only to the direct delegator. Ordinary messages need an explicit send reply; do not duplicate the automatic final handoff. ",
            "inspect without target shows your current Session; config and output default to false. Explicit targets remain available. Session include_output:true returns a bounded latest history tail unless history_page is set. ",
            "wait waits for consumable input in your Session, not a specific task, without consuming it. It returns immediately on input_available and defaults to 60000ms; prefer the default over short polling."
        ),
        schema()).expect("static agent tool declaration is valid")
    });
    SPEC.clone()
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false})
}
fn reference(name: &str) -> Value {
    json!({"$ref":format!("#/$defs/{name}")})
}
fn branch(action: &str, extra: Value, required: &[&str]) -> Value {
    let mut properties = extra.as_object().expect("static object").clone();
    properties.insert("action".into(), json!({"const":action}));
    let mut required_fields = vec!["action"];
    required_fields.extend_from_slice(required);
    object(Value::Object(properties), &required_fields)
}
fn delegation_fields(mut branch: Value, startup: Value) -> Value {
    branch["properties"]["allow_delegation"] = json!({
        "type":"boolean", "description":"Only a root can set true on a new delegation. Defaults to false."
    });
    branch["properties"]["max_children"] = json!({
        "type":"integer", "minimum":1, "description":format!(
            "Active direct child limit when allow_delegation is true; defaults to {}.",
            crate::action::DEFAULT_MAX_CHILDREN,
        )
    });
    branch["allOf"] = json!([
        {"if":{"anyOf":[{"required":["allow_delegation"]},{"required":["max_children"]}]},"then":startup},
        {"if":{"required":["max_children"]},"then":{"required":["allow_delegation"],"properties":{"allow_delegation":{"const":true}}}}
    ]);
    branch
}
fn schema() -> Value {
    let id = json!({"type":"string","pattern":"^[0123456789abcdefghjkmnpqrstvwxyz]{20}$"});
    let text = json!({"type":"string"});
    let boolean = json!({"type":"boolean","default":false});
    let host = object(json!({"host_id":id}), &["host_id"]);
    let agent = object(
        json!({"host_id":id,"agent_id":id}),
        &["host_id", "agent_id"],
    );
    let session = object(
        json!({"agent_id":id,"session_id":id}),
        &["agent_id", "session_id"],
    );
    let role = object(
        json!({"name":{"type":"string","minLength":1},"description":text,"instructions":text}),
        &["name", "description"],
    );
    let compaction = object(
        json!({"enabled":{"type":"boolean"},"trigger_percent":{"type":"integer","minimum":1,"maximum":100},"target_percent":{"type":"integer","minimum":1,"maximum":99}}),
        &[],
    );
    let positive = json!({"type":"integer","minimum":1});
    let execution = object(
        json!({"approval":{"enum":["auto","ask"],"default":"ask"},"prompt_timeout_ms":positive,"model_timeout_ms":{"type":"integer","minimum":1,"maximum":86400000},"policy_timeout_ms":positive,"tool_timeout_ms":positive,"model_retry_attempts":{"type":"integer","minimum":1,"maximum":4},"model_retry_base_delay_ms":{"type":"integer","minimum":1,"maximum":30000},"compaction":compaction}),
        &[],
    );
    let settings_properties = json!({
        "role":role,
        "instructions":text,
        "model":{"type":"string","minLength":1,"maxLength":256,"pattern":"^[A-Za-z0-9_.:/-]+$"},
        "reasoning":{"enum":["auto","disabled","low","medium","high","xhigh","max","ultra"]},
        "tools":{"type":"array","items":object(json!({"name":{"type":"string","minLength":1}}), &["name"])},
        "workspace":{"anyOf":[{"type":"null"},object(json!({"path":{"type":"string","minLength":1}}), &["path"])]},
        "execution":execution
    });
    let mut agent_changes = settings_properties.clone();
    agent_changes["name"] = json!({"type":"string","minLength":1});
    let mut agent_changes = object(agent_changes, &[]);
    agent_changes["minProperties"] = json!(1);
    let mut session_changes = settings_properties.clone();
    session_changes["title"] = json!({"type":["string","null"],"minLength":1});
    let mut session_changes = object(session_changes, &[]);
    session_changes["minProperties"] = json!(1);
    let target = |kind: &str, fields: &[&str]| {
        let mut properties = Map::new();
        properties.insert("kind".into(), json!({"const":kind}));
        for field in fields {
            properties.insert(
                (*field).into(),
                if *field == "tool_call_id" {
                    text.clone()
                } else {
                    id.clone()
                },
            );
        }
        let mut required = vec!["kind"];
        required.extend_from_slice(fields);
        object(Value::Object(properties), &required)
    };
    let message_target = json!({"oneOf":[target("agent_default", &["host_id","agent_id"]),target("session", &["agent_id","session_id"])]});
    let create_task =
        json!({"required":["text"],"properties":{"text":{"type":"string","pattern":"\\S"}}});
    let delegate_true = json!({"required":["delegate"],"properties":{"delegate":{"const":true}}});
    let mut alternatives = vec![
        delegation_fields(
            branch(
                "create",
                json!({"resource":{"const":"agent"},"target":reference("host_target"),"name":{"type":"string","minLength":1},"mode":reference("mode"),"from_agent_id":id,"settings":reference("settings"),"text":text}),
                &["resource", "name", "mode"],
            ),
            create_task.clone(),
        ),
        delegation_fields(
            branch(
                "create",
                json!({"resource":{"const":"session","default":"session"},"target":reference("agent_ref"),"name":{"type":"string","minLength":1,"maxLength":60},"mode":reference("mode"),"settings":reference("settings"),"text":text}),
                &[],
            ),
            create_task.clone(),
        ),
        delegation_fields(
            branch(
                "send",
                json!({"target":message_target,"text":text,"mode":{"enum":["steer","follow_up"],"default":"steer"},"delegate":boolean,"in_reply_to":id}),
                &["target", "text"],
            ),
            delegate_true.clone(),
        ),
        branch(
            "wait",
            json!({"timeout_ms":{"type":"integer","minimum":0}}),
            &[],
        ),
        branch(
            "update",
            json!({"resource":{"const":"agent"},"target":reference("agent_ref"),"changes":agent_changes}),
            &["resource", "target", "changes"],
        ),
        branch(
            "update",
            json!({"resource":{"const":"session"},"target":reference("session_ref"),"changes":session_changes}),
            &["resource", "target", "changes"],
        ),
        branch(
            "control",
            json!({"operation":{"const":"cancel_turn"},"target":reference("session_ref"),"turn_id":id,"keep_inbox":boolean}),
            &["operation", "target", "turn_id"],
        ),
        branch(
            "control",
            json!({"operation":{"const":"cancel_session"},"target":reference("session_ref"),"keep_inbox":boolean}),
            &["operation", "target"],
        ),
        branch(
            "control",
            json!({"operation":{"const":"close_session"},"target":reference("session_ref")}),
            &["operation", "target"],
        ),
        branch(
            "control",
            json!({"operation":{"const":"close_scope"}}),
            &["operation"],
        ),
    ];
    // Each inspect branch exposes only the flags which apply to its target.
    for (kind, fields, config, output) in [
        ("agent", vec!["host_id", "agent_id"], true, false),
        ("session", vec!["agent_id", "session_id"], true, true),
        (
            "turn",
            vec!["agent_id", "session_id", "turn_id"],
            false,
            true,
        ),
        (
            "toolcall",
            vec!["agent_id", "session_id", "turn_id", "tool_call_id"],
            false,
            true,
        ),
        (
            "message",
            vec!["agent_id", "session_id", "message_id"],
            false,
            true,
        ),
        ("receipt", vec!["host_id", "operation_id"], false, false),
    ] {
        let mut properties = json!({"target":target(kind, &fields)});
        properties["include_config"] = if config {
            boolean.clone()
        } else {
            json!({"const":false})
        };
        properties["include_output"] = if output {
            boolean.clone()
        } else {
            json!({"const":false})
        };
        let mut rules = Vec::new();
        if kind == "session" {
            properties["history_page"] = reference("history_page");
            rules.push(json!({"if":{"required":["history_page"]},"then":{"required":["include_output"],"properties":{"include_output":{"const":true}}}}));
        }
        if kind == "turn" || kind == "toolcall" {
            properties["output_range"] = reference("output_range");
            rules.push(json!({"if":{"required":["output_range"]},"then":{"required":["include_output"],"properties":{"include_output":{"const":true}}}}));
        }
        if kind == "turn" {
            properties["response_index"] = json!({"type":"integer","minimum":0});
            rules.push(
                json!({"if":{"required":["output_range"]},"then":{"required":["response_index"]}}),
            );
            rules.push(
                json!({"if":{"required":["response_index"]},"then":{"required":["output_range"]}}),
            );
        }
        let required = if kind == "session" {
            &[][..]
        } else {
            &["target"][..]
        };
        let mut inspect = branch("inspect", properties, required);
        if !rules.is_empty() {
            inspect["allOf"] = json!(rules);
        }
        alternatives.push(inspect);
    }
    // Shorthand creation requires a task; explicit Session creation may be empty.
    alternatives[1]["anyOf"] = json!([{"required":["resource"]}, create_task]);
    // The delegate flag cannot resolve an Agent default as a delegation target.
    alternatives[2]["allOf"].as_array_mut().expect("send grant constraints").push(json!({"if":delegate_true,"then":{"properties":{"target":target("session", &["agent_id","session_id"])}}}));
    let output_range = object(
        json!({"channel":{"type":"string","minLength":1},"start":{"type":"integer","minimum":0},"end":{"type":"integer","minimum":0},"max_bytes":positive}),
        &["channel", "start", "end", "max_bytes"],
    );
    let history_page = object(
        json!({"cursor":{"type":["string","null"]},"limit":positive}),
        &["limit"],
    );
    json!({"type":"object","oneOf":alternatives,"$defs":{"host_target":host,"agent_ref":agent,"session_ref":session,"mode":{"enum":["persistent","memory"]},"settings":object(settings_properties, &[]),"output_range":output_range,"history_page":history_page}})
}
