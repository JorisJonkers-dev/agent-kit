//! routing's own pieces: what the goldens reach only through a command's error text.

use crate::fields::Fields;
use crate::routing::read_critic_client;
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
