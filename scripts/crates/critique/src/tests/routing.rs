//! routing's own pieces: what the goldens reach only through a command's error text.

use crate::fields::Fields;
use crate::routing::{data_scope_bar, read_critic_client, CriticClient};
use model_gateway::json::parse;

#[test]
fn a_client_names_the_kind_it_was_read_as() {
    let clients = [
        (r#"{"kind":"vibe","binary":"v","agent":"a","model":"m","family":"mistral"}"#, "vibe"),
        (r#"{"kind":"agy","binary":"a","model":"gemini-1","family":"google"}"#, "agy"),
        (r#"{"kind":"codex","binary":"c","model":"m","family":"openai"}"#, "codex"),
        (r#"{"kind":"claude","binary":"c","model":"m","family":"anthropic"}"#, "claude"),
    ];
    for (text, kind) in clients {
        let value = parse(text).expect("json");
        let client = read_critic_client(&Fields::of(&value, "").expect("object")).expect("client");
        assert_eq!(client.kind(), kind);
    }
}

fn client(text: &str) -> Result<CriticClient, String> {
    let value = parse(text).expect("json");
    read_critic_client(&Fields::of(&value, "").expect("object")).map_err(|error| error.0)
}

const PI: &str = r#"{"kind":"pi","provider":"litellm-public","model":"public/gemini-3.8-flash","family":"google","data_scope":"public"}"#;

#[test]
fn a_pi_client_names_its_route_data_scope() {
    assert_eq!(client(PI).map(|client| client.kind()), Ok("pi"));
    assert!(client(&PI.replace("public\"}", "any\"}")).is_ok());
    assert_eq!(client(&PI.replace(",\"data_scope\":\"public\"", "")), Err("data_scope must be one of any, public".to_string()));
    assert_eq!(client(&PI.replace("\"data_scope\":\"public\"", "\"data_scope\":\"private\"")), Err("data_scope must be one of any, public".to_string()));
}

#[test]
fn only_a_public_scope_pi_client_is_barred_and_only_from_a_private_repository() {
    let public = client(PI).expect("client");
    let any = client(&PI.replace("public\"}", "any\"}")).expect("client");
    let vibe = client(r#"{"kind":"vibe","binary":"v","agent":"a","model":"m","family":"mistral"}"#).expect("client");
    assert_eq!([data_scope_bar(&public, false), data_scope_bar(&public, true)], [true, false]);
    assert_eq!([data_scope_bar(&any, false), data_scope_bar(&vibe, false)], [false, false]);
}
