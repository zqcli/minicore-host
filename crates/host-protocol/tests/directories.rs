use minicore_host_protocol::*;
use serde_json::json;

const ID: &str = "9k3m7x2p5v8c4r6t0w1z";

#[test]
fn directory_contracts_reject_authority_fields_and_invented_receipt_facts() {
    assert!(
        serde_json::from_value::<DirectoryListRequest>(
            json!({"path":"/root","limit":10,"internal":true})
        )
        .is_err()
    );
    let request: DirectoryListRequest =
        serde_json::from_value(json!({"path":"/root","limit":10})).unwrap();
    assert!(!request.hidden);
    assert!(request.cursor.is_none());
    let receipt = |result| json!({"kind":"workspace.mkdir","host_id":ID,"host_incarnation":ID,"operation_id":ID,"result":result});
    for invalid in [
        json!({"status":"created"}),
        json!({"status":"creating","path":"/root/x"}),
        json!({"status":"failed"}),
        json!({"status":"created","path":"/root/x","error":{}}),
    ] {
        assert!(serde_json::from_value::<DirectoryMkdirReceipt>(receipt(invalid)).is_err());
    }
    let value = receipt(json!({"status":"created","path":"/root/x"}));
    let parsed: Receipt = serde_json::from_value(value.clone()).unwrap();
    assert!(matches!(parsed, Receipt::WorkspaceMkdir(_)));
    assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    assert_eq!(
        Method::WorkspaceRoots.descriptor().access,
        AccessRequirement::Read
    );
    assert_eq!(
        Method::WorkspaceList.descriptor().access,
        AccessRequirement::Read
    );
    assert_eq!(
        Method::WorkspaceMkdir.descriptor().access,
        AccessRequirement::Operate
    );
}
