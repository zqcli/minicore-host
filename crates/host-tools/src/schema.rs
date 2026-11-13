use minicore_host_runtime::tools::ToolSpec;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;

/// One declaration is reused for discovery and the selected Runtime ToolSet.
pub fn agent_tool_spec() -> ToolSpec {
    static SPEC: LazyLock<ToolSpec> = LazyLock::new(|| {
        ToolSpec::new("agent".parse().expect("static tool name"),
        "Create, send to, inspect, update or control Agents and Sessions in this Host. Creation with text and send with delegate:true start independent work and return acceptance; completion arrives as a new message. wait waits for consumable input in this Session, not a particular child.",
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
        &["name", "description", "instructions"],
    );
    let compaction = object(
        json!({"enabled":{"type":"boolean"},"trigger_percent":{"type":"integer","minimum":1,"maximum":100},"target_percent":{"type":"integer","minimum":1,"maximum":99}}),
        &["enabled", "trigger_percent", "target_percent"],
    );
    let positive = json!({"type":"integer","minimum":1});
    let execution = object(
        json!({"approval":{"enum":["auto","ask","read_only"],"default":"ask"},"prompt_timeout_ms":positive,"model_timeout_ms":{"type":"integer","minimum":1,"maximum":86400000},"policy_timeout_ms":positive,"tool_timeout_ms":positive,"model_retry_attempts":{"type":"integer","minimum":1,"maximum":4},"model_retry_base_delay_ms":{"type":"integer","minimum":1,"maximum":30000},"compaction":compaction}),
        &[
            "prompt_timeout_ms",
            "model_timeout_ms",
            "policy_timeout_ms",
            "tool_timeout_ms",
            "model_retry_attempts",
            "model_retry_base_delay_ms",
            "compaction",
        ],
    );
    let settings_properties = json!({
        "role":role,
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
    let mut alternatives = vec![
        branch(
            "create",
            json!({"resource":{"const":"agent"},"target":reference("host_target"),"name":{"type":"string","minLength":1},"mode":reference("mode"),"from_agent_id":id,"settings":reference("settings"),"text":text}),
            &["resource", "target", "name", "mode"],
        ),
        branch(
            "create",
            json!({"resource":{"const":"session"},"target":reference("agent_ref"),"mode":reference("mode"),"settings":reference("settings"),"text":text}),
            &["resource", "target"],
        ),
        branch(
            "send",
            json!({"target":message_target,"text":text,"mode":{"enum":["follow_up","steer"],"default":"follow_up"},"delegate":boolean,"in_reply_to":id}),
            &["target", "text"],
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
        let mut inspect = branch("inspect", properties, &["target"]);
        if !rules.is_empty() {
            inspect["allOf"] = json!(rules);
        }
        alternatives.push(inspect);
    }
    // The delegate flag cannot resolve an Agent default as a delegation target.
    alternatives[2]["allOf"] = json!([{"if":{"properties":{"delegate":{"const":true}},"required":["delegate"]},"then":{"properties":{"target":target("session", &["agent_id","session_id"])}}}]);
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
