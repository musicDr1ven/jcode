use super::{fake_harness, fake_harness_with_capabilities, reply};
use jcode_harness_api::{ApiEvent, ApiRequest};
use jcode_sdk::{ErrorKind, ProviderProfileSpec};

#[test]
fn provider_admin_requests_carry_the_profile_and_require_the_capability() {
    let client = fake_harness_with_capabilities(
        |frame, writer| match &frame.request {
            ApiRequest::ProviderAdd {
                name,
                base_url,
                model,
                api_key,
                auth,
                set_default,
                ..
            } => {
                assert_eq!(name, "my-gw");
                assert_eq!(base_url, "https://llm.example.com/v1");
                assert_eq!(model, "model-a");
                assert_eq!(api_key.as_deref(), Some("secret"));
                assert_eq!(auth.as_deref(), Some("bearer"));
                assert!(!set_default);
                reply(frame, ApiEvent::Ok, writer);
            }
            ApiRequest::ProviderRemove { name } => {
                assert_eq!(name, "my-gw");
                reply(frame, ApiEvent::Ok, writer);
            }
            other => panic!("unexpected request: {other:?}"),
        },
        &["sessions", "provider_admin"],
    );

    client
        .add_provider_profile(ProviderProfileSpec {
            name: "my-gw".into(),
            base_url: "https://llm.example.com/v1".into(),
            model: "model-a".into(),
            api_key: Some("secret".into()),
            auth: Some("bearer".into()),
            ..Default::default()
        })
        .expect("add profile");
    client.remove_provider_profile("my-gw").expect("remove profile");
}

#[test]
fn provider_admin_is_refused_without_the_capability() {
    let client = fake_harness(|_, _| {});
    let error = client
        .add_provider_profile(ProviderProfileSpec {
            name: "my-gw".into(),
            base_url: "https://llm.example.com/v1".into(),
            model: "model-a".into(),
            ..Default::default()
        })
        .expect_err("runtime without provider_admin must refuse");
    assert_eq!(error.kind, ErrorKind::UnsupportedCapability);
    assert_eq!(error.code(), "unsupported_capability");
    assert!(error.to_string().contains("jcode provider add"), "{error}");
}
