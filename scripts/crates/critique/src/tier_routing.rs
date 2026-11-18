//! What LiteLLM said of the model that served one tier reviewer, kept on the reviewer's run and written to the
//! gateway's ledger line as the delegation record writes it (`tier`, `litellm_model_id`, `served_model`,
//! `attempted_fallbacks`, `fallback_fired`, `routing_capture`).
//!
//! A tier reviewer asks for one step model of the tier's chain through the tier's key. LiteLLM may still answer with
//! another deployment, so the review counts only when the proxy saw an answer from the model that was asked for: a
//! fallback, a different serving model or a response without LiteLLM's headers leaves the family unproven.

use crate::error::ShapeError;
use crate::fields::Fields;
use model_gateway::json::{self, Object, Value};
use model_gateway::routing::{Capture, Routing};

/// The routing of one tier reviewer's launch.
#[derive(Clone, Debug, PartialEq)]
pub struct TierRouting {
    pub tier: String,
    /// The step model the reviewer asked for.
    pub requested_model: String,
    /// How much the gateway saw: `recorded` is the only value that proves anything.
    pub capture: String,
    pub litellm_model_id: Option<String>,
    pub served_model: Option<String>,
    pub attempted_fallbacks: Option<u64>,
}

/// The failure a launch gets when its routing does not prove the requested model answered.
pub const COLLAPSED: &str = "routing-collapsed";
/// The failure a launch gets when its routing could not be seen at all.
pub const UNSEEN: &str = "routing-unseen";

impl TierRouting {
    /// The routing the gateway's proxy recorded for a launch that asked `tier` for `requested_model`.
    pub fn of(tier: &str, requested_model: &str, routing: &Routing) -> TierRouting {
        TierRouting {
            tier: tier.to_string(),
            requested_model: requested_model.to_string(),
            capture: routing.capture.as_str().to_string(),
            litellm_model_id: routing.model_id.clone(),
            served_model: routing.served_model.clone(),
            attempted_fallbacks: routing.attempted_fallbacks,
        }
    }

    /// Whether LiteLLM fell back to another deployment, or `None` when the count is unknown.
    pub fn fallback_fired(&self) -> Option<bool> {
        self.attempted_fallbacks.map(|count| count >= 1)
    }

    /// Why this routing does not prove the requested model served the review: the failure code and its detail.
    pub fn collapse(&self) -> Option<(&'static str, String)> {
        if self.capture != Capture::Recorded.as_str() || self.attempted_fallbacks.is_none() {
            return Some((UNSEEN, format!("the proxy's capture was {} and the fallback count {}, so which model served {} is unknown", self.capture, self.attempted_fallbacks.map_or("unreadable".to_string(), |count| count.to_string()), self.requested_model)));
        }
        let served = self.served_model.as_deref();
        if self.fallback_fired() == Some(true) || served.is_some_and(|served| !same_model(served, &self.requested_model)) {
            let fell = self.attempted_fallbacks.unwrap_or(0);
            return Some((COLLAPSED, format!("{} was asked for and LiteLLM answered with {} after {fell} attempted fallbacks", self.requested_model, served.unwrap_or("another deployment"))));
        }
        None
    }

    /// The routing as a run holds it.
    pub fn to_json(&self) -> Value {
        let text = |value: &Option<String>| value.as_deref().map_or(Value::Null, Value::str);
        json::object(vec![
            ("attempted_fallbacks", self.attempted_fallbacks.map_or(Value::Null, |count| Value::Num(count as f64))),
            ("fallback_fired", self.fallback_fired().map_or(Value::Null, Value::Bool)),
            ("litellm_model_id", text(&self.litellm_model_id)),
            ("requested_model", Value::str(&self.requested_model)),
            ("routing_capture", Value::str(&self.capture)),
            ("served_model", text(&self.served_model)),
            ("tier", Value::str(&self.tier)),
        ])
    }

    /// The routing as a run holds it, read back.
    pub fn read(fields: &Fields<'_>) -> Result<TierRouting, ShapeError> {
        let attempted = fields.nullable_number("attempted_fallbacks")?;
        Ok(TierRouting {
            tier: fields.string("tier")?,
            requested_model: fields.string("requested_model")?,
            capture: fields.string("routing_capture")?,
            litellm_model_id: fields.nullable_string("litellm_model_id")?,
            served_model: fields.nullable_string("served_model")?,
            attempted_fallbacks: attempted.map(|count| count as u64),
        })
    }

    /// The members the gateway's ledger line gets: the delegation record's own names, `harness` included.
    pub fn ledger_members(&self, object: &mut Object) {
        let Value::Obj(routing) = self.to_json() else { unreachable!("to_json is an object") };
        for (key, value) in routing.entries() {
            if key != "requested_model" {
                object.set(key, value.clone());
            }
        }
        object.set("harness", Value::str("pi"));
    }
}

/// Whether the model a response names is the one asked for, the LiteLLM group prefix aside (`free/x` is `x`).
fn same_model(served: &str, requested: &str) -> bool {
    let bare = |name: &str| name.rsplit('/').next().unwrap_or(name).to_string();
    served == requested || bare(served) == bare(requested)
}
