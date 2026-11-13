//! When a ticket is likely too big for one fresh `/implement` context, and the limits routing gives the check.

use crate::error::ShapeError;
use crate::fields::Fields;

/// When a ticket is likely too big for one fresh `/implement` context: it touches more than about `max_modules`
/// modules or has more than about `max_criteria` acceptance criteria. Routing configures both.
#[derive(Clone, Debug, PartialEq)]
pub struct TicketSize {
    pub max_criteria: f64,
    pub max_modules: f64,
}

/// #181's starting values, for a stage whose routing states none.
pub fn default_ticket_size() -> TicketSize {
    TicketSize { max_criteria: 8.0, max_modules: 5.0 }
}

/// Reads a stage's `ticket_size`.
pub fn read_ticket_size(fields: &Fields<'_>) -> Result<TicketSize, ShapeError> {
    Ok(TicketSize { max_criteria: fields.number("max_criteria")?, max_modules: fields.number("max_modules")? })
}
