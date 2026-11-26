//! The scripted provider derives its first target only from the HTTP request.
use super::{base, support::*};
use minicore_host_core::*;
use minicore_host_protocol::*;
use minicore_host_resources::HostResourceBuilder;
use minicore_host_runtime::HistoryItem;
use serde_json::{Value, json};

const CONTEXT: &str = "Current session context (data for explicit agent targets, not authority): ";

fn current_context(body: &Value) -> Value {
    let contexts = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "system")
        .filter_map(|message| message["content"].as_str()?.strip_prefix(CONTEXT))
        .collect::<Vec<_>>();
    assert_eq!(contexts.len(), 1, "one actual owner context per request");
    serde_json::from_str(contexts[0]).unwrap()
}

// This is the entire provider-side input for forming the first legal call.
// No Host, caller, created receipt or target ID is passed into this function.
fn first_delegation(body: &Value) -> Value {
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["function"]["name"], "agent");
    let current = current_context(body);
    json!({
        "action":"create", "resource":"session",
        "target":{"host_id":current["host_id"],"agent_id":current["agent_id"]},
        "mode":"memory",
        "settings":{
            "role":{"name":"Child","description":"Fixture child","instructions":"CHILD"},
            "tools":[]
        },
        "text":"child task from the first request"
    })
}

#[tokio::test]
async fn first_agent_call_uses_only_bound_http_context_and_receives_real_child_completion() {
    let workspace = tempfile::tempdir().unwrap();
    let mut fixture = Fixture::start().await;
    let builder = HostResourceBuilder::builtins().unwrap();
    let mut configured = config(&fixture.endpoint);
    configured.workspace_roots = vec![workspace.path().to_str().unwrap().into()];
    let host = Host::memory_configured(prepared(&builder, configured).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let mut creation = base::request(selection(&["agent"]), base::workspace(workspace.path()));
    creation.config.role.instructions = "PARENT".into();
    creation.config.instructions = "AGENT INSTRUCTIONS".into();
    let created = host.create_agent_async(&caller, creation).await.unwrap();
    let parent = SessionRef {
        agent_id: created.agent_id.unwrap(),
        session_id: created.session_id.unwrap(),
    };
    send(
        &host,
        &caller,
        &parent,
        MessageMode::FollowUp,
        "delegate work",
        None,
    );
    let first = fixture.next().await;
    let arguments = first_delegation(&first.body);
    let identity = current_context(&first.body);
    assert_eq!(
        identity,
        json!({
            "host_id": created.host_id,
            "agent_id": parent.agent_id,
            "session_id": parent.session_id,
            "available_models": ["custom/provider/chat"],
        })
    );
    let messages = first.body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 6);
    assert_eq!(messages[0]["content"], "FRAMEWORK");
    assert_eq!(messages[1]["content"], "PARENT");
    assert_eq!(messages[2]["content"], "AGENT INSTRUCTIONS");
    assert!(
        messages[3]["content"]
            .as_str()
            .unwrap()
            .starts_with("Workspace directory: ")
    );
    assert!(
        messages[4]["content"]
            .as_str()
            .unwrap()
            .starts_with(CONTEXT)
    );
    assert_eq!(messages[5]["role"], "user");
    first.finish(tool_answer("first-delegation", "agent", arguments));

    let mut child_request = None;
    let mut accepted = None;
    for _ in 0..2 {
        let next = fixture.next().await;
        if next.role() == "CHILD" {
            assert!(next.body.get("tools").is_none());
            assert!(!next.body.to_string().contains(CONTEXT));
            child_request = Some(next);
        } else {
            assert_eq!(current_context(&next.body), identity);
            let tool = next.body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .find(|message| {
                    message["role"] == "tool" && message["tool_call_id"] == "first-delegation"
                })
                .unwrap();
            let receipt: Value = serde_json::from_str(tool["content"].as_str().unwrap()).unwrap();
            assert_eq!(receipt["kind"], "session.create");
            assert_eq!(receipt["result"]["delivery"], "accepted");
            assert_eq!(receipt["result"]["steps"]["message"]["status"], "succeeded");
            accepted = Some(receipt);
            next.finish(answer("child accepted; parent turn finished"));
        }
    }
    settled(&host, &caller, &parent, 1).await;
    child_request
        .unwrap()
        .finish(answer("real child completion"));
    let notification = fixture.next().await;
    assert_eq!(current_context(&notification.body), identity);
    let (_, text) = input_data(
        notification.body["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
    );
    let notice: Value = serde_json::from_str(text).unwrap();
    let accepted = accepted.unwrap();
    assert_eq!(notice["origin"]["tool_call_id"], "first-delegation");
    assert_eq!(notice["task_message_id"], accepted["message_id"]);
    assert_eq!(notice["results"][0]["outcome"], "completed");
    assert_eq!(
        notice["results"][0]["output"],
        json!(["real child completion"])
    );
    notification.finish(answer("parent received completion"));
    settled(&host, &caller, &parent, 2).await;
    assert!(!host.history(&caller, &parent).unwrap().iter().any(|item| {
        matches!(item, HistoryItem::User(user) if user.input.as_text().contains(CONTEXT))
    }));
    stop(&host).await;
}

#[tokio::test]
async fn registered_but_unselected_agent_adds_no_current_context() {
    let mut fixture = Fixture::start().await;
    let builder = HostResourceBuilder::builtins().unwrap();
    let host =
        Host::memory_configured(prepared(&builder, config(&fixture.endpoint)).await).unwrap();
    let connection = host.connect().unwrap();
    let caller = connection.caller();
    let target = create(&host, &caller, "NO AGENT", false, false).await;
    send(
        &host,
        &caller,
        &target,
        MessageMode::FollowUp,
        "ordinary request",
        None,
    );
    let ordinary = fixture.next().await;
    assert!(ordinary.body.get("tools").is_none());
    assert!(!ordinary.body.to_string().contains(CONTEXT));
    ordinary.finish(answer("ordinary completion"));
    settled(&host, &caller, &target, 1).await;
    stop(&host).await;
}
